//! H10 MCP: tools from trusted project stdio servers, loaded on demand. hel owns discovery,
//! trust, the client side of the protocol, request timeouts, result conversion and the shutdown
//! of the servers it started. Calls pass through the same hooks and permission gate as built-ins.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use crate::shared::Shared;
use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File};
use std::io::{self, BufRead, BufReader, IsTerminal, Read, Write};
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use serde::Deserialize;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use crate::hooks::{Trust, read_plain, save_trust};
use crate::permissions::{Action, Approvable, Tool};
use crate::runtime::Runtime;

pub const LOAD: &str = "load_mcp_tool";
const CONFIG: &str = "mcp.json";
const TRUST: &str = "mcp-trust.json";
const PROTOCOL: &str = "2025-11-25";
const PREFIX: &str = "mcp__";
const DEFAULT_TIMEOUT: u64 = 300;
/// Function names accepted by OpenAI-compatible APIs.
const MAX_NAME: usize = 64;
const MAX_MESSAGE: usize = 10 * 1_048_576;
const MAX_STDERR: usize = 65_536;

#[derive(Clone, Copy)]
pub(crate) struct Limits {
    pub startup: Duration,
    pub shutdown_wait: Duration,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            startup: Duration::from_secs(30),
            shutdown_wait: Duration::from_secs(2),
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Config {
    servers: BTreeMap<String, ServerConfig>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ServerConfig {
    command: String,
    #[serde(default)]
    args: Vec<String>,
    #[serde(default)]
    env: BTreeMap<String, String>,
    /// Seconds a tool call may wait for its response.
    timeout: Option<u64>,
}

#[derive(Default)]
pub struct Mcp {
    servers: Vec<Server>,
    /// Discovered tools by their model-visible name.
    tools: BTreeMap<String, McpTool>,
    loaded: Shared<BTreeSet<String>>,
    loader: LoadTool,
    limits: Limits,
    trace: Shared<Vec<Value>>,
}

/// What a model-visible tool name refers to.
pub enum Lookup<'a> {
    Tool(&'a dyn Tool),
    NotLoaded,
    Unknown,
}

impl Mcp {
    /// Reads a trusted `.hel/mcp.json`, starts its servers and lists their tools. Without a
    /// terminal, only a configuration trusted earlier for this exact project and content runs.
    pub fn load(runtime: &Runtime) -> Self {
        Self::load_with(runtime, Limits::default(), |definition| {
            if !io::stdin().is_terminal() {
                return false;
            }
            eprintln!(
                "hel: project MCP servers (external commands, outside the tool sandbox):\n{definition}"
            );
            eprint!("Trust this project's MCP configuration? [y/N] ");
            if io::stderr().flush().is_err() {
                return false;
            }
            let mut answer = String::new();
            io::stdin().read_line(&mut answer).is_ok_and(|n| n > 0)
                && matches!(answer.trim().to_ascii_lowercase().as_str(), "y" | "yes")
        })
    }

    pub(crate) fn load_with(
        runtime: &Runtime,
        limits: Limits,
        mut review: impl FnMut(&str) -> bool,
    ) -> Self {
        let mut mcp = Self::default();
        mcp.limits = limits;
        if let Err(error) = mcp.activate(runtime, &mut review) {
            mcp.warning("configuration", &error);
        }
        mcp
    }

    fn activate(
        &mut self,
        runtime: &Runtime,
        review: &mut impl FnMut(&str) -> bool,
    ) -> Result<(), String> {
        let directory = runtime.project.join(".hel");
        match fs::symlink_metadata(&directory) {
            Ok(meta) if meta.is_dir() => {}
            Ok(_) => return Err(".hel must be a real directory; MCP skipped".into()),
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(()),
            Err(e) => return Err(e.to_string()),
        }
        let bytes = match read_plain(&directory.join(CONFIG)) {
            Ok(bytes) => bytes,
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(()),
            Err(e) => return Err(format!("could not load .hel/mcp.json: {e}; MCP skipped")),
        };
        // Parse before trusting or starting anything.
        let config: Config = serde_json::from_slice(&bytes)
            .map_err(|e| format!("invalid .hel/mcp.json: {e}; MCP skipped"))?;
        if let Some(name) = config.servers.keys().find(|name| !valid_server(name)) {
            return Err(format!(
                "invalid MCP server name {name:?}: use letters, digits, '-' and single '_'; MCP skipped"
            ));
        }
        let trust = Trust {
            schema_version: 1,
            project: runtime.project.clone(),
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
                self.warning(
                    "untrusted",
                    "project MCP servers were not trusted; MCP skipped",
                );
                return Ok(());
            }
            if let Err(e) = save_trust(&directory, TRUST, &trust) {
                self.warning(
                    "trust-save",
                    &format!("could not save MCP trust: {e}; approved servers are active for this invocation only"),
                );
            }
        }
        self.event(
            json!({"event": "McpConfig", "status": "trusted", "sha256": trust.sha256,
            "servers": config.servers.len()}),
        );
        let mut discovered: BTreeMap<String, Vec<McpTool>> = BTreeMap::new();
        for (name, server) in config.servers {
            match Server::start(runtime, &name, &server, self.limits, &self.trace) {
                Ok((started, tools)) => {
                    let index = self.servers.len();
                    self.event(json!({"event": "McpServer", "server": name,
                        "status": "connected", "tools": tools.len()}));
                    self.servers.push(started);
                    for tool in tools {
                        self.register(index, &name, tool, &mut discovered);
                    }
                }
                Err(error) => {
                    self.warning("server", &format!("MCP server {name} skipped: {error}"))
                }
            }
        }
        for (full, mut tools) in discovered {
            if tools.len() > 1 {
                self.warning(
                    "tool",
                    &format!("MCP tool name {full} is ambiguous; excluded"),
                );
            } else if let Some(tool) = tools.pop() {
                self.tools.insert(full, tool);
            }
        }
        Ok(())
    }

    fn register(
        &self,
        server: usize,
        server_name: &str,
        tool: Value,
        discovered: &mut BTreeMap<String, Vec<McpTool>>,
    ) {
        let Some(original) = tool["name"].as_str() else {
            self.warning(
                "tool",
                &format!("MCP server {server_name} listed a tool without a name"),
            );
            return;
        };
        let schema = tool["inputSchema"].clone();
        if schema["type"] != "object" {
            self.warning(
                "tool",
                &format!("MCP tool {server_name}/{original} has no object inputSchema; excluded"),
            );
            return;
        }
        let sanitized: String = original
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                    c
                } else {
                    '_'
                }
            })
            .collect();
        let full = format!("{PREFIX}{server_name}__{sanitized}");
        if full.len() > MAX_NAME {
            self.warning(
                "tool",
                &format!("MCP tool name {full} exceeds {MAX_NAME} characters; excluded"),
            );
            return;
        }
        let description = tool["description"]
            .as_str()
            .unwrap_or("")
            .trim()
            .to_string();
        discovered.entry(full.clone()).or_default().push(McpTool {
            definition: json!({
                "type": "function",
                "function": {"name": full, "description": description, "parameters": schema},
            }),
            name: full,
            original: original.to_string(),
            description,
            server,
        });
    }

    /// Whether at least one trusted server is connected; only then is `load_mcp_tool` offered.
    pub fn active(&self) -> bool {
        !self.servers.is_empty()
    }

    /// The list of MCP tools for the system prompt: names and descriptions only.
    pub fn system_section(&self) -> Option<String> {
        if !self.active() {
            return None;
        }
        let mut text = format!(
            "MCP tools from connected servers. Before calling one, load it with {LOAD}; \
             a loaded tool is callable from the next request. Loading does not contact the server."
        );
        if self.tools.is_empty() {
            text.push_str("\n(no tools were listed)");
        }
        for tool in self.tools.values() {
            let summary = tool.description.lines().next().unwrap_or("");
            text.push_str(&format!("\n- {}: {}", tool.name, truncate(summary, 300)));
        }
        Some(text)
    }

    /// The system message with the MCP section appended, creating one if needed.
    pub fn extend_system(&self, system: Option<Value>) -> Option<Value> {
        let Some(section) = self.system_section() else {
            return system;
        };
        match system {
            Some(mut message) => {
                let content = message["content"].as_str().unwrap_or("").to_string();
                message["content"] = json!(format!("{content}\n\n{section}"));
                Some(message)
            }
            None => Some(json!({"role": "system", "content": section})),
        }
    }

    /// `load_mcp_tool` and the definitions of loaded MCP tools, in name order.
    pub fn definitions(&self) -> Vec<Value> {
        if !self.active() {
            return Vec::new();
        }
        let mut definitions = vec![load_definition()];
        let loaded = self.loaded.borrow();
        definitions.extend(
            self.tools
                .values()
                .filter(|tool| loaded.contains(&tool.name))
                .map(|tool| tool.definition.clone()),
        );
        definitions
    }

    pub fn lookup(&self, name: &str) -> Lookup<'_> {
        if !self.active() {
            return Lookup::Unknown;
        }
        if name == LOAD {
            return Lookup::Tool(&self.loader);
        }
        match self.tools.get(name) {
            Some(tool) if self.loaded.borrow().contains(name) => Lookup::Tool(tool),
            Some(_) => Lookup::NotLoaded,
            None => Lookup::Unknown,
        }
    }

    fn load_tools(&self, args: &Value) -> Result<String, String> {
        let names: Vec<&str> = args["names"]
            .as_array()
            .map(|names| names.iter().filter_map(Value::as_str).collect())
            .unwrap_or_default();
        if names.is_empty() {
            return Err(format!("{LOAD} needs a non-empty \"names\" list"));
        }
        let mut lines = Vec::new();
        let mut usable = false;
        for name in names {
            if !self.tools.contains_key(name) {
                lines.push(format!(
                    "{name}: unknown MCP tool; use a name from the MCP tool list"
                ));
            } else if self.loaded.borrow_mut().insert(name.to_string()) {
                usable = true;
                lines.push(format!("{name}: loaded; callable from the next request"));
            } else {
                usable = true;
                lines.push(format!("{name}: already loaded"));
            }
        }
        let text = lines.join("\n");
        if usable { Ok(text) } else { Err(text) }
    }

    fn call(&self, tool: &McpTool, args: &Value) -> Result<String, String> {
        let server = &self.servers[tool.server];
        let result = server.request(
            "tools/call",
            json!({"name": tool.original, "arguments": args}),
            server.timeout,
            &self.trace,
        )?;
        convert(&result)
    }

    pub fn write_trace(&self, record_path: &Path) -> io::Result<()> {
        if self.trace.borrow().is_empty() {
            return Ok(());
        }
        let raw = record_path.parent().unwrap_or(Path::new(".")).join("raw");
        fs::create_dir_all(&raw)?;
        let mut out = File::create(raw.join("mcp.jsonl"))?;
        for event in self.trace.borrow().iter() {
            writeln!(out, "{event}")?;
        }
        for server in &self.servers {
            let stderr = server.stderr.lock().map(|b| b.clone()).unwrap_or_default();
            if !stderr.is_empty() {
                let event = json!({"event": "McpStderr", "server": server.name,
                    "text": String::from_utf8_lossy(&stderr)});
                writeln!(out, "{event}")?;
            }
        }
        Ok(())
    }

    fn event(&self, event: Value) {
        self.trace.borrow_mut().push(event);
    }

    fn warning(&self, kind: &str, message: &str) {
        // JSON quoting keeps configuration- and server-controlled characters inert in a terminal.
        eprintln!("hel: mcp {kind}: {}", json!(message));
        self.event(json!({"event": "McpConfig", "status": kind, "message": message}));
    }
}

