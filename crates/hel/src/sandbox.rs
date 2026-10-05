//! Native macOS file access enforcement. No fallback to unsandboxed execution.
//! Network policy is outside H8: network operations remain allowed.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::runtime::Runtime;

// The launcher receives paths as parameters, never interpolated SBPL source.
const PROFILE: &str = r#"
(version 1)
(deny default)
(allow process-exec process-fork)
(allow signal (target same-sandbox))
(allow process-info* (target same-sandbox))
(allow sysctl-read
    (sysctl-name "hw.ncpu") (sysctl-name "hw.activecpu")
    (sysctl-name "hw.logicalcpu") (sysctl-name "hw.logicalcpu_max")
    (sysctl-name "hw.physicalcpu") (sysctl-name "hw.physicalcpu_max")
    (sysctl-name "hw.memsize") (sysctl-name "hw.pagesize")
    (sysctl-name "hw.machine") (sysctl-name "hw.cputype")
    (sysctl-name "kern.osrelease") (sysctl-name "kern.ostype")
    (sysctl-name "kern.osversion") (sysctl-name "kern.hostname")
    (sysctl-name "kern.osvariant_status") (sysctl-name "kern.bootargs")
    (sysctl-name "hw.ephemeral_storage") (sysctl-name "hw.pagesize_compat"))
(allow system-mac-syscall (mac-policy-name "vnguard"))
(allow system-mac-syscall (require-all (mac-policy-name "Sandbox") (mac-syscall-number 67)))
(allow network*)
(allow mach-lookup
    (global-name "com.apple.system.opendirectoryd.libinfo")
    (global-name "com.apple.system.logger")
    (global-name "com.apple.logd")
    (global-name "com.apple.secinitd")
    (global-name "com.apple.trustd")
    (global-name "com.apple.trustd.agent"))
(allow file-read* file-map-executable
    (require-all
        (require-any
            (subpath (param "PROJECT"))
            (literal (param "EXECUTABLE"))
            (subpath "/bin") (subpath "/sbin")
            (subpath "/usr/bin") (subpath "/usr/sbin")
            (subpath "/usr/lib") (subpath "/usr/libexec") (subpath "/usr/share")
            (subpath "/System/Library") (subpath "/System/Cryptexes/OS")
            (subpath "/System/Volumes/Preboot/Cryptexes/OS/System/Library")
            (subpath "/System/Volumes/Preboot/Cryptexes/OS/usr/lib") (subpath "/Library/Apple"))
        (require-not (subpath (param "STORE")))))
(allow file-read* file-map-executable
    (subpath (param "TMP")) (subpath (param "SPILL")))
(allow file-write*
    (require-all (subpath (param "PROJECT")) (require-not (subpath (param "STORE")))))
(allow file-write* (subpath (param "TMP")))
(allow file-read-metadata
    (path-ancestors (param "PROJECT"))
    (path-ancestors (param "EXECUTABLE"))
    (path-ancestors (param "TMP"))
    (path-ancestors (param "SPILL"))
    (literal "/") (literal "/dev") (literal "/etc")
    (literal "/tmp") (literal "/var"))
(allow file-read*
    (literal "/")
    (literal "/dev/random") (literal "/dev/urandom")
    (literal "/private/etc/localtime") (subpath "/private/var/db/timezone")
    (literal "/private/etc/passwd")
    (literal "/private/var/select/sh"))
(allow file-read* file-write-data file-ioctl (literal "/dev/dtracehelper"))
(allow file-read* file-write-data
    (literal "/dev/null") (literal "/dev/zero"))
(allow file-read-data file-write-data
    (literal "/dev/fd/0") (literal "/dev/fd/1") (literal "/dev/fd/2"))
"#;

pub fn command(runtime: &Runtime, program: &str) -> Result<Command, String> {
    if !cfg!(target_os = "macos") {
        return Err("native sandbox is only supported on macOS; command was not executed".into());
    }
    let executable = resolve(program)?;
    command_with_launcher(runtime, &executable, Path::new("/usr/bin/sandbox-exec"))
}

