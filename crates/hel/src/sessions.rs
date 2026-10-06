//! Completed-turn JSON snapshots, session leases, and the small session-management CLI.

use std::error::Error;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::{context::Meter, read_file::Reader, runtime, skills::Skills};

type Result<T> = std::result::Result<T, Box<dyn Error>>;
const SNAPSHOT: &str = "snapshot.json";
const SCHEMA: u32 = 1;

#[derive(Clone, Serialize, Deserialize)]
pub struct RequestConfig {
    pub model: record::ModelRequest,
    pub max_output_tokens: u32,
    pub tools: Value,
    pub compaction: Option<crate::context::Policy>,
}

impl RequestConfig {
    fn input_signature(&self) -> Value {
        let mut params = self.model.params.clone();
        if let Some(params) = params.as_object_mut() {
            params.remove("max_tokens");
            params.remove("max_completion_tokens");
        }
        serde_json::json!({
            "provider": self.model.provider,
            "model": self.model.requested,
            "params": params,
            "tools": self.model.params.get("tools").unwrap_or(&self.tools),
        })
    }
}

#[derive(Serialize, Deserialize)]
struct Snapshot {
    schema_version: u32,
    session_id: String,
    cwd: PathBuf,
    created_at: String,
    updated_at: String,
    request_config: RequestConfig,
    messages: Vec<Value>,
    meter: Meter,
    reader: Reader,
    #[serde(default)]
    skills: Skills,
}

pub struct Restored {
    pub messages: Vec<Value>,
    pub meter: Meter,
    pub reader: Reader,
    pub skills: Skills,
}

pub struct Store {
    pub id: String,
    pub root: PathBuf,
    project: PathBuf,
    created_at: String,
    lease: File,
}

impl Store {
    pub fn open(project: &Path, resume: Option<&str>) -> Result<Self> {
        let project = project.canonicalize()?;
        if let Some(id) = resume {
            validate_id(id)?;
        }
        let (store, _guard) = open_store(&project, true)?;
        let id = resume
            .map(str::to_owned)
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
        let root = store.join(&id);
        if resume.is_some() {
            plain_dir(&root)?;
        } else {
            fs::create_dir(&root)?;
            runtime::private_dir(&root)?;
        }
        let lease = acquire(&root, resume.is_none())?;
        if resume.is_none() {
            runtime::private_dir(&root.join("spill"))?;
        } else {
            plain_dir(&root.join("spill"))?;
        }
        let created_at = if resume.is_some() {
            let snapshot = read_snapshot(&root)?;
            validate_snapshot(&snapshot, &project, &id)?;
            snapshot.created_at
        } else {
            now()
        };
        Ok(Self {
            id,
            root,
            project,
            created_at,
            lease,
        })
    }

    pub fn runtime(&self) -> io::Result<runtime::Runtime> {
        runtime::Runtime::for_session(
            &self.project,
            &self.root.join("spill"),
            self.lease.try_clone()?,
        )
    }

    pub fn restore(&self, system: Option<Value>, current: &RequestConfig) -> Result<Restored> {
        let snapshot = read_snapshot(&self.root)?;
        validate_snapshot(&snapshot, &self.project, &self.id)?;
        let mut skills = snapshot.skills.clone();
        let enabled = current.tools.as_array().is_some_and(|tools| {
            tools
                .iter()
                .any(|t| t["function"]["name"] == crate::tools::READ_FILE)
        });
        skills.configure(system, enabled);
        let system = skills.system_message();
        let mut messages = snapshot.messages.clone();
        if messages.first().is_some_and(|m| m["role"] == "system") {
            messages.remove(0);
        }
        if let Some(system) = system {
            messages.insert(0, system);
        }
        let meter = if messages == snapshot.messages
            && snapshot.request_config.input_signature() == current.input_signature()
        {
            snapshot.meter
        } else {
            Meter::default()
        };
        Ok(Restored {
            messages,
            meter,
            reader: snapshot.reader,
            skills,
        })
    }

    pub fn save(
        &self,
        config: &RequestConfig,
        messages: &[Value],
        meter: &Meter,
        reader: &Reader,
        skills: &Skills,
    ) -> Result<()> {
        let snapshot = Snapshot {
            schema_version: SCHEMA,
            session_id: self.id.clone(),
            cwd: self.project.clone(),
            created_at: self.created_at.clone(),
            updated_at: now(),
            request_config: config.clone(),
            messages: messages.to_vec(),
            meter: meter.clone(),
            reader: reader.clone(),
            skills: skills.clone(),
        };
        validate_snapshot(&snapshot, &self.project, &self.id)?;
        let bytes = serde_json::to_vec_pretty(&snapshot)?;
        let temporary = self
            .root
            .join(format!(".snapshot-{}.tmp", uuid::Uuid::new_v4()));
        let result = (|| -> io::Result<()> {
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .custom_flags(libc::O_NOFOLLOW)
                .open(&temporary)?;
            file.write_all(&bytes)?;
            file.write_all(b"\n")?;
            file.sync_all()?;
            fs::rename(&temporary, self.root.join(SNAPSHOT))?;
            runtime::open_directory(&self.root)?.sync_all()?;
            Ok(())
        })();
        if result.is_err() {
            let _ = fs::remove_file(&temporary);
        }
        result?;
        Ok(())
    }
}

/// The caller keeps its messages even on failure and tries again at the next completed turn.
pub fn save_completed(
    store: &Store,
    config: &RequestConfig,
    messages: &[Value],
    meter: &Meter,
    reader: &Reader,
    skills: &Skills,
    log: &crate::output::RunLog,
) -> bool {
    if log.termination != record::Termination::Completed
        || log.error.is_some()
        || log.final_output.is_none()
    {
        return false;
    }
    match store.save(config, messages, meter, reader, skills) {
        Ok(()) => true,
        Err(error) => {
            eprintln!(
                "hel: warning: session {} was not saved: {error}. Conversation continues in memory; resume uses the last successful save.",
                store.id
            );
            false
        }
    }
}

