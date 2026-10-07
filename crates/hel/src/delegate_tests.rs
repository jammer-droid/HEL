//! End-to-end delegation through the real CLI entry against a mock model endpoint.

use std::fs;
use std::path::Path;

use serde_json::{Value, json};

use crate::sessions::tests::{child, mock_responses, project, successful};

fn tool_call(id: &str, name: &str, args: Value) -> Value {
    json!({"role": "assistant", "content": null, "tool_calls": [{"id": id, "type": "function",
        "function": {"name": name, "arguments": args.to_string()}}]})
}

fn answer(text: &str) -> Value {
    json!({"role": "assistant", "content": text})
}

fn names(tools: &Value) -> Vec<&str> {
    tools
        .as_array()
        .unwrap()
        .iter()
        .map(|t| t["function"]["name"].as_str().unwrap())
        .collect()
}

fn roles(request: &Value) -> Vec<&str> {
    request["messages"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["role"].as_str().unwrap())
        .collect()
}

fn jsonl(path: &Path) -> Vec<Value> {
    fs::read_to_string(path)
        .unwrap()
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect()
}

/// Parent reads, delegates; the child edits, tries to delegate again, answers; the parent
/// answers. Returns the requests the model endpoint received and the run directory.
fn run(mode: &str) -> (Vec<Value>, std::path::PathBuf) {
    let root = project();
    fs::write(root.join("services.ini"), "[api]\nport = 8143\n").unwrap();
    fs::write(root.join("api.yaml"), "port: 7000\n").unwrap();
    let run_dir = root.join("run");
    fs::create_dir(&run_dir).unwrap();
    fs::write(
        run_dir.join("context.json"),
        serde_json::to_string(&crate::manual_context()).unwrap(),
    )
    .unwrap();
    let (url, server) = mock_responses(vec![
        tool_call("p1", "read_file", json!({"path": "services.ini"})),
        tool_call(
            "p2",
            "delegate_task",
            json!({"task": "set api.yaml port to 8143"}),
        ),
        tool_call(
            "c1",
            "search_replace",
            json!({"path": "api.yaml", "search": "port: 7000", "replace": "port: 8143"}),
        ),
        tool_call("c2", "delegate_task", json!({"task": "nested"})),
        answer("changed api.yaml"),
        answer("8143"),
    ]);
    let context = run_dir.join("context.json");
    let record = run_dir.join("record.json");
    successful(
        child(
            &root,
            &[
                "--tools",
                "read_file,search_replace",
                "--access",
                "auto",
                "--no-compaction",
                "--delegate",
                mode,
                "--instruction",
                "read, delegate, report",
                "--context",
                context.to_str().unwrap(),
                "--record",
                record.to_str().unwrap(),
            ],
            Some(&url),
        )
        .output()
        .unwrap(),
    );
    assert_eq!(
        fs::read_to_string(root.join("api.yaml")).unwrap(),
        "port: 8143\n"
    );
    (server.join().unwrap(), run_dir)
}

#[test]
fn full_copy_continues_the_parent_request_and_returns_the_answer() {
    let (requests, run_dir) = run("full");
    // The child's first request is the parent's last request plus the task.
    let parent = requests[1]["messages"].as_array().unwrap();
    let child_first = requests[2]["messages"].as_array().unwrap();
    assert_eq!(child_first[..parent.len()], parent[..]);
    assert_eq!(
        child_first[parent.len()],
        json!({"role": "user", "content": crate::delegate::child_task("set api.yaml port to 8143")})
    );
    // Parent and child send the same tool list, delegate_task included.
    for request in &requests {
        assert_eq!(request["tools"], requests[0]["tools"]);
    }
    assert_eq!(
        names(&requests[0]["tools"]),
        ["read_file", "search_replace", "delegate_task"]
    );
    let description = requests[0]["tools"][2]["function"]["description"]
        .as_str()
        .unwrap();
    assert!(description.contains("a copy of this conversation so far"));
    // The nested call was refused inside the child.
    let child_last = requests[4]["messages"].as_array().unwrap();
    assert_eq!(
        child_last.last().unwrap()["content"],
        format!("error: {}", crate::delegate::DEPTH_LIMIT)
    );
    // The parent resumes from its own conversation with only the child's answer added.
    let resumed = requests[5]["messages"].as_array().unwrap();
    assert_eq!(resumed[..parent.len()], parent[..]);
    assert_eq!(resumed[parent.len()]["tool_calls"][0]["id"], "p2");
    assert_eq!(
        resumed[parent.len() + 1],
        json!({"role": "tool", "tool_call_id": "p2", "content": "changed api.yaml"})
    );
    assert_eq!(resumed.len(), parent.len() + 2);

    let raw = jsonl(&run_dir.join("raw/requests.jsonl"));
    let agents: Vec<Option<&str>> = raw.iter().map(|e| e["agent"].as_str()).collect();
    assert_eq!(
        agents,
        [
            None,
            None,
            Some("child"),
            Some("child"),
            Some("child"),
            None
        ]
    );
    let delegations = jsonl(&run_dir.join("raw/delegations.jsonl"));
    assert_eq!(delegations.len(), 1);
    let trace = &delegations[0];
    assert_eq!(trace["mode"], "full");
    assert_eq!(trace["task"], "set api.yaml port to 8143");
    assert_eq!(trace["child_start_messages"], parent.len() + 1);
    assert_eq!(trace["child_requests"].as_array().unwrap().len(), 3);
    let child_tools: Vec<&str> = trace["child_events"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["name"].as_str().unwrap())
        .collect();
    assert_eq!(child_tools, ["search_replace", "delegate_task"]);
    // H13: the child's refused call carries what it returned, like a parent call would.
    let refused = &trace["child_events"][1];
    assert_eq!(refused["ok"], false);
    assert_eq!(refused["error"], crate::delegate::DEPTH_LIMIT);
    assert!(refused.get("exit_code").is_none());
    assert!(trace["child_events"][0].get("error").is_none());
    assert_eq!(
        trace["child_delegations"][0]["result"]["error"],
        crate::delegate::DEPTH_LIMIT
    );
    assert_eq!(trace["result"]["ok"], "changed api.yaml");

    let record: Value =
        serde_json::from_str(&fs::read_to_string(run_dir.join("record.json")).unwrap()).unwrap();
    assert_eq!(record["usage"]["model_calls"]["value"], 6.0);
    assert_eq!(record["outcome"]["final_output"], "8143");
    let parent_tools: Vec<&str> = record["events"]
        .as_array()
        .unwrap()
        .iter()
        .map(|e| e["name"].as_str().unwrap())
        .collect();
    assert_eq!(parent_tools, ["read_file", "delegate_task"]);
}

