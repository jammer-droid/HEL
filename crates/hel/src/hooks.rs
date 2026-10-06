//! Trusted project command hooks. Policy and its state belong to the external command;
//! hel owns discovery, trust, invocation, decisions, process limits, and model feedback.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::fs::{self, File, OpenOptions};
use std::io::{self, IsTerminal, Read, Write};
use std::os::fd::{AsRawFd, RawFd};
use std::os::unix::fs::OpenOptionsExt;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::time::{Duration, Instant};

use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::runtime::Runtime;

const CONFIG: &str = "hooks.json";
const TRUST: &str = "hooks-trust.json";
const DEFAULT_TIMEOUT: u64 = 600;
const MAX_BYTES: usize = 1_048_576;
const MAX_HANDLERS: usize = 128;
const FEEDBACK_TOKENS: u64 = 2_500;

#[derive(Clone, Copy, PartialEq)]
enum Event {
    Pre,
    Post,
}
impl Event {
    fn name(self) -> &'static str {
        match self {
            Self::Pre => "PreToolUse",
            Self::Post => "PostToolUse",
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Config {
    #[serde(default)]
    description: Option<String>,
    hooks: BTreeMap<String, Vec<Group>>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Group {
    #[serde(default)]
    matcher: Option<String>,
    hooks: Vec<Definition>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Definition {
    #[serde(rename = "type")]
    kind: String,
    command: String,
    timeout: Option<u64>,
}
struct Handler {
    event: Event,
    matcher: Option<Regex>,
    command: String,
    timeout: Duration,
    index: usize,
}
impl Handler {
    fn matches(&self, event: Event, tool: &str) -> bool {
        self.event == event && self.matcher.as_ref().is_none_or(|m| m.is_match(tool))
    }
}

#[derive(Serialize, Deserialize)]
struct Trust {
    schema_version: u32,
    project: PathBuf,
    sha256: String,
}

#[derive(Default)]
pub struct Hooks {
    handlers: Vec<Handler>,
    session_id: String,
    trace: RefCell<Vec<Value>>,
}

pub struct Call<'a> {
    pub id: &'a str,
    pub name: &'a str,
    pub args: &'a Value,
}

enum Decision {
    Continue,
    Feedback(String),
}

impl Hooks {
    /// No prompt is read from piped model instructions. Existing exact trust works in batch mode.
    pub fn load(project: &Path, session_id: &str) -> Self {
        Self::load_with_review(project, session_id, |definition| {
            if !io::stdin().is_terminal() {
                return false;
            }
            eprintln!(
                "hel: project hooks (external commands, outside the tool sandbox):\n{definition}"
            );
            eprint!("Trust this project's hook configuration? [y/N] ");
            if io::stderr().flush().is_err() {
                return false;
            }
            let mut answer = String::new();
            io::stdin().read_line(&mut answer).is_ok_and(|n| n > 0)
                && matches!(answer.trim().to_ascii_lowercase().as_str(), "y" | "yes")
        })
    }

    fn load_with_review(
        project: &Path,
        session_id: &str,
        mut review: impl FnMut(&str) -> bool,
    ) -> Self {
        let mut hooks = Self {
            session_id: session_id.to_owned(),
            ..Self::default()
        };
        match hooks.activate(project, &mut review) {
            Ok(()) => {}
            Err(error) => hooks.warning("configuration", &error),
        }
        hooks
    }

    fn activate(
        &mut self,
        project: &Path,
        review: &mut impl FnMut(&str) -> bool,
    ) -> Result<(), String> {
        let project = project.canonicalize().map_err(|e| e.to_string())?;
        let directory = project.join(".hel");
        let meta = match fs::symlink_metadata(&directory) {
            Ok(meta) => meta,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(()),
            Err(e) => return Err(e.to_string()),
        };
        if !meta.is_dir() {
            return Err(".hel must be a real directory; hooks skipped".into());
        }
        let bytes = match read_plain(&directory.join(CONFIG)) {
            Ok(bytes) => bytes,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(()),
            Err(e) => {
                return Err(format!(
                    "could not load .hel/hooks.json: {e}; hooks skipped"
                ));
            }
        };
        // Parse everything before running or trusting anything. An invalid matcher must not
        // silently become a match-all, and unknown handler kinds must not execute as commands.
        let config: Config = serde_json::from_slice(&bytes)
            .map_err(|e| format!("invalid .hel/hooks.json: {e}; hooks skipped"))?;
        let handlers = compile(config)?;
        let trust = Trust {
            schema_version: 1,
            project,
            sha256: format!("{:x}", Sha256::digest(&bytes)),
        };
        let accepted = read_plain(&directory.join(TRUST))
            .ok()
            .and_then(|b| serde_json::from_slice::<Trust>(&b).ok())
            .is_some_and(|old| {
                old.schema_version == trust.schema_version
                    && old.project == trust.project
                    && old.sha256 == trust.sha256
            });
        if !accepted {
            let definition: Value = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
            let display = serde_json::to_string_pretty(&json!({
                "project": trust.project,
                "configuration": definition,
            }))
            .map_err(|e| e.to_string())?;
            if !review(&display) {
                self.warning("untrusted", "project hooks were not trusted; hooks skipped");
                return Ok(());
            }
            // Trust applies to these reviewed bytes, not a later re-read of the configuration.
            if let Err(e) = save_trust(&directory, &trust) {
                self.warning("trust-save", &format!("could not save hook trust: {e}; approved hooks are active for this invocation only"));
            }
        }
        self.trace.borrow_mut().push(json!({
            "event": "HookConfig", "status": "trusted", "sha256": trust.sha256,
            "project": trust.project, "handlers": handlers.len(),
        }));
        self.handlers = handlers;
        Ok(())
    }

    fn warning(&self, kind: &str, message: &str) {
        // JSON quoting makes script/config-controlled terminal characters visible, not active.
        eprintln!("hel: hook {kind}: {}", json!(message));
        self.trace.borrow_mut().push(json!({
            "event": "HookConfig", "status": kind, "message": message,
        }));
    }

    pub fn before(&self, runtime: &Runtime, call: &Call<'_>) -> Option<String> {
        for handler in self
            .handlers
            .iter()
            .filter(|h| h.matches(Event::Pre, call.name))
        {
            if let Decision::Feedback(reason) = self.invoke(runtime, call, handler, None) {
                return Some(model_feedback(
                    runtime,
                    &format!("PreToolUse hook blocked {}: {reason}", call.name),
                ));
            }
        }
        None
    }

    pub fn after(&self, runtime: &Runtime, call: &Call<'_>, original: &str) -> Option<String> {
        let mut feedback = Vec::new();
        for handler in self
            .handlers
            .iter()
            .filter(|h| h.matches(Event::Post, call.name))
        {
            // Every Post receives the actual tool output, never a previous handler's feedback.
            if let Decision::Feedback(reason) = self.invoke(runtime, call, handler, Some(original))
            {
                feedback.push(format!(
                    "[PostToolUse hook {}]\n{reason}",
                    handler.index + 1
                ));
            }
        }
        if feedback.is_empty() {
            None
        } else {
            Some(model_feedback(runtime, &feedback.join("\n\n")))
        }
    }

    fn invoke(
        &self,
        runtime: &Runtime,
        call: &Call<'_>,
        handler: &Handler,
        original: Option<&str>,
    ) -> Decision {
        let mut input = json!({
            "hook_event_name": handler.event.name(),
            "session_id": self.session_id,
            "cwd": runtime.project,
            "tool_use_id": call.id,
            "tool_name": call.name,
            "tool_input": call.args,
        });
        if let Some(original) = original {
            input["tool_response"] = json!(original);
        }
        let started = Instant::now();
        let mut exit_code = None;
        let outcome =
            run_command(runtime, handler, input.to_string().as_bytes()).and_then(|result| {
                exit_code = result.status.code();
                parse_output(handler.event, &result)
            });
        let (status, message) = match &outcome {
            Ok(Decision::Continue) => ("continued", None),
            Ok(Decision::Feedback(reason)) => (
                if handler.event == Event::Pre {
                    "blocked"
                } else {
                    "feedback"
                },
                Some(reason.as_str()),
            ),
            Err(error) => {
                eprintln!(
                    "hel: {} hook {} error: {}",
                    handler.event.name(),
                    handler.index + 1,
                    json!(error)
                );
                ("error", Some(error.as_str()))
            }
        };
        self.trace.borrow_mut().push(json!({
            "event": handler.event.name(), "tool_use_id": call.id, "tool_name": call.name,
            "handler": handler.index, "command": handler.command, "status": status,
            "exit_code": exit_code, "duration_ms": started.elapsed().as_millis(),
            "message": message.map(|m| m.chars().take(4096).collect::<String>()),
        }));
        outcome.unwrap_or(Decision::Continue)
    }

    pub fn write_trace(&self, record_path: &Path) -> io::Result<()> {
        let raw = record_path.parent().unwrap_or(Path::new(".")).join("raw");
        fs::create_dir_all(&raw)?;
        let mut out = File::create(raw.join("hooks.jsonl"))?;
        for trace in self.trace.borrow().iter() {
            writeln!(out, "{trace}")?;
        }
        Ok(())
    }
}

fn compile(config: Config) -> Result<Vec<Handler>, String> {
    let _description = config.description;
    let mut handlers = Vec::new();
    for (name, groups) in config.hooks {
        let event = match name.as_str() {
            "PreToolUse" => Event::Pre,
            "PostToolUse" => Event::Post,
            _ => return Err(format!("unsupported hook event: {name}; hooks skipped")),
        };
        for group in groups {
            let matcher = match group.matcher.as_deref() {
                None | Some("" | "*") => None,
                Some(pattern) => {
                    Some(Regex::new(pattern).map_err(|e| format!("invalid hook matcher: {e}"))?)
                }
            };
            for definition in group.hooks {
                if definition.kind != "command" || definition.command.trim().is_empty() {
                    return Err(
                        "hook needs type=command and a non-empty command; hooks skipped".into(),
                    );
                }
                let timeout = definition.timeout.unwrap_or(DEFAULT_TIMEOUT);
                if timeout == 0 {
                    return Err(
                        "hook timeout must be a positive number of seconds; hooks skipped".into(),
                    );
                }
                if handlers.len() >= MAX_HANDLERS {
                    return Err(format!(
                        "hook configuration exceeds {MAX_HANDLERS} handlers; hooks skipped"
                    ));
                }
                let index = handlers
                    .iter()
                    .filter(|h: &&Handler| h.event == event)
                    .count();
                handlers.push(Handler {
                    event,
                    matcher: matcher.clone(),
                    command: definition.command,
                    timeout: Duration::from_secs(timeout),
                    index,
                });
            }
        }
    }
    Ok(handlers)
}

fn read_plain(path: &Path) -> io::Result<Vec<u8>> {
    let file = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)?;
    if !file.metadata()?.is_file() {
        return Err(io::Error::other("not a regular file"));
    }
    let mut bytes = Vec::new();
    file.take(MAX_BYTES as u64 + 1).read_to_end(&mut bytes)?;
    if bytes.len() > MAX_BYTES {
        return Err(io::Error::other("file exceeds 1 MiB"));
    }
    Ok(bytes)
}

fn save_trust(directory: &Path, trust: &Trust) -> io::Result<()> {
    if !fs::symlink_metadata(directory)?.is_dir() {
        return Err(io::Error::other(".hel must remain a real directory"));
    }
    let temporary = directory.join(format!(".hooks-trust-{}.tmp", uuid::Uuid::new_v4()));
    let result = (|| {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW)
            .open(&temporary)?;
        serde_json::to_writer_pretty(&mut file, trust)?;
        file.write_all(b"\n")?;
        file.sync_all()?;
        fs::rename(&temporary, directory.join(TRUST))
    })();
    if result.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    result
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Output {
    #[serde(rename = "hookSpecificOutput")]
    specific: Option<Specific>,
    decision: Option<String>,
    reason: Option<String>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Specific {
    #[serde(rename = "hookEventName")]
    event: String,
    #[serde(rename = "permissionDecision")]
    decision: String,
    #[serde(rename = "permissionDecisionReason")]
    reason: Option<String>,
}

fn parse_output(event: Event, result: &CommandResult) -> Result<Decision, String> {
    if result.status.code() == Some(2) {
        return Ok(Decision::Feedback(reason_or_default(
            Some(String::from_utf8_lossy(&result.stderr).into_owned()),
            event,
        )));
    }
    if !result.status.success() {
        return Err(format!(
            "command exited with {:?}: {}",
            result.status.code(),
            String::from_utf8_lossy(&result.stderr)
        ));
    }
    if result.stdout.iter().all(u8::is_ascii_whitespace) {
        return Ok(Decision::Continue);
    }
    let output: Output = serde_json::from_slice(&result.stdout)
        .map_err(|e| format!("invalid hook JSON response: {e}"))?;
    match event {
        Event::Pre => {
            if output.decision.is_some() || output.reason.is_some() {
                return Err("PreToolUse decisions must use hookSpecificOutput".into());
            }
            match output.specific {
                None => Ok(Decision::Continue),
                Some(specific) if specific.event != event.name() => {
                    Err("hookEventName does not match invocation".into())
                }
                Some(specific) => match specific.decision.as_str() {
                    "deny" => Ok(Decision::Feedback(reason_or_default(
                        specific.reason,
                        event,
                    ))),
                    // A non-blocking decision never grants extra permission to the tool.
                    "allow" => Ok(Decision::Continue),
                    _ => Err("unsupported PreToolUse permissionDecision".into()),
                },
            }
        }
        Event::Post => {
            if output.specific.is_some() {
                return Err("PostToolUse feedback must use decision and reason".into());
            }
            match output.decision.as_deref() {
                None if output.reason.is_none() => Ok(Decision::Continue),
                Some("block") => Ok(Decision::Feedback(reason_or_default(output.reason, event))),
                _ => Err("unsupported PostToolUse decision".into()),
            }
        }
    }
}

fn reason_or_default(reason: Option<String>, event: Event) -> String {
    reason.filter(|s| !s.trim().is_empty()).unwrap_or_else(|| {
        format!(
            "{} hook returned a blocking decision without a reason",
            event.name()
        )
    })
}

struct CommandResult {
    status: ExitStatus,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}

/// A separate process group lets a timeout close the shell and its still-running children.
/// Nonblocking pipes prevent deadlock on either a large input or simultaneous stdout/stderr.
struct Running {
    child: Child,
    group: i32,
    complete: bool,
}
impl Drop for Running {
    fn drop(&mut self) {
        if !self.complete {
            // SAFETY: the child was started in its own process group with this positive PID.
            unsafe {
                libc::kill(-self.group, libc::SIGKILL);
            }
            let _ = self.child.wait();
        }
    }
}

fn run_command(
    runtime: &Runtime,
    handler: &Handler,
    input: &[u8],
) -> Result<CommandResult, String> {
    let started = Instant::now();
    let child = Command::new("/bin/sh")
        .args(["-c", &handler.command])
        .current_dir(&runtime.project)
        .env("TMPDIR", &runtime.tmp)
        .env("TMP", &runtime.tmp)
        .env("TEMP", &runtime.tmp)
        .env_remove("DEEPSEEK_API_KEY")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0)
        .spawn()
        .map_err(|e| format!("could not start hook command: {e}"))?;
    let mut running = Running {
        group: child.id() as i32,
        child,
        complete: false,
    };
    let mut stdin = running.child.stdin.take();
    let mut stdout = running.child.stdout.take().ok_or("hook stdout missing")?;
    let mut stderr = running.child.stderr.take().ok_or("hook stderr missing")?;
    for fd in [
        stdin.as_ref().map(AsRawFd::as_raw_fd),
        Some(stdout.as_raw_fd()),
        Some(stderr.as_raw_fd()),
    ]
    .into_iter()
    .flatten()
    {
        nonblocking(fd).map_err(|e| e.to_string())?;
    }
    let mut written = 0;
    let mut out = Vec::new();
    let mut err = Vec::new();
    let (mut stdout_done, mut stderr_done) = (false, false);
    loop {
        if started.elapsed() >= handler.timeout {
            return Err(format!(
                "hook command timed out after {} seconds",
                handler.timeout.as_secs()
            ));
        }
        if let Some(pipe) = stdin.as_mut() {
            match pipe.write(&input[written..]) {
                Ok(n) => written += n,
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => {}
                Err(e) if e.kind() == io::ErrorKind::BrokenPipe => {
                    written = input.len();
                }
                Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                Err(e) => return Err(format!("could not write hook input: {e}")),
            }
            if written == input.len() {
                stdin = None;
            }
        }
        if !stdout_done {
            stdout_done = drain(&mut stdout, &mut out)?;
        }
        if !stderr_done {
            stderr_done = drain(&mut stderr, &mut err)?;
        }
        let status = running.child.try_wait().map_err(|e| e.to_string())?;
        if let Some(status) = status
            && stdout_done
            && stderr_done
        {
            running.complete = true;
            return Ok(CommandResult {
                status,
                stdout: out,
                stderr: err,
            });
        }
        std::thread::sleep(Duration::from_millis(2));
    }
}

fn nonblocking(fd: RawFd) -> io::Result<()> {
    // SAFETY: fd belongs to a live child pipe; fcntl changes descriptor flags only.
    let current = unsafe { libc::fcntl(fd, libc::F_GETFL) };
    if current == -1 || unsafe { libc::fcntl(fd, libc::F_SETFL, current | libc::O_NONBLOCK) } == -1
    {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

fn drain(pipe: &mut impl Read, bytes: &mut Vec<u8>) -> Result<bool, String> {
    let mut buffer = [0_u8; 8192];
    loop {
        match pipe.read(&mut buffer) {
            Ok(0) => return Ok(true),
            Ok(n) => {
                if bytes.len() + n > MAX_BYTES {
                    return Err("hook stdout or stderr exceeded the 1 MiB capture limit".into());
                }
                bytes.extend_from_slice(&buffer[..n]);
            }
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => return Ok(false),
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(e) => return Err(format!("could not read hook output: {e}")),
        }
    }
}

fn model_feedback(runtime: &Runtime, text: &str) -> String {
    if crate::context::estimate(&[json!(text)]) <= FEEDBACK_TOKENS {
        return text.to_owned();
    }
    let marker = match runtime.save_spill(text.as_bytes()) {
        Ok(path) => format!(
            "\n\n[Hook feedback shortened. Full feedback: {}. Use read_file to inspect it.]\n\n",
            path.display()
        ),
        Err(_) => "\n\n[Hook feedback shortened; could not save the full feedback.]\n\n".into(),
    };
    let chars: Vec<char> = text.chars().collect();
    let mut kept = 8000.min(chars.len());
    loop {
        let head = kept * 4 / 5;
        let tail = kept - head;
        let preview: String = chars[..head]
            .iter()
            .chain(marker.chars().collect::<Vec<_>>().iter())
            .chain(chars[chars.len() - tail..].iter())
            .collect();
        if crate::context::estimate(&[json!(preview)]) <= FEEDBACK_TOKENS {
            return preview;
        }
        // Escapes/control characters count as serialized characters, not their UTF-8 size.
        if kept == 0 {
            return "[Hook feedback omitted; feedback path exceeds the preview limit.]".into();
        }
        kept /= 2;
    }
}

#[cfg(test)]
#[path = "hooks_tests.rs"]
mod tests;
