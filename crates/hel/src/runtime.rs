//! H8: one hel invocation owns its project boundary, temporary files and spill storage.
//! A lock protects live runs from cleanup. Commands inherit the lock descriptor, so a
//! surviving child keeps its data alive even if the hel process exits unexpectedly.

use std::ffi::CString;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::os::fd::{AsRawFd, FromRawFd, RawFd};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::{Component, Path, PathBuf};
use std::time::{Duration, SystemTime};

use crate::read_file::Reader;

pub const RETENTION: Duration = Duration::from_secs(24 * 60 * 60);

pub struct Runtime {
    pub project: PathBuf,
    pub store: PathBuf,
    pub root: PathBuf,
    pub tmp: PathBuf,
    pub spill: PathBuf,
    pub reader: Reader,
    pub skills: crate::shared::Shared<crate::skills::Skills>,
    pub hooks: crate::hooks::Hooks,
    pub mcp: crate::mcp::Mcp,
    project_dir: File,
    spill_dir: File,
    lease: Option<File>,
    session_lease: Option<File>,
}

impl Runtime {
    pub fn new(project: &Path) -> io::Result<Self> {
        Self::new_in(project, &std::env::temp_dir().join("hel-runs"))
    }

    pub fn new_in(project: &Path, store: &Path) -> io::Result<Self> {
        let project = project.canonicalize()?;
        let project_dir = open_directory(&project)?;
        private_dir(store)?;
        let store = store.canonicalize()?;
        clean_runs(&store, SystemTime::now());
        let root = store.join(uuid::Uuid::new_v4().to_string());
        fs::DirBuilder::new().mode(0o700).create(&root)?;
        let lease = OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW)
            .open(root.join(".active"))?;
        lease.lock()?;
        let tmp = root.join("tmp");
        let spill = root.join("spill");
        private_dir(&tmp)?;
        private_dir(&spill)?;
        let spill_dir = open_directory(&spill)?;
        Ok(Self {
            project,
            store,
            root,
            tmp,
            spill,
            reader: Reader::default(),
            skills: crate::shared::Shared::default(),
            hooks: crate::hooks::Hooks::default(),
            mcp: crate::mcp::Mcp::default(),
            project_dir,
            spill_dir,
            lease: Some(lease),
            session_lease: None,
        })
    }

    /// tmp remains invocation-owned; the supplied spill and lease belong to the session.
    pub fn for_session(project: &Path, spill: &Path, lease: File) -> io::Result<Self> {
        let mut runtime = Self::new(project)?;
        runtime.spill = spill.canonicalize()?;
        runtime.spill_dir = open_directory(&runtime.spill)?;
        runtime.session_lease = Some(lease);
        Ok(runtime)
    }

    pub fn lease_fds(&self) -> Vec<RawFd> {
        let mut fds = vec![self.lease_fd()];
        if let Some(lease) = &self.session_lease {
            fds.push(lease.as_raw_fd());
        }
        fds
    }

    /// .hel is reserved for harness state, including nested project stores.
    pub fn protected(&self, path: &Path) -> bool {
        path.starts_with(&self.store) || path.components().any(|c| c.as_os_str() == ".hel")
    }

    pub fn lease_fd(&self) -> RawFd {
        self.lease
            .as_ref()
            .expect("live runtime owns its lease")
            .as_raw_fd()
    }

    /// Canonical names select a root; descriptor-relative opening prevents a later symlink
    /// substitution from redirecting the actual open outside that root.
    pub fn read_open(&self, path: &Path) -> io::Result<(PathBuf, File)> {
        let target = self.project.join(path).canonicalize()?;
        let (root, dir) = if target.starts_with(&self.spill) {
            (&self.spill, &self.spill_dir)
        } else if target.starts_with(&self.project) && !self.protected(&target) {
            (&self.project, &self.project_dir)
        } else {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "outside the working directory and current session spill",
            ));
        };
        let file = open_beneath(dir, target.strip_prefix(root).unwrap(), libc::O_RDONLY)?;
        regular_file(&file)?;
        Ok((target, file))
    }

    pub fn edit_open(&self, path: &Path, create: bool) -> io::Result<File> {
        let joined = self.project.join(path);
        let name = joined
            .file_name()
            .ok_or_else(|| io::Error::other("not a file path"))?;
        let parent = joined
            .parent()
            .ok_or_else(|| io::Error::other("not a file path"))?
            .canonicalize()?;
        let unresolved = parent.join(name);
        let target = match unresolved.canonicalize() {
            Ok(path) => path,
            Err(e) if e.kind() == io::ErrorKind::NotFound => unresolved,
            Err(e) => return Err(e),
        };
        if !target.starts_with(&self.project) || self.protected(&target) {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "outside the working directory or protected run storage",
            ));
        }
        let flags = libc::O_RDWR | if create { libc::O_CREAT } else { 0 };
        let file = open_beneath(
            &self.project_dir,
            target.strip_prefix(&self.project).unwrap(),
            flags,
        )?;
        regular_file(&file)?;
        Ok(file)
    }

    pub fn save_spill(&self, content: &[u8]) -> io::Result<PathBuf> {
        let name = format!("{}.txt", uuid::Uuid::new_v4());
        let mut file = open_beneath(
            &self.spill_dir,
            Path::new(&name),
            libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL,
        )?;
        file.write_all(content)?;
        Ok(self.spill.join(name))
    }
}

