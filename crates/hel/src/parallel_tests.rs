//! H12: which calls run together, and end-to-end runs of `--parallel` against a mock endpoint
//! that answers each request after a delay, several at once.

use std::fs;
use std::io::{self, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use crate::sessions::tests::{child, project, read_request, successful};
use crate::{Call, Session, concurrent_run};

fn call(id: &str, name: &str, args: Value) -> Value {
    json!({"id": id, "type": "function", "function": {"name": name, "arguments": args.to_string()}})
}

fn calls(names: &[&str]) -> Vec<Call> {
    names
        .iter()
        .enumerate()
        .map(|(i, name)| Call::parse(&call(&format!("c{i}"), name, json!({"task": "t"}))))
        .collect()
}

#[test]
fn only_consecutive_read_only_or_delegate_calls_run_together() {
    let root = project();
    let runtime = crate::runtime::Runtime::new_in(&root, &root.join("runs")).unwrap();
    let client = crate::api::Client::new(
        "unused".into(),
        "deepseek-flash",
        16,
        &json!({}),
        Duration::from_secs(1),
    )
    .unwrap();
    let tools = crate::tools::Toolset::new(&["read_file", "glob", "grep", "bash"])
        .unwrap()
        .with_delegate(crate::delegate::Mode::TaskOnly);
    let mut approval = crate::permissions::Input::Unavailable;
    let mut session = Session {
        client: &client,
        runtime: &runtime,
        tools: &tools,
        compaction: None,
        access: crate::permissions::Access::Auto,
        approval: &mut approval,
        depth: 0,
        parallel: true,
    };
    let run = |session: &Session, names: &[&str]| concurrent_run(session, &calls(names));
    assert_eq!(
        run(
            &session,
            &["read_file", "glob", "grep", "bash", "read_file"]
        ),
        3
    );
    assert_eq!(run(&session, &["bash", "read_file", "read_file"]), 0);
    assert_eq!(
        run(&session, &["delegate_task", "delegate_task", "read_file"]),
        2
    );
    assert_eq!(run(&session, &["read_file", "delegate_task"]), 1);
    // write_file is not offered here, and a tool outside the set never joins a group.
    assert_eq!(run(&session, &["write_file", "read_file"]), 0);
    session.depth = 1;
    assert_eq!(run(&session, &["delegate_task", "delegate_task"]), 0);
    assert_eq!(run(&session, &["read_file", "read_file"]), 2);
    session.depth = 0;
    session.parallel = false;
    assert_eq!(run(&session, &["read_file", "read_file"]), 0);
    assert_eq!(run(&session, &["delegate_task", "delegate_task"]), 0);
}

#[test]
fn parallel_flag_is_off_unless_given() {
    let parse = |list: &[&str]| crate::parse_args(list.iter().map(|s| s.to_string())).unwrap();
    assert!(!parse(&[]).parallel);
    assert!(parse(&["--parallel"]).parallel);
}

/// Answers every request `delay` after it arrives, each connection on its own thread, using
/// `respond` to pick the reply from the request. Returns the URL and the requests received.
fn delayed_server(
    count: usize,
    delay: Duration,
    respond: fn(&Value) -> Value,
) -> (String, thread::JoinHandle<Vec<Value>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    listener.set_nonblocking(true).unwrap();
    let join = thread::spawn(move || {
        let started = Instant::now();
        let received = Arc::new(Mutex::new(Vec::new()));
        let mut workers = Vec::new();
        while workers.len() < count {
            match listener.accept() {
                Ok((mut stream, _)) => {
                    let received = Arc::clone(&received);
                    workers.push(thread::spawn(move || {
                        let request = read_request(&mut stream);
                        thread::sleep(delay);
                        let message = respond(&request);
                        let body = json!({"model": request["model"], "choices": [{"finish_reason": "stop", "message": message}],
                            "usage": {"prompt_tokens": 100, "completion_tokens": 5, "prompt_cache_hit_tokens": 0}}).to_string();
                        write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", body.len(), body).unwrap();
                        received.lock().unwrap().push(request);
                    }));
                }
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                    assert!(
                        started.elapsed() < Duration::from_secs(20),
                        "mock request timeout"
                    );
                    thread::sleep(Duration::from_millis(5));
                }
                Err(e) => panic!("mock accept: {e}"),
            }
        }
        for worker in workers {
            worker.join().unwrap();
        }
        Arc::try_unwrap(received).unwrap().into_inner().unwrap()
    });
    (url, join)
}

const MODULES: [&str; 3] = ["auth", "billing", "search"];

/// The parent asks for three subagents at once, each child answers with its module, and the
/// parent answers last.
fn three_children(request: &Value) -> Value {
    let messages = request["messages"].as_array().unwrap();
    let last = messages.last().unwrap();
    if last["role"] == "tool" {
        return json!({"role": "assistant", "content": "done"});
    }
    let text = last["content"].as_str().unwrap_or_default();
    if let Some(module) = MODULES
        .iter()
        .find(|m| text.contains(&format!("module {m}")))
    {
        return json!({"role": "assistant", "content": format!("{module} found")});
    }
    let calls: Vec<Value> = MODULES
        .iter()
        .enumerate()
        .map(|(i, m)| {
            call(
                &format!("d{i}"),
                "delegate_task",
                json!({"task": format!("look at module {m}")}),
            )
        })
        .collect();
    json!({"role": "assistant", "content": null, "tool_calls": calls})
}

