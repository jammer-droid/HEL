use super::*;
use crate::permissions::{Access, Input};
use crate::sessions::tests::{child, mock_responses, project, successful};
use crate::tools::Toolset;
use std::cell::Cell;
use std::path::PathBuf;

/// A stdio MCP server driven by tool names. Every received message is logged for assertions.
const SERVER: &str = r#"
import json, os, signal, sys, time
LOG = os.path.join(os.path.dirname(os.path.abspath(__file__)), "log.jsonl")
if "--stubborn" in sys.argv:
    signal.signal(signal.SIGTERM, lambda *_: None)
def schema(props=None):
    return {"type": "object", "properties": props or {}}
TOOLS = [
    {"name": "echo", "description": "Echo text back.\nSecond line.", "inputSchema": schema({"text": {"type": "string"}})},
    {"name": "fail", "description": "Return a tool error.", "inputSchema": schema()},
    {"name": "rpcerr", "description": "Return a protocol error.", "inputSchema": schema()},
    {"name": "image", "description": "Return an image and a caption.", "inputSchema": schema()},
    {"name": "structured", "description": "Return structured content only.", "inputSchema": schema()},
    {"name": "slow", "description": "Answer late.", "inputSchema": schema()},
    {"name": "chatty", "description": "Send a notification and a request first.", "inputSchema": schema()},
    {"name": "die", "description": "Exit.", "inputSchema": schema()},
    {"name": "a.b", "description": "Dotted name.", "inputSchema": schema()},
    {"name": "x" * 70, "description": "Too long.", "inputSchema": schema()},
    {"name": "noschema", "description": "Missing schema."},
]
def send(message):
    sys.stdout.write(json.dumps(message) + "\n"); sys.stdout.flush()
def text(value, error=False):
    return {"content": [{"type": "text", "text": value}], "isError": error}
for line in sys.stdin:
    message = json.loads(line)
    with open(LOG, "a") as log:
        log.write(json.dumps(message) + "\n")
    method, mid = message.get("method"), message.get("id")
    if mid is None or method is None:
        continue
    if method == "initialize":
        send({"jsonrpc": "2.0", "id": mid, "result": {"protocolVersion": "2025-11-25", "capabilities": {"tools": {}}, "serverInfo": {"name": "t", "version": "0"}}})
    elif method == "tools/list":
        if message.get("params", {}).get("cursor") == "page2":
            send({"jsonrpc": "2.0", "id": mid, "result": {"tools": TOOLS[6:]}})
        else:
            send({"jsonrpc": "2.0", "id": mid, "result": {"tools": TOOLS[:6], "nextCursor": "page2"}})
    elif method == "tools/call":
        name = message["params"]["name"]
        args = message["params"].get("arguments") or {}
        if name == "echo":
            result = text(args.get("text", ""))
        elif name == "fail":
            result = text("tool failed", True)
        elif name == "rpcerr":
            send({"jsonrpc": "2.0", "id": mid, "error": {"code": -32000, "message": "boom"}}); continue
        elif name == "image":
            result = {"content": [{"type": "image", "data": "AAAA", "mimeType": "image/png"}, {"type": "text", "text": "caption"}]}
        elif name == "structured":
            result = {"content": [], "structuredContent": {"port": 8437}}
        elif name == "slow":
            time.sleep(1.5); result = text("late")
        elif name == "chatty":
            send({"jsonrpc": "2.0", "method": "notifications/message", "params": {"level": "info", "data": "hi"}})
            send({"jsonrpc": "2.0", "id": "srv-1", "method": "roots/list"})
            result = text("chatted")
        elif name == "die":
            sys.exit(0)
        else:
            result = text("?")
        send({"jsonrpc": "2.0", "id": mid, "result": result})
"#;

struct Fixture {
    root: PathBuf,
}

impl Fixture {
    fn new(servers: Value) -> Self {
        let root = project();
        fs::create_dir_all(root.join(".hel/mcp")).unwrap();
        fs::write(root.join(".hel/mcp/server.py"), SERVER).unwrap();
        fs::write(
            root.join(".hel/mcp.json"),
            serde_json::to_vec_pretty(&json!({ "servers": servers })).unwrap(),
        )
        .unwrap();
        Self { root }
    }