impl Drop for Mcp {
    fn drop(&mut self) {
        for server in &self.servers {
            server.shutdown(self.limits.shutdown_wait);
        }
    }
}

fn valid_server(name: &str) -> bool {
    !name.is_empty()
        && !name.contains("__")
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

fn truncate(text: &str, max: usize) -> String {
    match text.char_indices().nth(max) {
        Some((at, _)) => format!("{}…", &text[..at]),
        None => text.to_string(),
    }
}

fn load_definition() -> Value {
    json!({
        "type": "function",
        "function": {
            "name": LOAD,
            "description": "Load MCP tools listed in the system prompt so they can be called. \
                A loaded tool becomes callable from the next request. Loading does not call the server.",
            "parameters": {
                "type": "object",
                "properties": {
                    "names": {
                        "type": "array",
                        "items": {"type": "string"},
                        "description": "Full MCP tool names, such as mcp__server__tool.",
                    },
                },
                "required": ["names"],
            },
        },
    })
}

/// Text blocks are joined. Other content is only marked; structured content is used when there
/// is no text at all. `isError` results are returned as errors for the model to read.
fn convert(result: &Value) -> Result<String, String> {
    let mut parts = Vec::new();
    for block in result["content"].as_array().into_iter().flatten() {
        match block["type"].as_str() {
            Some("text") => parts.push(block["text"].as_str().unwrap_or("").to_string()),
            Some(kind) => parts.push(format!("[{kind} content omitted]")),
            None => parts.push("[unknown content omitted]".to_string()),
        }
    }
    let has_text = result["content"]
        .as_array()
        .is_some_and(|blocks| blocks.iter().any(|b| b["type"] == "text"));
    if !has_text && let Some(structured) = result.get("structuredContent").filter(|v| !v.is_null())
    {
        parts = vec![structured.to_string()];
    }
    let text = if parts.is_empty() {
        "(empty result)".to_string()
    } else {
        parts.join("\n")
    };
    if result["isError"] == true {
        Err(text)
    } else {
        Ok(text)
    }
}

struct McpTool {
    name: String,
    original: String,
    description: String,
    definition: Value,
    server: usize,
}

impl Approvable for McpTool {
    /// Server annotations are not trusted for policy; every MCP call counts as execution.
    fn action(&self, _args: &Value) -> Action {
        Action::Execute
    }
}

impl Tool for McpTool {
    fn name(&self) -> &str {
        &self.name
    }
    fn run(&self, runtime: &Runtime, args: &Value) -> Result<String, String> {
        runtime.mcp.call(self, args)
    }
}

#[derive(Default)]
struct LoadTool;

impl Approvable for LoadTool {
    /// Loading changes only the next request's tool list; no server is contacted.
    fn action(&self, _args: &Value) -> Action {
        Action::Read
    }
}

impl Tool for LoadTool {
    fn name(&self) -> &str {
        LOAD
    }
    fn run(&self, runtime: &Runtime, args: &Value) -> Result<String, String> {
        runtime.mcp.load_tools(args)
    }
}

enum Incoming {
    Message(Value),
    Invalid(String),
    Closed,
}

struct Server {
    name: String,
    child: Shared<Child>,
    stdin: Shared<Option<ChildStdin>>,
    incoming: Shared<Receiver<Incoming>>,
    next_id: AtomicU64,
    alive: AtomicBool,
    timeout: Duration,
    stderr: Arc<Mutex<Vec<u8>>>,
}

impl Server {
    fn start(
        runtime: &Runtime,
        name: &str,
        config: &ServerConfig,
        limits: Limits,
        trace: &Shared<Vec<Value>>,
    ) -> Result<(Self, Vec<Value>), String> {
        let timeout = match config.timeout {
            Some(0) => return Err("timeout must be a positive number of seconds".into()),
            Some(seconds) => Duration::from_secs(seconds),
            None => Duration::from_secs(DEFAULT_TIMEOUT),
        };
        let mut child = Command::new(&config.command)
            .args(&config.args)
            .current_dir(&runtime.project)
            .env("TMPDIR", &runtime.tmp)
            .env("TMP", &runtime.tmp)
            .env("TEMP", &runtime.tmp)
            .env_remove("DEEPSEEK_API_KEY")
            .envs(&config.env)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .process_group(0)
            .spawn()
            .map_err(|e| format!("could not start {:?}: {e}", config.command))?;
        let stdout = child.stdout.take().ok_or("server stdout missing")?;
        let mut stderr_pipe = child.stderr.take().ok_or("server stderr missing")?;
        let stdin = child.stdin.take();
        let (sender, incoming) = mpsc::channel();
        thread::spawn(move || read_messages(stdout, sender));
        let stderr = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::clone(&stderr);
        thread::spawn(move || {
            let mut buffer = [0_u8; 8192];
            while let Ok(n) = stderr_pipe.read(&mut buffer) {
                if n == 0 {
                    break;
                }
                if let Ok(mut kept) = sink.lock() {
                    let room = MAX_STDERR.saturating_sub(kept.len());
                    kept.extend_from_slice(&buffer[..n.min(room)]);
                }
            }
        });
        let server = Self {
            name: name.to_string(),
            child: Shared::new(child),
            stdin: Shared::new(stdin),
            incoming: Shared::new(incoming),
            next_id: AtomicU64::new(1),
            alive: AtomicBool::new(true),
            timeout,
            stderr,
        };
        let deadline = Instant::now() + limits.startup;
        let started = (|| {
            let remaining = || deadline.saturating_duration_since(Instant::now());
            server.request(
                "initialize",
                json!({
                    "protocolVersion": PROTOCOL,
                    "capabilities": {},
                    "clientInfo": {"name": "hel", "version": env!("CARGO_PKG_VERSION")},
                }),
                remaining(),
                trace,
            )?;
            server.notify("notifications/initialized", json!({}))?;
            let mut tools = Vec::new();
            let mut cursor: Option<Value> = None;
            loop {
                let params = match &cursor {
                    Some(cursor) => json!({"cursor": cursor}),
                    None => json!({}),
                };
                let page = server.request("tools/list", params, remaining(), trace)?;
                tools.extend(page["tools"].as_array().cloned().unwrap_or_default());
                match page.get("nextCursor").filter(|c| !c.is_null()) {
                    Some(next) => cursor = Some(next.clone()),
                    None => break,
                }
            }
            Ok::<_, String>(tools)
        })();
        match started {
            Ok(tools) => Ok((server, tools)),
            Err(error) => {
                server.shutdown(limits.shutdown_wait);
                Err(error)
            }
        }
    }

    fn send(&self, message: &Value) -> Result<(), String> {
        let mut stdin = self.stdin.borrow_mut();
        let Some(pipe) = stdin.as_mut() else {
            return Err(format!("MCP server {} is closed", self.name));
        };
        let mut line = message.to_string();
        line.push('\n');
        pipe.write_all(line.as_bytes())
            .and_then(|()| pipe.flush())
            .map_err(|e| {
                self.alive.store(false, Ordering::SeqCst);
                format!("MCP server {} disconnected: {e}", self.name)
            })
    }

    fn notify(&self, method: &str, params: Value) -> Result<(), String> {
        self.send(&json!({"jsonrpc": "2.0", "method": method, "params": params}))
    }

    fn request(
        &self,
        method: &str,
        params: Value,
        timeout: Duration,
        trace: &Shared<Vec<Value>>,
    ) -> Result<Value, String> {
        if !self.alive.load(Ordering::SeqCst) {
            return Err(format!("MCP server {} disconnected", self.name));
        }
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        self.send(&json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}))?;
        let deadline = Instant::now() + timeout;
        loop {
            let remaining = deadline.saturating_duration_since(Instant::now());
            match self.incoming.borrow().recv_timeout(remaining) {
                Ok(Incoming::Message(message)) => {
                    if message.get("method").is_some() {
                        // A request from the server: hel offers no client capabilities.
                        if let Some(request) = message.get("id").filter(|v| !v.is_null()) {
                            let _ = self.send(&json!({"jsonrpc": "2.0", "id": request,
                                "error": {"code": -32601, "message": "method not supported by hel"}}));
                        }
                        continue;
                    }
                    if message["id"] != json!(id) {
                        // Late responses to cancelled requests are discarded.
                        continue;
                    }
                    if let Some(error) = message.get("error") {
                        return Err(format!(
                            "MCP error {}: {}",
                            error["code"],
                            error["message"].as_str().unwrap_or("")
                        ));
                    }
                    return Ok(message.get("result").cloned().unwrap_or(Value::Null));
                }
                Ok(Incoming::Invalid(reason)) => {
                    trace
                        .borrow_mut()
                        .push(json!({"event": "McpMessage", "server": self.name,
                        "status": "invalid", "message": reason}));
                }
                Ok(Incoming::Closed) | Err(RecvTimeoutError::Disconnected) => {
                    self.alive.store(false, Ordering::SeqCst);
                    trace
                        .borrow_mut()
                        .push(json!({"event": "McpServer", "server": self.name,
                        "status": "disconnected"}));
                    return Err(format!("MCP server {} disconnected", self.name));
                }
                Err(RecvTimeoutError::Timeout) => {
                    let _ = self.notify(
                        "notifications/cancelled",
                        json!({"requestId": id, "reason": "timeout"}),
                    );
                    trace
                        .borrow_mut()
                        .push(json!({"event": "McpRequest", "server": self.name,
                        "method": method, "status": "timeout", "id": id}));
                    return Err(format!(
                        "MCP request {method} to {} timed out after {} seconds; the request was cancelled",
                        self.name,
                        timeout.as_secs_f64()
                    ));
                }
            }
        }
    }

    /// MCP stdio shutdown: close the server's input, wait, then SIGTERM and finally SIGKILL.
    fn shutdown(&self, wait: Duration) {
        self.alive.store(false, Ordering::SeqCst);
        drop(self.stdin.borrow_mut().take());
        let mut child = self.child.borrow_mut();
        let group = child.id() as i32;
        for signal in [None, Some(libc::SIGTERM), Some(libc::SIGKILL)] {
            if let Some(signal) = signal {
                // SAFETY: the server was started in its own process group with this positive PID.
                unsafe {
                    libc::kill(-group, signal);
                }
            }
            let started = Instant::now();
            while started.elapsed() < wait {
                if matches!(child.try_wait(), Ok(Some(_)) | Err(_)) {
                    return;
                }
                thread::sleep(Duration::from_millis(10));
            }
        }
        let _ = child.wait();
    }
}

/// Newline-delimited JSON-RPC from the server. An oversized line ends the connection because
/// the framing can no longer be trusted.
fn read_messages(stdout: impl Read, sender: mpsc::Sender<Incoming>) {
    let mut reader = BufReader::new(stdout);
    loop {
        let mut line = Vec::new();
        match reader
            .by_ref()
            .take(MAX_MESSAGE as u64 + 1)
            .read_until(b'\n', &mut line)
        {
            Ok(0) | Err(_) => break,
            Ok(_) if line.len() > MAX_MESSAGE => {
                let _ = sender.send(Incoming::Invalid("message exceeds 10 MiB".into()));
                break;
            }
            Ok(_) => {
                if line.iter().all(u8::is_ascii_whitespace) {
                    continue;
                }
                let incoming = match serde_json::from_slice::<Value>(&line) {
                    Ok(message) => Incoming::Message(message),
                    Err(e) => Incoming::Invalid(format!("invalid JSON: {e}")),
                };
                if sender.send(incoming).is_err() {
                    return;
                }
            }
        }
    }
    let _ = sender.send(Incoming::Closed);
}

#[cfg(test)]
#[path = "mcp_tests.rs"]
mod tests;