fn jsonl(path: &Path) -> Vec<Value> {
    fs::read_to_string(path)
        .unwrap()
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect()
}

/// Runs the three-child conversation and returns the requests and the delegation trace.
fn delegate_three(extra: &[&str]) -> (Vec<Value>, Vec<Value>) {
    let root = project();
    let run_dir: PathBuf = root.join("run");
    fs::create_dir(&run_dir).unwrap();
    let context = run_dir.join("context.json");
    fs::write(
        &context,
        serde_json::to_string(&crate::manual_context()).unwrap(),
    )
    .unwrap();
    let record = run_dir.join("record.json");
    let (url, server) = delayed_server(5, Duration::from_millis(300), three_children);
    let mut args = vec![
        "--tools",
        "read_file",
        "--access",
        "auto",
        "--no-compaction",
        "--delegate",
        "task-only",
        "--instruction",
        "check three modules",
        "--context",
        context.to_str().unwrap(),
        "--record",
        record.to_str().unwrap(),
    ];
    args.extend_from_slice(extra);
    successful(child(&root, &args, Some(&url)).output().unwrap());
    (
        server.join().unwrap(),
        jsonl(&run_dir.join("raw/delegations.jsonl")),
    )
}

fn spans(trace: &[Value]) -> Vec<(u64, u64)> {
    trace
        .iter()
        .map(|d| {
            (
                d["started_at_ms"].as_u64().unwrap(),
                d["ended_at_ms"].as_u64().unwrap(),
            )
        })
        .collect()
}

/// The parent's last request: its conversation with the three results in the order it asked.
fn assert_results_in_call_order(requests: &[Value]) {
    let last = requests
        .iter()
        .find(|r| r["messages"].as_array().unwrap().last().unwrap()["role"] == "tool")
        .unwrap();
    let messages = last["messages"].as_array().unwrap();
    let results: Vec<(&str, &str)> = messages[messages.len() - 3..]
        .iter()
        .map(|m| {
            (
                m["tool_call_id"].as_str().unwrap(),
                m["content"].as_str().unwrap(),
            )
        })
        .collect();
    assert_eq!(
        results,
        [
            ("d0", "auth found"),
            ("d1", "billing found"),
            ("d2", "search found")
        ]
    );
}

#[test]
fn parallel_children_overlap_and_return_in_call_order() {
    let (requests, trace) = delegate_three(&["--parallel"]);
    assert_eq!(requests.len(), 5);
    assert_results_in_call_order(&requests);
    assert_eq!(trace.len(), 3);
    for (i, d) in trace.iter().enumerate() {
        assert_eq!(d["seq"], i + 1);
        assert_eq!(d["call_id"], format!("d{i}"));
        assert_eq!(d["concurrent"], 3);
    }
    let spans = spans(&trace);
    let latest_start = spans.iter().map(|s| s.0).max().unwrap();
    let earliest_end = spans.iter().map(|s| s.1).min().unwrap();
    assert!(
        latest_start < earliest_end,
        "children did not overlap: {spans:?}"
    );
}

#[test]
fn without_the_flag_children_run_one_after_another() {
    let (requests, trace) = delegate_three(&[]);
    assert_results_in_call_order(&requests);
    let spans = spans(&trace);
    for pair in spans.windows(2) {
        assert!(pair[0].1 <= pair[1].0, "children overlapped: {spans:?}");
    }
    assert!(trace.iter().all(|d| d["concurrent"] == 1));
}

/// Three reads in one response: with the flag they run together, and the results still enter
/// the conversation in the order asked.
#[test]
fn parallel_reads_keep_call_order() {
    let root = project();
    for (name, text) in [("a.txt", "alpha"), ("b.txt", "beta"), ("c.txt", "gamma")] {
        fs::write(root.join(name), text).unwrap();
    }
    fn reads(request: &Value) -> Value {
        let messages = request["messages"].as_array().unwrap();
        if messages.last().unwrap()["role"] == "tool" {
            return json!({"role": "assistant", "content": "read"});
        }
        let calls: Vec<Value> = ["a.txt", "b.txt", "c.txt"]
            .iter()
            .enumerate()
            .map(|(i, p)| call(&format!("r{i}"), "read_file", json!({"path": p})))
            .collect();
        json!({"role": "assistant", "content": null, "tool_calls": calls})
    }
    let (url, server) = delayed_server(2, Duration::from_millis(10), reads);
    successful(
        child(
            &root,
            &[
                "--tools",
                "read_file",
                "--no-compaction",
                "--parallel",
                "--instruction",
                "read",
            ],
            Some(&url),
        )
        .output()
        .unwrap(),
    );
    let requests = server.join().unwrap();
    let last = requests
        .iter()
        .find(|r| r["messages"].as_array().unwrap().last().unwrap()["role"] == "tool")
        .unwrap();
    let messages = last["messages"].as_array().unwrap();
    let results: Vec<(&str, &str)> = messages[messages.len() - 3..]
        .iter()
        .map(|m| {
            (
                m["tool_call_id"].as_str().unwrap(),
                m["content"].as_str().unwrap(),
            )
        })
        .collect();
    assert_eq!(results, [("r0", "alpha"), ("r1", "beta"), ("r2", "gamma")]);
}