fn command_with_launcher(
    runtime: &Runtime,
    executable: &Path,
    launcher: &Path,
) -> Result<Command, String> {
    if !launcher.is_file() {
        return Err("sandbox launcher is unavailable; command was not executed".into());
    }
    let mut command = Command::new(launcher);
    command.args(["-p", PROFILE]);
    for (name, value) in [
        ("PROJECT", runtime.project.as_path()),
        ("STORE", runtime.store.as_path()),
        ("TMP", runtime.tmp.as_path()),
        ("SPILL", runtime.spill.as_path()),
        ("EXECUTABLE", executable),
    ] {
        let value = value.to_str().ok_or("sandbox paths must be UTF-8")?;
        command.arg("-D").arg(format!("{name}={value}"));
    }
    command
        .arg("--")
        .arg(executable)
        .current_dir(&runtime.project)
        .env("TMPDIR", &runtime.tmp)
        .env("TMP", runtime.tmp.as_path())
        .env("TEMP", &runtime.tmp)
        .env_remove("DEEPSEEK_API_KEY")
        .stdin(Stdio::null());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        let fd = runtime.lease_fd();
        // SAFETY: only async-signal-safe fcntl is used between fork and exec. The runtime
        // outlives command execution at each call site; other descriptors keep CLOEXEC.
        unsafe {
            command.pre_exec(move || {
                let flags = libc::fcntl(fd, libc::F_GETFD);
                if flags == -1 || libc::fcntl(fd, libc::F_SETFD, flags & !libc::FD_CLOEXEC) == -1 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
    }
    Ok(command)
}

fn resolve(program: &str) -> Result<PathBuf, String> {
    use std::os::unix::fs::PermissionsExt;
    let candidates: Vec<PathBuf> = if Path::new(program).components().count() > 1 {
        vec![PathBuf::from(program)]
    } else {
        std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default())
            .map(|dir| dir.join(program))
            .collect()
    };
    candidates
        .into_iter()
        .find(|path| {
            std::fs::metadata(path)
                .is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
        })
        .ok_or_else(|| format!("program not found: {program}"))?
        .canonicalize()
        .map_err(|e| e.to_string())
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;
    use std::fs;
    use std::time::{Duration, SystemTime};

    fn setup() -> (PathBuf, Runtime) {
        let base = std::env::temp_dir().join(format!("hel-sandbox-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(base.join("project")).unwrap();
        let runtime = Runtime::new_in(&base.join("project"), &base.join("runs")).unwrap();
        (base, runtime)
    }

    #[test]
    fn native_policy_allows_work_and_tmp_but_denies_outside_and_spill_writes() {
        let (base, runtime) = setup();
        let protected = base.join("protected");
        fs::write(&protected, "protected").unwrap();
        fs::write(runtime.project.join("normal"), "before").unwrap();
        let spill = runtime.save_spill(b"stored").unwrap();
        let output = command(&runtime, "/bin/sh")
            .unwrap()
            .args([
                "-c",
                r#"
set -eu
/bin/sh -c 'cat normal; printf after > normal'
printf scratch > "$TMPDIR/scratch"
cat "$TMPDIR/scratch"
cat "$1"
if cat "$2"; then exit 30; fi
if /bin/sh -c 'printf changed > "$1"' sh "$2"; then exit 31; fi
if /bin/sh -c 'printf changed > "$1"' sh "$1"; then exit 32; fi
if rm "$1"; then exit 33; fi
printf OK
"#,
                "sh",
            ])
            .arg(&spill)
            .arg(&protected)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(output.stdout, b"beforescratchstoredOK");
        assert_eq!(fs::read(&protected).unwrap(), b"protected");
        assert_eq!(fs::read(&spill).unwrap(), b"stored");
        assert_eq!(fs::read(runtime.project.join("normal")).unwrap(), b"after");
        drop(runtime);
        fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn native_policy_denies_cross_run_paths_even_under_a_broad_project_root() {
        let (base, old) = setup();
        drop(old);
        let a = Runtime::new_in(&base, &base.join("runs")).unwrap();
        let b = Runtime::new_in(&base, &base.join("runs")).unwrap();
        let own = a.save_spill(b"own").unwrap();
        let peer = b.save_spill(b"peer").unwrap();
        let peer_tmp = b.tmp.join("scratch");
        fs::write(&peer_tmp, "other tmp").unwrap();
        std::os::unix::fs::symlink(&peer, base.join("peer-link")).unwrap();
        let output = command(&a, "/bin/sh")
            .unwrap()
            .args([
                "-c",
                r#"
set -eu
cat "$1"
if /bin/sh -c 'printf changed > "$1"' sh "$1"; then exit 40; fi
shift
for path in "$@"; do
  if cat "$path"; then exit 41; fi
  if /bin/sh -c 'printf changed > "$1"' sh "$path"; then exit 42; fi
done
"#,
                "sh",
            ])
            .arg(&own)
            .arg(&peer)
            .arg(&peer_tmp)
            .arg(base.join("peer-link"))
            .arg(
                a.tmp
                    .join("../../")
                    .join(b.root.file_name().unwrap())
                    .join("tmp/scratch"),
            )
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(output.stdout, b"own");
        assert_eq!(fs::read(&peer).unwrap(), b"peer");
        assert_eq!(fs::read(&peer_tmp).unwrap(), b"other tmp");
        drop(a);
        drop(b);
        fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn measured_outside_probe_has_clean_stdout_after_system_path_fix() {
        let (base, runtime) = setup();
        fs::create_dir(base.join("protected")).unwrap();
        fs::write(base.join("protected/read.txt"), "TEST_ONLY_READ_MARKER\n").unwrap();
        fs::write(base.join("protected/write.txt"), "unchanged\n").unwrap();
        fs::write(
            runtime.project.join("probe.sh"),
            include_str!("../../../evals/tasks/sandbox-outside-01/fixture/workspace/probe.sh"),
        )
        .unwrap();
        let output = command(&runtime, "/bin/bash")
            .unwrap()
            .args(["--noprofile", "--norc", "-c", "/bin/sh probe.sh"])
            .output()
            .unwrap();
        assert!(output.status.success());
        assert_eq!(output.stdout, b"read=denied\nwrite=denied\n");
        assert!(
            output.stderr.is_empty(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(
            fs::read(runtime.project.join("read-attempt.txt"))
                .unwrap()
                .is_empty()
        );
        assert_eq!(
            fs::read(base.join("protected/write.txt")).unwrap(),
            b"unchanged\n"
        );
        for name in ["read-error.txt", "write-error.txt"] {
            let errors = fs::read_to_string(runtime.project.join(name)).unwrap();
            assert!(errors.contains("Operation not permitted"));
            assert!(!errors.contains("/private/var/select/sh"));
        }
        drop(runtime);
        fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn system_shell_starts_without_selector_warning() {
        let (base, runtime) = setup();
        let output = command(&runtime, "/bin/sh")
            .unwrap()
            .args(["-c", "printf 'ready\\n'"])
            .output()
            .unwrap();
        assert!(output.status.success());
        assert_eq!(output.stdout, b"ready\n");
        assert!(
            output.stderr.is_empty(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        drop(runtime);
        fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn system_runtime_allowlist_does_not_expose_data_volume_aliases() {
        let (base, runtime) = setup();
        let protected = base.join("protected");
        fs::write(&protected, "protected").unwrap();
        let canonical = protected.canonicalize().unwrap();
        let alias = Path::new("/System/Volumes/Data").join(canonical.strip_prefix("/").unwrap());
        if alias.exists() {
            let output = command(&runtime, "/bin/cat")
                .unwrap()
                .arg(&alias)
                .output()
                .unwrap();
            assert!(!output.status.success());
            assert!(output.stdout.is_empty());
        }
        drop(runtime);
        fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn startup_failure_never_executes_the_command() {
        let (base, runtime) = setup();
        assert!(
            command_with_launcher(
                &runtime,
                Path::new("/bin/sh"),
                &base.join("missing-launcher")
            )
            .is_err()
        );
        // A parser failure in the actual OS launcher must also leave the requested command unrun.
        let marker = runtime.project.join("should-not-exist");
        let output = Command::new("/usr/bin/sandbox-exec")
            .args([
                "-p",
                "(version 1)(",
                "--",
                "/bin/sh",
                "-c",
                "touch \"$1\"",
                "sh",
            ])
            .arg(&marker)
            .output()
            .unwrap();
        assert!(!output.status.success());
        assert!(!marker.exists());
        drop(runtime);
        fs::remove_dir_all(base).unwrap();
    }

    // Spawned twice by the following test to exercise two independent hel runtimes.
    #[test]
    fn parallel_owner_helper() {
        let Ok(base) = std::env::var("HEL_TEST_PARALLEL_ROOT") else {
            return;
        };
        let base = PathBuf::from(base);
        let id = std::env::var("HEL_TEST_PARALLEL_ID").unwrap();
        let runtime = Runtime::new_in(&base.join("project"), &base.join("runs")).unwrap();
        let spill = runtime.save_spill(id.as_bytes()).unwrap();
        let tmp = runtime.tmp.join("scratch");
        fs::write(&tmp, &id).unwrap();
        fs::write(
            base.join(format!("ready-{id}.json")),
            serde_json::to_vec(&serde_json::json!({
                "spill": spill, "tmp": tmp, "pid": std::process::id()
            }))
            .unwrap(),
        )
        .unwrap();
        let peer_file = base.join(format!("peer-{id}.json"));
        wait_for(&peer_file);
        let peer: serde_json::Value =
            serde_json::from_slice(&fs::read(peer_file).unwrap()).unwrap();
        let output = command(&runtime, "/bin/sh")
            .unwrap()
            .args([
                "-c",
                r#"
set -eu
cat "$1"; cat "$2"
shift 2
for path in "$@"; do
  if cat "$path" >/dev/null; then exit 51; fi
  if /bin/sh -c 'printf changed > "$1"' sh "$path"; then exit 52; fi
done
printf ':peer-denied'
"#,
                "sh",
            ])
            .arg(&spill)
            .arg(&tmp)
            .arg(peer["spill"].as_str().unwrap())
            .arg(peer["tmp"].as_str().unwrap())
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert_eq!(output.stdout, format!("{id}{id}:peer-denied").as_bytes());
        fs::write(base.join(format!("result-{id}")), &output.stdout).unwrap();
        // Keep this runtime alive until BOTH owners have attempted their peer's paths.
        wait_for(&base.join("release"));
    }

    fn wait_for(path: &Path) {
        let deadline = std::time::Instant::now() + Duration::from_secs(20);
        while !path.is_file() {
            assert!(
                std::time::Instant::now() < deadline,
                "timed out waiting for {}",
                path.display()
            );
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    fn two_parallel_hel_owners_cannot_read_or_write_each_others_storage() {
        let (base, runtime) = setup();
        drop(runtime);
        let mut children = Vec::new();
        for id in ["A", "B"] {
            children.push(
                Command::new(std::env::current_exe().unwrap())
                    .args([
                        "--exact",
                        "sandbox::tests::parallel_owner_helper",
                        "--nocapture",
                    ])
                    .env("HEL_TEST_PARALLEL_ROOT", &base)
                    .env("HEL_TEST_PARALLEL_ID", id)
                    .stdin(Stdio::null())
                    .stdout(Stdio::piped())
                    .stderr(Stdio::piped())
                    .spawn()
                    .unwrap(),
            );
        }
        for id in ["A", "B"] {
            wait_for(&base.join(format!("ready-{id}.json")));
        }
        for (id, peer) in [("A", "B"), ("B", "A")] {
            fs::copy(
                base.join(format!("ready-{peer}.json")),
                base.join(format!("peer-{id}.json")),
            )
            .unwrap();
        }
        for id in ["A", "B"] {
            wait_for(&base.join(format!("result-{id}")));
            assert_eq!(
                fs::read_to_string(base.join(format!("result-{id}"))).unwrap(),
                format!("{id}{id}:peer-denied")
            );
            let ready: serde_json::Value =
                serde_json::from_slice(&fs::read(base.join(format!("ready-{id}.json"))).unwrap())
                    .unwrap();
            assert_eq!(
                fs::read_to_string(ready["spill"].as_str().unwrap()).unwrap(),
                id
            );
            assert_eq!(
                fs::read_to_string(ready["tmp"].as_str().unwrap()).unwrap(),
                id
            );
        }
        fs::write(base.join("release"), "done").unwrap();
        for child in children {
            let result = child.wait_with_output().unwrap();
            assert!(
                result.status.success(),
                "{}",
                String::from_utf8_lossy(&result.stderr)
            );
        }
        fs::remove_dir_all(base).unwrap();
    }

    #[test]
    fn inherited_lease_preserves_a_surviving_childs_tmp() {
        let (base, runtime) = setup();
        let root = runtime.root.clone();
        let tmp = runtime.tmp.clone();
        let mut child = command(&runtime, "/bin/sleep")
            .unwrap()
            .arg("0.3")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        drop(runtime);
        crate::runtime::clean_runs(
            &base.join("runs"),
            SystemTime::now() + crate::runtime::RETENTION + Duration::from_secs(1),
        );
        assert!(tmp.exists(), "live child retains the run lock");
        assert!(child.wait().unwrap().success());
        crate::runtime::clean_runs(&base.join("runs"), SystemTime::now());
        assert!(!root.exists(), "inactive tmp-only run is removed");
        fs::remove_dir_all(base).unwrap();
    }
}