impl Drop for Runtime {
    fn drop(&mut self) {
        // Closing, rather than explicitly unlocking, preserves locks inherited by children.
        drop(self.lease.take());
        clean_run(&self.root, SystemTime::now());
    }
}

fn regular_file(file: &File) -> io::Result<()> {
    if file.metadata()?.is_file() {
        Ok(())
    } else {
        Err(io::Error::other("not a regular file"))
    }
}

pub(crate) fn open_directory(path: &Path) -> io::Result<File> {
    OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW)
        .open(path)
}

fn open_beneath(root: &File, relative: &Path, flags: i32) -> io::Result<File> {
    let parts: Vec<_> = relative.components().collect();
    if parts.is_empty() || parts.iter().any(|c| !matches!(c, Component::Normal(_))) {
        return Err(io::Error::other("expected a relative file path"));
    }
    let mut dir = root.try_clone()?;
    for (index, part) in parts.iter().enumerate() {
        let name = CString::new(part.as_os_str().as_bytes()).map_err(io::Error::other)?;
        let final_part = index + 1 == parts.len();
        let mode = if final_part {
            flags | libc::O_NONBLOCK
        } else {
            libc::O_RDONLY | libc::O_DIRECTORY
        };
        // SAFETY: dir is live, name is NUL-terminated, and returned ownership is unique.
        let fd = unsafe {
            libc::openat(
                dir.as_raw_fd(),
                name.as_ptr(),
                mode | libc::O_NOFOLLOW | libc::O_CLOEXEC,
                0o600,
            )
        };
        if fd == -1 {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: successful openat returned a newly owned descriptor.
        let file = unsafe { File::from_raw_fd(fd) };
        if final_part {
            return Ok(file);
        }
        dir = file;
    }
    unreachable!()
}

pub(crate) fn private_dir(path: &Path) -> io::Result<()> {
    match fs::symlink_metadata(path) {
        Ok(meta) => {
            // SAFETY: geteuid has no preconditions.
            if !meta.is_dir()
                || meta.file_type().is_symlink()
                || meta.uid() != unsafe { libc::geteuid() }
            {
                return Err(io::Error::other(
                    "run storage must be an owned, plain directory",
                ));
            }
            fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
        }
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            match fs::DirBuilder::new().mode(0o700).create(path) {
                Ok(()) => {}
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => return private_dir(path),
                Err(e) => return Err(e),
            }
        }
        Err(e) => return Err(e),
    }
    Ok(())
}

pub fn clean_runs(store: &Path, now: SystemTime) {
    let Ok(entries) = fs::read_dir(store) else {
        return;
    };
    for entry in entries.flatten() {
        if entry.file_type().is_ok_and(|t| t.is_dir()) {
            clean_run(&entry.path(), now);
        }
    }
}