pub fn list(project: &Path) -> Result<Vec<String>> {
    let project = project.canonicalize()?;
    if !project.join(".hel/sessions").exists() {
        return Ok(Vec::new());
    }
    let (store, _guard) = open_store(&project, false)?;
    let mut rows = Vec::new();
    for entry in fs::read_dir(store)? {
        let entry = entry?;
        if !entry.file_type()?.is_dir() {
            continue;
        }
        let Some(id) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        if validate_id(&id).is_err() {
            continue;
        }
        let state = match acquire(&entry.path(), false) {
            Ok(_lease) => "idle",
            Err(_) => "busy/unavailable",
        };
        let detail = match read_snapshot(&entry.path()) {
            Ok(snapshot) if validate_snapshot(&snapshot, &project, &id).is_ok() => format!(
                "{}\t{} messages",
                snapshot.updated_at,
                snapshot.messages.len()
            ),
            _ => "no readable completed snapshot".to_string(),
        };
        rows.push(format!("{id}\t{state}\t{detail}"));
    }
    rows.sort();
    Ok(rows)
}

pub fn delete(project: &Path, id: &str) -> Result<()> {
    validate_id(id)?;
    let project = project.canonicalize()?;
    let (store, _guard) = open_store(&project, false)?;
    let root = store.join(id);
    plain_dir(&root)?;
    let _lease = acquire(&root, false)?;
    fs::remove_dir_all(&root)?;
    Ok(())
}

fn now() -> String {
    humantime::format_rfc3339_millis(SystemTime::now()).to_string()
}

fn validate_id(id: &str) -> Result<()> {
    let uuid = uuid::Uuid::parse_str(id).map_err(|_| "invalid session ID (expected a UUID)")?;
    if uuid.to_string() != id {
        return Err("session ID must use its canonical UUID spelling".into());
    }
    Ok(())
}

fn plain_dir(path: &Path) -> io::Result<()> {
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.is_dir() || metadata.file_type().is_symlink() {
        return Err(io::Error::other(
            "session storage must be a plain directory",
        ));
    }
    Ok(())
}

fn open_store(project: &Path, create: bool) -> Result<(PathBuf, File)> {
    let home = project.join(".hel");
    let store = home.join("sessions");
    if create {
        runtime::private_dir(&home)?;
        runtime::private_dir(&store)?;
        // This reserves the local state directory without editing the project's .gitignore.
        let ignore = home.join(".gitignore");
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW)
            .open(ignore)
        {
            Ok(mut file) => file.write_all(b"*\n")?,
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(e.into()),
        }
    } else {
        plain_dir(&home)?;
        plain_dir(&store)?;
    }
    // Serialize acquire/delete so a deleted lease cannot be replaced under a waiting opener.
    let guard = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(home.join(".sessions.lock"))?;
    if !guard.metadata()?.is_file() {
        return Err("invalid session store lock".into());
    }
    guard.lock()?;
    Ok((store, guard))
}

fn acquire(root: &Path, create: bool) -> Result<File> {
    let lease = OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(create)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(root.join(".active"))?;
    if !lease.metadata()?.is_file() {
        return Err("invalid session lease".into());
    }
    lease
        .try_lock()
        .map_err(|_| "session is already in use; resume/delete refused")?;
    Ok(lease)
}

fn read_snapshot(root: &Path) -> Result<Snapshot> {
    let mut file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(root.join(SNAPSHOT))?;
    if !file.metadata()?.is_file() {
        return Err("snapshot is not a regular file".into());
    }
    let mut text = String::new();
    file.read_to_string(&mut text)?;
    Ok(serde_json::from_str(&text)?)
}

fn validate_snapshot(snapshot: &Snapshot, project: &Path, id: &str) -> Result<()> {
    if snapshot.schema_version != SCHEMA {
        return Err("unsupported session schema version".into());
    }
    if snapshot.session_id != id || snapshot.cwd != project {
        return Err("session ID or project directory does not match".into());
    }
    snapshot.reader.validate()?;
    snapshot.skills.validate()?;
    let mut pending = std::collections::HashSet::new();
    for (index, message) in snapshot.messages.iter().enumerate() {
        match message["role"].as_str() {
            Some("system") if index == 0 => {}
            Some("user" | "assistant") => {
                if !pending.is_empty() {
                    return Err("snapshot has missing tool results".into());
                }
                if let Some(calls) = message.get("tool_calls").filter(|v| !v.is_null()) {
                    let calls = calls.as_array().ok_or("invalid saved tool calls")?;
                    for call in calls {
                        let id = call["id"]
                            .as_str()
                            .filter(|v| !v.is_empty())
                            .ok_or("invalid saved tool call ID")?;
                        if !pending.insert(id.to_string()) {
                            return Err("duplicate saved tool call ID".into());
                        }
                    }
                }
            }
            Some("tool") => {
                let id = message["tool_call_id"]
                    .as_str()
                    .ok_or("invalid saved tool result")?;
                if !pending.remove(id) {
                    return Err("saved tool result has no matching call".into());
                }
            }
            _ => return Err("invalid saved message role".into()),
        }
    }
    if !pending.is_empty()
        || !snapshot
            .messages
            .last()
            .is_some_and(|m| m["role"] == "assistant" && m["content"].is_string())
    {
        return Err("snapshot must end with a completed assistant answer".into());
    }
    Ok(())
}

#[cfg(test)]
#[path = "sessions_tests.rs"]
mod tests;