#[test]
fn no_tools_drops_tool_calls_and_results() {
    let (requests, run_dir) = run("no-tools");
    assert_eq!(roles(&requests[2]), ["system", "user", "user"]);
    let child_first = requests[2]["messages"].as_array().unwrap();
    assert_eq!(
        child_first[..2],
        requests[0]["messages"].as_array().unwrap()[..2]
    );
    assert!(
        requests[0]["tools"][2]["function"]["description"]
            .as_str()
            .unwrap()
            .contains("does not receive tool calls or tool results")
    );
    let trace = &jsonl(&run_dir.join("raw/delegations.jsonl"))[0];
    assert_eq!(trace["mode"], "no-tools");
    assert_eq!(trace["child_start_messages"], 3);
}

#[test]
fn task_only_starts_from_the_system_prompt() {
    let (requests, _) = run("task-only");
    assert_eq!(roles(&requests[2]), ["system", "user"]);
    assert_eq!(requests[2]["messages"][0], requests[0]["messages"][0]);
    assert_eq!(
        requests[2]["messages"][1]["content"],
        crate::delegate::child_task("set api.yaml port to 8143")
    );
}

/// H13: a failed call records what the tool returned; a bash call also records its exit code.
#[test]
#[cfg(target_os = "macos")]
fn failed_calls_record_exit_code_and_error() {
    let root = project();
    fs::write(root.join("a.txt"), "one\n").unwrap();
    fs::write(root.join("b.txt"), "two\n").unwrap();
    let run_dir = root.join("run");
    fs::create_dir(&run_dir).unwrap();
    fs::write(
        run_dir.join("context.json"),
        serde_json::to_string(&crate::manual_context()).unwrap(),
    )
    .unwrap();
    let (url, server) = mock_responses(vec![
        tool_call("b1", "bash", json!({"command": "diff a.txt b.txt"})),
        tool_call("b2", "bash", json!({"command": "cat a.txt"})),
        tool_call("r1", "read_file", json!({"path": "missing.txt"})),
        answer("done"),
    ]);
    let context = run_dir.join("context.json");
    let record = run_dir.join("record.json");
    successful(
        child(
            &root,
            &[
                "--tools",
                "bash,read_file",
                "--access",
                "auto",
                "--no-compaction",
                "--instruction",
                "compare",
                "--context",
                context.to_str().unwrap(),
                "--record",
                record.to_str().unwrap(),
            ],
            Some(&url),
        )
        .output()
        .unwrap(),
    );
    server.join().unwrap();
    let record: Value = serde_json::from_str(&fs::read_to_string(record).unwrap()).unwrap();
    let events = record["events"].as_array().unwrap();
    assert_eq!(events[0]["ok"], false);
    assert_eq!(events[0]["exit_code"], 1);
    let diff = events[0]["error"].as_str().unwrap();
    assert!(
        diff.starts_with("exit=1\n") && diff.contains("< one"),
        "{diff}"
    );
    assert_eq!(events[1]["ok"], true);
    assert_eq!(events[1]["exit_code"], 0);
    assert!(events[1].get("error").is_none());
    assert_eq!(events[2]["ok"], false);
    assert!(events[2].get("exit_code").is_none());
    let missing = events[2]["error"].as_str().unwrap();
    assert!(!missing.starts_with("error: "), "{missing}");
    assert!(missing.contains("No such file or directory"), "{missing}");
}