fn clean_run(root: &Path, now: SystemTime) {
    if !fs::symlink_metadata(root).is_ok_and(|m| m.is_dir() && !m.file_type().is_symlink()) {
        return;
    }
    let Ok(lease) = OpenOptions::new()
        .read(true)
        .write(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(root.join(".active"))
    else {
        return;
    };
    if !lease.metadata().is_ok_and(|m| m.is_file()) || lease.try_lock().is_err() {
        return;
    }
    let tmp = root.join("tmp");
    if fs::symlink_metadata(&tmp).is_ok_and(|m| m.is_dir() && !m.file_type().is_symlink()) {
        let _ = fs::remove_dir_all(tmp);
    }
    let spill = root.join("spill");
    if !fs::symlink_metadata(&spill).is_ok_and(|m| m.is_dir() && !m.file_type().is_symlink()) {
        return;
    }
    let Ok(entries) = fs::read_dir(&spill) else {
        return;
    };
    for entry in entries.flatten() {
        if let Ok(meta) = fs::symlink_metadata(entry.path())
            && meta.is_file()
            && !meta.file_type().is_symlink()
            && meta
                .modified()
                .ok()
                .and_then(|m| now.duration_since(m).ok())
                .is_some_and(|age| age >= RETENTION)
        {
            let _ = fs::remove_file(entry.path());
        }
    }
    if fs::read_dir(&spill).is_ok_and(|mut entries| entries.next().is_none()) {
        let _ = fs::remove_dir_all(root);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("hel-runtime-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&dir).unwrap();
        dir
    }

    #[test]
    fn only_project_and_own_spill_are_readable_and_spill_is_never_editable() {
        let base = base();
        let store = base.join("runs");
        // Deliberately make project an ancestor of the store: broad project access must
        // not accidentally expose another run's files or allow editing our own spill.
        let a = Runtime::new_in(&base, &store).unwrap();
        let b = Runtime::new_in(&base, &store).unwrap();
        let own = a.save_spill(b"own").unwrap();
        let other = b.save_spill(b"other").unwrap();
        assert!(a.read_open(&own).is_ok());
        assert!(a.read_open(&other).is_err());
        assert!(a.edit_open(&own, false).is_err());
        fs::write(a.tmp.join("scratch"), "tmp").unwrap();
        assert!(a.read_open(&a.tmp.join("scratch")).is_err());
        std::os::unix::fs::symlink(&other, base.join("other-link")).unwrap();
        assert!(a.read_open(Path::new("other-link")).is_err());
        assert!(a.edit_open(Path::new("other-link"), false).is_err());
        fs::write(base.join("ordinary"), "normal").unwrap();
        std::os::unix::fs::symlink(base.join("ordinary"), base.join("inside-link")).unwrap();
        assert!(a.read_open(Path::new("inside-link")).is_ok());
        drop(a);
        drop(b);
        fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn cleanup_preserves_live_runs_and_retains_fresh_spills_after_exit() {
        let base = base();
        let runtime = Runtime::new_in(&base, &base.join("runs")).unwrap();
        let root = runtime.root.clone();
        let tmp = runtime.tmp.clone();
        let spill = runtime.save_spill(b"keep").unwrap();
        fs::write(tmp.join("scratch"), "temp").unwrap();
        let future = SystemTime::now() + RETENTION + Duration::from_secs(1);
        clean_runs(&runtime.store, future);
        assert!(
            spill.exists() && tmp.join("scratch").exists(),
            "live data cannot be cleaned even when old"
        );
        drop(runtime);
        // Concurrent tests can briefly inherit the lease between fork and exec, before
        // CLOEXEC closes it. Cleanup is intentionally best-effort while any lease is live.
        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        while tmp.exists() && std::time::Instant::now() < deadline {
            clean_runs(&base.join("runs"), SystemTime::now());
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(!tmp.exists());
        assert!(spill.exists());
        while root.exists() && std::time::Instant::now() < deadline {
            clean_runs(&base.join("runs"), future);
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(!root.exists());
        fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn cleanup_does_not_follow_replaced_tmp_or_spill_links() {
        let base = base();
        let runtime = Runtime::new_in(&base, &base.join("runs")).unwrap();
        let outside = base.join("outside");
        fs::create_dir(&outside).unwrap();
        fs::write(outside.join("keep"), "keep").unwrap();
        fs::remove_dir(&runtime.tmp).unwrap();
        fs::remove_dir(&runtime.spill).unwrap();
        std::os::unix::fs::symlink(&outside, &runtime.tmp).unwrap();
        std::os::unix::fs::symlink(&outside, &runtime.spill).unwrap();
        let store = runtime.store.clone();
        drop(runtime);
        clean_runs(
            &store,
            SystemTime::now() + RETENTION + Duration::from_secs(1),
        );
        assert_eq!(fs::read_to_string(outside.join("keep")).unwrap(), "keep");
        fs::remove_dir_all(base).unwrap();
    }
}