    fn standard() -> Self {
        Self::new(
            json!({"t": {"command": "python3", "args": [".hel/mcp/server.py"], "timeout": 1}}),
        )
    }

    fn runtime(&self) -> Runtime {
        Runtime::new(&self.root).unwrap()
    }

    fn trusted(&self, runtime: &Runtime) -> Mcp {
        Mcp::load_with(runtime, quick(), |_| true)
    }

    fn calls(&self) -> Vec<Value> {
        self.log()
            .into_iter()
            .filter(|m| m["method"] == "tools/call")
            .collect()
    }

    fn log(&self) -> Vec<Value> {
        fs::read_to_string(self.root.join(".hel/mcp/log.jsonl"))
            .unwrap_or_default()
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn quick() -> Limits {
    Limits {
        startup: Duration::from_secs(5),
        shutdown_wait: Duration::from_millis(300),
    }
}

fn names(definitions: &Value) -> Vec<String> {
    definitions
        .as_array()
        .unwrap()
        .iter()
        .map(|d| d["function"]["name"].as_str().unwrap().to_string())
        .collect()
}

fn load(runtime: &mut Runtime, fixture: &Fixture) {
    runtime.mcp = fixture.trusted(runtime);
}

fn call(runtime: &Runtime, tools: &Toolset, name: &str, args: Value) -> Result<String, String> {
    tools
        .call(runtime, name, &args, Access::Auto, &mut Input::Unavailable)
        .result
}

#[test]
fn absent_configuration_changes_nothing() {
    let root = project();
    let runtime = Runtime::new(&root).unwrap();
    let mcp = Mcp::load_with(&runtime, quick(), |_| panic!("nothing to review"));
    assert!(!mcp.active());
    assert!(mcp.definitions().is_empty());
    assert_eq!(mcp.extend_system(None), None);
    let tools = Toolset::new(&["read_file"]).unwrap();
    assert_eq!(names(&tools.request_definitions(&runtime)), ["read_file"]);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn untrusted_configuration_starts_no_server_and_reviews_changes_again() {
    let fixture = Fixture::standard();
    let runtime = fixture.runtime();
    let mcp = Mcp::load_with(&runtime, quick(), |_| false);
    assert!(!mcp.active());
    assert!(fixture.log().is_empty(), "no server may start before trust");
    assert!(!fixture.root.join(".hel/mcp-trust.json").exists());
    drop(mcp);

    let reviews = Cell::new(0);
    let mcp = Mcp::load_with(&runtime, quick(), |display| {
        assert!(display.contains("server.py"));
        reviews.set(reviews.get() + 1);
        true
    });
    assert!(mcp.active());
    drop(mcp);
    let mcp = Mcp::load_with(&runtime, quick(), |_| panic!("same trusted bytes"));
    assert!(mcp.active());
    drop(mcp);
    let config = fixture.root.join(".hel/mcp.json");
    let mut bytes = fs::read(&config).unwrap();
    bytes.push(b'\n');
    fs::write(&config, bytes).unwrap();
    let mcp = Mcp::load_with(&runtime, quick(), |_| {
        reviews.set(reviews.get() + 1);
        false
    });
    assert!(!mcp.active());
    assert_eq!(reviews.get(), 2);
}

#[test]
fn lists_names_and_descriptions_first_and_excludes_unusable_tools() {
    let fixture = Fixture::standard();
    let mut runtime = fixture.runtime();
    load(&mut runtime, &fixture);
    let section = runtime.mcp.system_section().unwrap();
    assert!(section.contains("- mcp__t__echo: Echo text back."));
    assert!(!section.contains("Second line"), "{section}");
    assert!(
        section.contains("mcp__t__a_b"),
        "names are sanitized: {section}"
    );
    assert!(
        section.contains("mcp__t__die"),
        "second page is listed: {section}"
    );
    assert!(!section.contains("noschema") && !section.contains("xxxxxxxx"));
    assert!(
        !section.contains("\"properties\""),
        "no schema before loading"
    );
    let system = runtime
        .mcp
        .extend_system(Some(json!({"role": "system", "content": "base"})))
        .unwrap();
    assert!(
        system["content"]
            .as_str()
            .unwrap()
            .starts_with("base\n\nMCP tools")
    );
    let tools = Toolset::new(&["read_file"]).unwrap();
    assert_eq!(
        names(&tools.request_definitions(&runtime)),
        ["read_file", LOAD]
    );
    let trace = runtime.mcp.trace.borrow();
    for excluded in ["noschema", "exceeds 64"] {
        assert!(
            trace.iter().any(|e| e.to_string().contains(excluded)),
            "{excluded}"
        );
    }
}

#[test]
fn loading_adds_definitions_and_calls_pass_the_permission_gate() {
    let fixture = Fixture::standard();
    let mut runtime = fixture.runtime();
    load(&mut runtime, &fixture);
    let tools = Toolset::new(&["read_file"]).unwrap();
    let echo = "mcp__t__echo";

    let early = call(&runtime, &tools, echo, json!({"text": "hi"})).unwrap_err();
    assert!(early.contains("not loaded"), "{early}");
    assert!(
        fixture.calls().is_empty(),
        "unloaded calls never reach the server"
    );

    let unknown = call(&runtime, &tools, LOAD, json!({"names": ["mcp__t__nope"]})).unwrap_err();
    assert!(unknown.contains("unknown MCP tool"), "{unknown}");
    let loaded = call(&runtime, &tools, LOAD, json!({"names": [echo]})).unwrap();
    assert!(loaded.contains("loaded; callable from the next request"));
    assert!(
        fixture.calls().is_empty(),
        "loading does not contact the server"
    );
    let again = call(&runtime, &tools, LOAD, json!({"names": [echo]})).unwrap();
    assert!(again.contains("already loaded"));
    let definitions = tools.request_definitions(&runtime);
    assert_eq!(names(&definitions), ["read_file", LOAD, echo]);
    assert_eq!(
        definitions[2]["function"]["parameters"]["properties"]["text"]["type"],
        "string"
    );

    // Loading is a read; MCP calls are execution in every access level.
    let read_only = tools.call(
        &runtime,
        LOAD,
        &json!({"names": [echo]}),
        Access::ReadOnly,
        &mut Input::Unavailable,
    );
    assert!(read_only.result.is_ok());
    let denied = tools.call(
        &runtime,
        echo,
        &json!({"text": "x"}),
        Access::ReadOnly,
        &mut Input::Unavailable,
    );
    assert!(denied.result.unwrap_err().contains("permission denied"));
    assert_eq!(denied.trace.action, Some(Action::Execute));
    let refused = tools.call(
        &runtime,
        echo,
        &json!({"text": "x"}),
        Access::Confirm,
        &mut Input::Script([false].into()),
    );
    assert!(refused.result.is_err());
    assert!(
        fixture.calls().is_empty(),
        "denied calls never reach the server"
    );
    let approved = tools.call(
        &runtime,
        echo,
        &json!({"text": "approved"}),
        Access::Confirm,
        &mut Input::Script([true].into()),
    );
    assert_eq!(approved.result.unwrap(), "approved");
    assert_eq!(
        call(&runtime, &tools, echo, json!({"text": "auto"})).unwrap(),
        "auto"
    );
    let sent = fixture.calls();
    assert_eq!(sent.len(), 2);
    assert_eq!(
        sent[1]["params"],
        json!({"name": "echo", "arguments": {"text": "auto"}})
    );
}

#[test]
fn pre_tool_use_hooks_see_mcp_tool_names() {
    let fixture = Fixture::standard();
    let hooks = json!({"hooks": {"PreToolUse": [{"matcher": "^mcp__",
        "hooks": [{"type": "command", "command": "echo external tool blocked >&2; exit 2"}]}]}});
    let bytes = serde_json::to_vec(&hooks).unwrap();
    fs::write(fixture.root.join(".hel/hooks.json"), &bytes).unwrap();
    save_trust(
        &fixture.root.join(".hel"),
        "hooks-trust.json",
        &Trust {
            schema_version: 1,
            project: fixture.root.clone(),
            sha256: format!("{:x}", Sha256::digest(&bytes)),
        },
    )
    .unwrap();
    let mut runtime = fixture.runtime();
    load(&mut runtime, &fixture);
    runtime.hooks = crate::hooks::Hooks::load(&fixture.root, "session");
    let tools = Toolset::new(&["read_file"]).unwrap();
    call(&runtime, &tools, LOAD, json!({"names": ["mcp__t__echo"]})).unwrap();
    let blocked = tools.call(
        &runtime,
        "mcp__t__echo",
        &json!({"text": "x"}),
        Access::Auto,
        &mut Input::Unavailable,
    );
    assert!(blocked.trace.hook_blocked);
    assert!(
        blocked
            .result
            .unwrap_err()
            .contains("external tool blocked")
    );
    assert!(fixture.calls().is_empty());
}

#[test]
fn results_and_errors_are_returned_to_the_model() {
    let fixture = Fixture::standard();
    let mut runtime = fixture.runtime();
    load(&mut runtime, &fixture);
    let tools = Toolset::new(&["read_file"]).unwrap();
    let all =
        ["fail", "rpcerr", "image", "structured", "chatty", "echo"].map(|n| format!("mcp__t__{n}"));
    call(&runtime, &tools, LOAD, json!({"names": all})).unwrap();
    let run = |name: &str| call(&runtime, &tools, &format!("mcp__t__{name}"), json!({}));
    assert_eq!(run("fail").unwrap_err(), "tool failed");
    assert_eq!(run("rpcerr").unwrap_err(), "MCP error -32000: boom");
    assert_eq!(run("image").unwrap(), "[image content omitted]\ncaption");
    assert_eq!(run("structured").unwrap(), r#"{"port":8437}"#);
    assert_eq!(run("chatty").unwrap(), "chatted");
    // The server reads stdin in order, so a later call proves it has logged hel's reply.
    call(&runtime, &tools, "mcp__t__echo", json!({"text": "sync"})).unwrap();
    let reply = fixture
        .log()
        .into_iter()
        .find(|m| m["id"] == "srv-1")
        .expect("server request answered");
    assert_eq!(reply["error"]["code"], -32601);
}

#[test]
fn timeout_cancels_the_request_and_keeps_the_server() {
    let fixture = Fixture::standard();
    let mut runtime = fixture.runtime();
    load(&mut runtime, &fixture);
    let tools = Toolset::new(&["read_file"]).unwrap();
    call(
        &runtime,
        &tools,
        LOAD,
        json!({"names": ["mcp__t__slow", "mcp__t__echo"]}),
    )
    .unwrap();
    let pid = runtime.mcp.servers[0].child.borrow().id();
    let slow = call(&runtime, &tools, "mcp__t__slow", json!({})).unwrap_err();
    assert!(slow.contains("timed out"), "{slow}");
    // The late "late" answer to the cancelled request must not satisfy the next call.
    assert_eq!(
        call(&runtime, &tools, "mcp__t__echo", json!({"text": "after"})).unwrap(),
        "after"
    );
    let cancelled = fixture
        .log()
        .into_iter()
        .find(|m| m["method"] == "notifications/cancelled")
        .expect("cancellation sent");
    let slow_id = fixture.calls()[0]["id"].clone();
    assert_eq!(cancelled["params"]["requestId"], slow_id);
    assert_eq!(runtime.mcp.servers[0].child.borrow().id(), pid);
    assert!(runtime.mcp.servers[0].alive.get());
}

#[test]
fn a_server_that_exits_is_not_restarted() {
    let fixture = Fixture::standard();
    let mut runtime = fixture.runtime();
    load(&mut runtime, &fixture);
    let tools = Toolset::new(&["read_file"]).unwrap();
    call(
        &runtime,
        &tools,
        LOAD,
        json!({"names": ["mcp__t__die", "mcp__t__echo"]}),
    )
    .unwrap();
    let died = call(&runtime, &tools, "mcp__t__die", json!({})).unwrap_err();
    assert!(died.contains("disconnected"), "{died}");
    let after = call(&runtime, &tools, "mcp__t__echo", json!({"text": "x"})).unwrap_err();
    assert!(after.contains("disconnected"), "{after}");
    let starts = fixture
        .log()
        .iter()
        .filter(|m| m["method"] == "initialize")
        .count();
    assert_eq!(starts, 1);
}

#[test]
fn failed_servers_are_skipped_and_others_continue() {
    let fixture = Fixture::new(json!({
        "t": {"command": "python3", "args": [".hel/mcp/server.py"]},
        "missing": {"command": "/nonexistent/mcp-server"},
        "silent": {"command": "python3", "args": ["-c", "import time; time.sleep(30)"]},
    }));
    let runtime = fixture.runtime();
    let limits = Limits {
        startup: Duration::from_millis(500),
        shutdown_wait: Duration::from_millis(200),
    };
    let mcp = Mcp::load_with(&runtime, limits, |_| true);
    assert_eq!(mcp.servers.len(), 1);
    assert!(mcp.tools.keys().all(|name| name.starts_with("mcp__t__")));
    let trace: Vec<String> = mcp.trace.borrow().iter().map(|e| e.to_string()).collect();
    assert!(trace.iter().any(|e| e.contains("missing skipped")));
    assert!(
        trace
            .iter()
            .any(|e| e.contains("silent skipped") && e.contains("timed out"))
    );
}

#[test]
fn exit_closes_input_then_terminates_stubborn_servers() {
    for args in [
        vec![".hel/mcp/server.py"],
        vec![".hel/mcp/server.py", "--stubborn"],
    ] {
        let fixture = Fixture::new(json!({"t": {"command": "python3", "args": args}}));
        let runtime = fixture.runtime();
        let mcp = fixture.trusted(&runtime);
        let pid = mcp.servers[0].child.borrow().id() as i32;
        let started = Instant::now();
        drop(mcp);
        // SAFETY: signal 0 only checks whether the reaped process still exists.
        assert_eq!(
            unsafe { libc::kill(pid, 0) },
            -1,
            "server {pid} still running"
        );
        assert!(started.elapsed() < Duration::from_secs(3));
    }
}

#[test]
fn cli_offers_loaded_tools_from_the_next_request_and_resume_reloads() {
    let fixture = Fixture::standard();
    let bytes = fs::read(fixture.root.join(".hel/mcp.json")).unwrap();
    // The child process has no terminal, so trust is recorded beforehand.
    save_trust(
        &fixture.root.join(".hel"),
        TRUST,
        &Trust {
            schema_version: 1,
            project: fixture.root.clone(),
            sha256: format!("{:x}", Sha256::digest(&bytes)),
        },
    )
    .unwrap();
    let tool_call = |id: &str, name: &str, args: Value| {
        json!({"role": "assistant", "content": null, "tool_calls": [{"id": id, "type": "function",
            "function": {"name": name, "arguments": args.to_string()}}]})
    };
    let (url, server) = mock_responses(vec![
        tool_call("c1", LOAD, json!({"names": ["mcp__t__echo"]})),
        tool_call("c2", "mcp__t__echo", json!({"text": "8437"})),
        json!({"role": "assistant", "content": "port is 8437"}),
        json!({"role": "assistant", "content": "resumed"}),
    ]);
    let args = ["--tools", "read_file", "--access", "auto", "--no-env"];
    let mut first = args.to_vec();
    first.extend(["--instruction", "look up the port"]);
    successful(child(&fixture.root, &first, Some(&url)).output().unwrap());
    let sessions = fixture.root.join(".hel/sessions");
    let id = fs::read_dir(&sessions)
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .file_name()
        .into_string()
        .unwrap();
    let mut second = args.to_vec();
    second.extend(["--resume", &id, "--instruction", "again"]);
    successful(child(&fixture.root, &second, Some(&url)).output().unwrap());
    let requests = server.join().unwrap();
    let tools = |i: usize| names(&requests[i]["tools"]);
    assert_eq!(tools(0), ["read_file", LOAD]);
    assert_eq!(tools(1), ["read_file", LOAD, "mcp__t__echo"]);
    assert_eq!(tools(2), ["read_file", LOAD, "mcp__t__echo"]);
    assert_eq!(
        tools(3),
        ["read_file", LOAD],
        "resume starts without loaded tools"
    );
    for request in &requests {
        let system = request["messages"][0]["content"].as_str().unwrap();
        assert!(
            system.contains("- mcp__t__echo: Echo text back."),
            "{system}"
        );
    }
    let messages = requests[2]["messages"].as_array().unwrap();
    assert_eq!(messages.last().unwrap()["content"], "8437");
    let resumed = requests[3]["messages"].as_array().unwrap();
    assert!(
        resumed.iter().any(|m| m["tool_call_id"] == "c2"),
        "conversation restored"
    );
    let starts = fixture
        .log()
        .iter()
        .filter(|m| m["method"] == "initialize")
        .count();
    assert_eq!(starts, 2, "each process connects its own server");
}
