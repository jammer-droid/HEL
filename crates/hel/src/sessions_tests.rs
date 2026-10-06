use super::*;
use serde_json::json;
use std::io::{BufRead, BufReader};
use std::net::{TcpListener, TcpStream};
use std::process::{Command, Output, Stdio};
use std::thread;
use std::time::{Duration, Instant};

fn project() -> PathBuf {
    let root = std::env::temp_dir().join(format!("hel-session-test-{}", uuid::Uuid::new_v4()));
    fs::create_dir(&root).unwrap();
    root.canonicalize().unwrap()
}

fn config() -> RequestConfig {
    RequestConfig {
        model: record::ModelRequest {
            provider: "deepseek".into(),
            requested: "deepseek-flash".into(),
            params: json!({}),
        },
        max_output_tokens: 8192,
        compaction: None,
        tools: crate::tools::Toolset::new(crate::tools::DEFAULT)
            .unwrap()
            .definitions()
            .clone(),
    }
}

fn messages() -> Vec<Value> {
    vec![
        json!({"role":"system","content":"old rules"}),
        json!({"role":"user","content":"remember value"}),
        json!({"role":"assistant","content":"done", "reasoning_content":"reason", "tool_calls": null}),
    ]
}

#[test]
fn snapshot_preserves_json_and_restores_meter_only_when_inputs_match() {
    let root = project();
    let store = Store::open(&root, None).unwrap();
    let cfg = config();
    let history = messages();
    let mut meter = Meter::default();
    meter.observed(2, 900);
    store
        .save(
            &cfg,
            &history,
            &meter,
            &Reader::default(),
            &Skills::default(),
        )
        .unwrap();
    let id = store.id.clone();
    drop(store);
    let store = Store::open(&root, Some(&id)).unwrap();
    let restored = store.restore(Some(history[0].clone()), &cfg).unwrap();
    assert_eq!(restored.messages, history);
    assert_eq!(restored.meter, meter);
    let mut changed = cfg.clone();
    changed.max_output_tokens = 4096;
    changed.compaction = Some(crate::context::Policy {
        at: 1000,
        keep_recent: 100,
    });
    assert_eq!(
        store
            .restore(Some(history[0].clone()), &changed)
            .unwrap()
            .meter,
        meter
    );
    for changed in [
        RequestConfig {
            model: record::ModelRequest {
                requested: "other-model".into(),
                ..cfg.model.clone()
            },
            ..cfg.clone()
        },
        RequestConfig {
            tools: json!([]),
            ..cfg.clone()
        },
    ] {
        assert_eq!(
            store
                .restore(Some(history[0].clone()), &changed)
                .unwrap()
                .meter,
            Meter::default()
        );
    }
    let current = json!({"role":"system","content":"new rules"});
    let restored = store.restore(Some(current.clone()), &cfg).unwrap();
    assert_eq!(restored.messages[0], current);
    assert_eq!(&restored.messages[1..], &history[1..]);
    assert_eq!(restored.meter, Meter::default());
    let restored = store.restore(None, &cfg).unwrap();
    assert_eq!(restored.messages, history[1..]);
    assert_eq!(restored.meter, Meter::default());
    drop(store);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn snapshots_reject_unknown_schema_bad_tool_links_and_symlinks() {
    let root = project();
    let store = Store::open(&root, None).unwrap();
    store
        .save(
            &config(),
            &messages(),
            &Meter::default(),
            &Reader::default(),
            &Skills::default(),
        )
        .unwrap();
    let path = store.root.join(SNAPSHOT);
    let original = fs::read(&path).unwrap();
    let mut bad: Value = serde_json::from_slice(&original).unwrap();
    bad["schema_version"] = json!(999);
    fs::write(&path, bad.to_string()).unwrap();
    assert!(
        store
            .restore(None, &config())
            .unwrap_err_text()
            .contains("schema")
    );
    fs::write(&path, &original).unwrap();
    let bad_history = vec![
        json!({"role":"tool","tool_call_id":"missing","content":"x"}),
        json!({"role":"assistant","content":"done"}),
    ];
    assert!(
        store
            .save(
                &config(),
                &bad_history,
                &Meter::default(),
                &Reader::default(),
                &Skills::default()
            )
            .is_err()
    );
    assert_eq!(fs::read(&path).unwrap(), original);
    fs::remove_file(&path).unwrap();
    fs::write(root.join("ordinary"), &original).unwrap();
    std::os::unix::fs::symlink(root.join("ordinary"), &path).unwrap();
    assert!(store.restore(None, &config()).is_err());
    drop(store);
    assert!(Store::open(&root, Some("../ordinary")).is_err());
    assert!(delete(&root, "../ordinary").is_err());
    fs::remove_dir_all(root).unwrap();
}

// Result::unwrap_err otherwise requires Debug on the restored state, which is not a public need.
trait ErrorText {
    fn unwrap_err_text(self) -> String;
}
impl<T> ErrorText for Result<T> {
    fn unwrap_err_text(self) -> String {
        match self {
            Ok(_) => panic!("expected error"),
            Err(e) => e.to_string(),
        }
    }
}

#[test]
fn spill_and_cursor_survive_exit_until_session_deletion() {
    let root = project();
    fs::write(root.join("ordinary"), "keep").unwrap();
    let store = Store::open(&root, None).unwrap();
    let runtime = store.runtime().unwrap();
    let text = "읽기🙂".repeat(4000);
    let spill = runtime.save_spill(text.as_bytes()).unwrap();
    let first = runtime
        .reader
        .read(&runtime, &json!({"path": spill}))
        .unwrap();
    let cursor = first
        .split("\"cursor\":\"")
        .nth(1)
        .unwrap()
        .split('"')
        .next()
        .unwrap()
        .to_string();
    store
        .save(
            &config(),
            &messages(),
            &Meter::default(),
            &runtime.reader,
            &runtime.skills.borrow(),
        )
        .unwrap();
    let id = store.id.clone();
    let run_store = runtime.store.clone();
    drop(runtime);
    drop(store);
    runtime::clean_runs(
        &run_store,
        SystemTime::now() + runtime::RETENTION + Duration::from_secs(1),
    );
    assert!(spill.exists());
    let store = Store::open(&root, Some(&id)).unwrap();
    let state = store.restore(None, &config()).unwrap();
    let mut runtime = store.runtime().unwrap();
    runtime.reader = state.reader;
    let rest = runtime
        .reader
        .read(&runtime, &json!({"cursor": cursor}))
        .unwrap();
    assert!(rest.starts_with("읽기") || rest.starts_with('🙂') || rest.starts_with('기'));
    assert!(runtime.read_open(&store.root.join(SNAPSHOT)).is_err());
    assert!(runtime.edit_open(&spill, false).is_err());
    let peer = Store::open(&root, None).unwrap();
    let peer_runtime = peer.runtime().unwrap();
    assert!(peer_runtime.read_open(&spill).is_err());
    assert!(
        delete(&root, &id)
            .unwrap_err()
            .to_string()
            .contains("in use")
    );
    fs::write(&spill, "modified by host").unwrap();
    assert!(
        runtime
            .reader
            .read(&runtime, &json!({"cursor": cursor}))
            .unwrap_err()
            .contains("changed")
    );
    drop(peer_runtime);
    drop(peer);
    drop(runtime);
    drop(store);
    delete(&root, &id).unwrap();
    assert!(!spill.exists());
    assert_eq!(fs::read_to_string(root.join("ordinary")).unwrap(), "keep");
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn failed_save_keeps_previous_snapshot_and_next_completed_turn_retries() {
    let root = project();
    let store = Store::open(&root, None).unwrap();
    let mut history = messages();
    store
        .save(
            &config(),
            &history,
            &Meter::default(),
            &Reader::default(),
            &Skills::default(),
        )
        .unwrap();
    let before = fs::read(store.root.join(SNAPSHOT)).unwrap();
    let moved = store.root.with_extension("temporarily-unavailable");
    fs::rename(&store.root, &moved).unwrap();
    history.push(json!({"role":"user","content":"next"}));
    history.push(json!({"role":"assistant","content":"next answer"}));
    let mut log = crate::output::RunLog::new();
    log.final_output = Some("next answer".into());
    assert!(!save_completed(
        &store,
        &config(),
        &history,
        &Meter::default(),
        &Reader::default(),
        &Skills::default(),
        &log
    ));
    assert_eq!(fs::read(moved.join(SNAPSHOT)).unwrap(), before);
    fs::rename(&moved, &store.root).unwrap();
    history.push(json!({"role":"user","content":"and then"}));
    history.push(json!({"role":"assistant","content":"latest"}));
    assert!(save_completed(
        &store,
        &config(),
        &history,
        &Meter::default(),
        &Reader::default(),
        &Skills::default(),
        &log
    ));
    assert_eq!(read_snapshot(&store.root).unwrap().messages, history);
    log.termination = record::Termination::MaxOutputTokens;
    assert!(!save_completed(
        &store,
        &config(),
        &messages(),
        &Meter::default(),
        &Reader::default(),
        &Skills::default(),
        &log
    ));
    assert_eq!(read_snapshot(&store.root).unwrap().messages, history);
    drop(store);
    fs::remove_dir_all(root).unwrap();
}

// Runs the real CLI entry in a new test process; the shipped executable has no mock URL switch.
#[test]
fn process_entry() {
    let Ok(text) = std::env::var("HEL_TEST_ARGS") else {
        return;
    };
    let args: Vec<String> = serde_json::from_str(&text).unwrap();
    if let Err(error) = crate::parse_args(args.into_iter())
        .map_err(|e| -> Box<dyn Error> { e.into() })
        .and_then(crate::run_args)
    {
        eprintln!("{error}");
        std::process::exit(2);
    }
}

fn child(root: &Path, args: &[&str], url: Option<&str>) -> Command {
    let mut command = Command::new(std::env::current_exe().unwrap());
    command
        .args(["--exact", "sessions::tests::process_entry", "--nocapture"])
        .current_dir(root)
        .env_clear()
        .env("PATH", "/usr/bin:/bin:/usr/sbin:/sbin")
        .env("HEL_TEST_ARGS", serde_json::to_string(args).unwrap())
        .stdin(Stdio::null());
    if let Some(url) = url {
        command
            .env("HEL_TEST_API_URL", url)
            .env("DEEPSEEK_API_KEY", "mock-only");
    }
    command
}

fn successful(output: Output) {
    assert!(
        output.status.success(),
        "stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn read_request(stream: &mut TcpStream) -> Value {
    // Accepted sockets can inherit the listener's nonblocking mode on macOS.
    stream.set_nonblocking(false).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    let mut reader = BufReader::new(stream);
    let mut length = None;
    loop {
        let mut line = String::new();
        reader.read_line(&mut line).unwrap();
        if line == "\r\n" {
            break;
        }
        assert!(!line.is_empty());
        if let Some((key, value)) = line.split_once(':')
            && key.eq_ignore_ascii_case("content-length")
        {
            length = Some(value.trim().parse::<usize>().unwrap());
        }
    }
    let mut body = vec![0; length.unwrap()];
    reader.read_exact(&mut body).unwrap();
    serde_json::from_slice(&body).unwrap()
}

fn mock_server(count: usize) -> (String, thread::JoinHandle<Vec<Value>>) {
    mock_responses((1..=count).map(|i| json!({"role":"assistant","content":format!("answer {i}"),"reasoning_content":"mock reasoning"})).collect())
}

fn mock_responses(responses: Vec<Value>) -> (String, thread::JoinHandle<Vec<Value>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    listener.set_nonblocking(true).unwrap();
    let join = thread::spawn(move || {
        let started = Instant::now();
        let mut requests = Vec::new();
        while requests.len() < responses.len() {
            match listener.accept() {
                Ok((mut stream, _)) => {
                    let request = read_request(&mut stream);
                    let response = json!({"model":request["model"],"choices":[{"finish_reason":"stop","message":responses[requests.len()]}],"usage":{"prompt_tokens":100,"completion_tokens":5,"prompt_cache_hit_tokens":0}}).to_string();
                    write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", response.len(), response).unwrap();
                    requests.push(request);
                }
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                    assert!(
                        started.elapsed() < Duration::from_secs(10),
                        "mock request timeout"
                    );
                    thread::sleep(Duration::from_millis(10));
                }
                Err(e) => panic!("mock accept: {e}"),
            }
        }
        requests
    });
    (url, join)
}

#[test]
fn actual_cli_processes_save_resume_and_refresh_project_rules() {
    let root = project();
    fs::write(root.join("HEL.md"), "OLD_RULE").unwrap();
    let (url, server) = mock_server(2);
    successful(
        child(&root, &["--instruction", "first secret value"], Some(&url))
            .output()
            .unwrap(),
    );
    let store = root.join(".hel/sessions");
    let ids: Vec<_> = fs::read_dir(&store)
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect();
    assert_eq!(ids.len(), 1);
    let id = &ids[0];
    let saved = read_snapshot(&store.join(id)).unwrap();
    assert_eq!(saved.messages.last().unwrap()["content"], "answer 1");
    fs::write(root.join("HEL.md"), "NEW_RULE").unwrap();
    successful(
        child(
            &root,
            &["--resume", id, "--instruction", "followup"],
            Some(&url),
        )
        .output()
        .unwrap(),
    );
    let requests = server.join().unwrap();
    let first = requests[0]["messages"].as_array().unwrap();
    let next = requests[1]["messages"].as_array().unwrap();
    assert_eq!(next.len(), 4);
    assert!(first[0]["content"].as_str().unwrap().contains("OLD_RULE"));
    assert!(next[0]["content"].as_str().unwrap().contains("NEW_RULE"));
    assert!(!next[0]["content"].as_str().unwrap().contains("OLD_RULE"));
    assert_eq!(next[1], first[1]);
    assert_eq!(next[2], saved.messages[2]);
    assert_eq!(next[3]["content"], "followup");
    assert_eq!(read_snapshot(&store.join(id)).unwrap().messages.len(), 5);
    successful(child(&root, &["sessions"], None).output().unwrap());
    successful(
        child(&root, &["sessions", "delete", id], None)
            .output()
            .unwrap(),
    );
    assert!(!store.join(id).exists());
    assert_eq!(fs::read_to_string(root.join("HEL.md")).unwrap(), "NEW_RULE");
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn separate_process_restores_compacted_history_and_tool_connections_in_first_request() {
    let root = project();
    let store = Store::open(&root, None).unwrap();
    let history = vec![
        json!({"role":"user","content":"<compacted-summary>earlier task</compacted-summary>"}),
        json!({"role":"user","content":"inspect"}),
        json!({"role":"assistant","content":null,"reasoning_content":"kept reasoning","tool_calls":[{"id":"call-one","type":"function","function":{"name":"read_file","arguments":"{\"path\":\"x\"}"}}]}),
        json!({"role":"tool","tool_call_id":"call-one","content":"kept result"}),
        json!({"role":"assistant","content":"completed"}),
    ];
    store
        .save(
            &config(),
            &history,
            &Meter::default(),
            &Reader::default(),
            &Skills::default(),
        )
        .unwrap();
    let id = store.id.clone();
    drop(store);
    let (url, server) = mock_server(1);
    successful(
        child(
            &root,
            &[
                "--resume",
                &id,
                "--no-env",
                "--no-context-file",
                "--instruction",
                "continue",
            ],
            Some(&url),
        )
        .output()
        .unwrap(),
    );
    let requests = server.join().unwrap();
    let sent = requests[0]["messages"].as_array().unwrap();
    assert_eq!(&sent[..history.len()], history);
    assert_eq!(sent.last().unwrap()["content"], "continue");
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn occupied_session_refuses_resume_and_delete_in_another_process() {
    let root = project();
    let store = Store::open(&root, None).unwrap();
    store
        .save(
            &config(),
            &messages(),
            &Meter::default(),
            &Reader::default(),
            &Skills::default(),
        )
        .unwrap();
    for args in [
        vec!["--resume", store.id.as_str(), "--instruction", "no call"],
        vec!["sessions", "delete", store.id.as_str()],
    ] {
        let output = child(&root, &args, Some("http://127.0.0.1:1"))
            .output()
            .unwrap();
        assert!(!output.status.success());
        assert!(String::from_utf8_lossy(&output.stderr).contains("already in use"));
    }
    drop(store);
    fs::remove_dir_all(root).unwrap();
}

#[cfg(target_os = "macos")]
#[test]
fn native_tools_cannot_modify_sessions_or_read_peers_and_children_keep_lease() {
    let root = project();
    let store = Store::open(&root, None).unwrap();
    let runtime = store.runtime().unwrap();
    let own = runtime.save_spill(b"own").unwrap();
    store
        .save(
            &config(),
            &messages(),
            &Meter::default(),
            &Reader::default(),
            &Skills::default(),
        )
        .unwrap();
    let peer = Store::open(&root, None).unwrap();
    let other_runtime = peer.runtime().unwrap();
    let other = other_runtime.save_spill(b"peer").unwrap();
    let output = crate::sandbox::command(&runtime, "/bin/sh")
        .unwrap()
        .args([
            "-c",
            r#"
set -eu
cat "$1"
if cat "$2"; then exit 31; fi
if cat "$3"; then exit 32; fi
if printf changed > "$1"; then exit 33; fi
if rm "$3"; then exit 34; fi
printf OK
"#,
            "sh",
        ])
        .arg(&own)
        .arg(&other)
        .arg(store.root.join(SNAPSHOT))
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(output.stdout, b"ownOK");
    fs::write(root.join("visible.txt"), "visible").unwrap();
    let found = crate::search::execute(&runtime, "glob", &json!({"pattern":"**/*"})).unwrap();
    assert!(found.contains("visible.txt"), "{found}");
    assert!(!found.contains(".hel"), "{found}");
    assert!(
        crate::search::execute(&runtime, "grep", &json!({"path":".hel", "pattern":"."})).is_err()
    );
    let id = store.id.clone();
    let mut child = crate::sandbox::command(&runtime, "/bin/sleep")
        .unwrap()
        .arg("0.3")
        .spawn()
        .unwrap();
    drop(runtime);
    drop(store);
    assert!(
        delete(&root, &id)
            .unwrap_err()
            .to_string()
            .contains("in use")
    );
    child.wait().unwrap();
    delete(&root, &id).unwrap();
    drop(other_runtime);
    drop(peer);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn resumed_process_uses_the_same_cursor_to_read_session_spill() {
    let root = project();
    let store = Store::open(&root, None).unwrap();
    let runtime = store.runtime().unwrap();
    let text = format!("{}CURSOR_TAIL", "x".repeat(12_000));
    let spill = runtime.save_spill(text.as_bytes()).unwrap();
    let first = runtime
        .reader
        .read(&runtime, &json!({"path":spill}))
        .unwrap();
    let cursor = first
        .split("\"cursor\":\"")
        .nth(1)
        .unwrap()
        .split('"')
        .next()
        .unwrap()
        .to_string();
    store
        .save(
            &config(),
            &messages(),
            &Meter::default(),
            &runtime.reader,
            &runtime.skills.borrow(),
        )
        .unwrap();
    let id = store.id.clone();
    drop(runtime);
    drop(store);
    let (url, server) = mock_responses(vec![
        json!({"role":"assistant","content":null,"tool_calls":[{"id":"cursor-read","type":"function","function":{"name":"read_file","arguments":json!({"cursor":cursor}).to_string()}}]}),
        json!({"role":"assistant","content":"read finished"}),
    ]);
    successful(
        child(
            &root,
            &[
                "--resume",
                &id,
                "--tools",
                "read_file",
                "--no-env",
                "--no-context-file",
                "--no-compaction",
                "--instruction",
                "continue the read",
            ],
            Some(&url),
        )
        .output()
        .unwrap(),
    );
    let requests = server.join().unwrap();
    let tool_result = requests[1]["messages"].as_array().unwrap().last().unwrap();
    assert_eq!(tool_result["role"], "tool");
    assert_eq!(tool_result["tool_call_id"], "cursor-read");
    assert_eq!(
        format!(
            "{}{}",
            first.split("\n\n[Read truncated.").next().unwrap(),
            tool_result["content"].as_str().unwrap()
        ),
        text
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn simultaneous_new_sessions_are_independent_and_git_ignores_the_store() {
    let root = project();
    successful(
        Command::new("git")
            .args(["init", "--quiet"])
            .current_dir(&root)
            .output()
            .unwrap(),
    );
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
    let handles: Vec<_> = (0..2)
        .map(|_| {
            let root = root.clone();
            let barrier = barrier.clone();
            thread::spawn(move || {
                barrier.wait();
                Store::open(&root, None).unwrap()
            })
        })
        .collect();
    let stores: Vec<_> = handles.into_iter().map(|h| h.join().unwrap()).collect();
    assert_ne!(stores[0].id, stores[1].id);
    for store in &stores {
        store
            .save(
                &config(),
                &messages(),
                &Meter::default(),
                &Reader::default(),
                &Skills::default(),
            )
            .unwrap();
        successful(
            Command::new("git")
                .args(["check-ignore", "--quiet"])
                .arg(store.root.join(SNAPSHOT))
                .current_dir(&root)
                .output()
                .unwrap(),
        );
    }
    drop(stores);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn actual_cli_skill_snapshot_is_lazy_and_reread_updates_metadata_and_body() {
    let root = project();
    fs::create_dir_all(root.join(".agents/skills/review")).unwrap();
    let path = ".agents/skills/review/SKILL.md";
    fs::write(
        root.join(path),
        "---\nname: review\ndescription: Old description\n---\nRULE_V1",
    )
    .unwrap();
    let call = |id: &str| json!({"role":"assistant","content":null,"tool_calls":[{"id":id,"type":"function","function":{"name":"read_file","arguments":json!({"path":path}).to_string()}}]});
    let (url, server) = mock_responses(vec![
        call("initial"),
        call("duplicate"),
        json!({"role":"assistant","content":"first done"}),
        call("changed"),
        json!({"role":"assistant","content":"resumed done"}),
        call("same"),
        json!({"role":"assistant","content":"third done"}),
    ]);
    successful(
        child(
            &root,
            &["--tools", "read_file", "--instruction", "review"],
            Some(&url),
        )
        .output()
        .unwrap(),
    );
    let dir = fs::read_dir(root.join(".hel/sessions"))
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    let id = dir.file_name().unwrap().to_str().unwrap();
    fs::write(
        root.join(path),
        "---\nname: code-review\ndescription: New description\n---\nRULE_V2",
    )
    .unwrap();
    for _ in 0..2 {
        successful(
            child(
                &root,
                &[
                    "--tools",
                    "read_file",
                    "--resume",
                    id,
                    "--instruction",
                    "continue",
                ],
                Some(&url),
            )
            .output()
            .unwrap(),
        );
    }
    let requests = server.join().unwrap();
    let system = |i: usize| requests[i]["messages"][0]["content"].as_str().unwrap();
    assert!(system(0).contains("Old description") && !system(0).contains("RULE_V1"));
    assert!(requests[1]["messages"].to_string().contains("RULE_V1"));
    assert!(
        requests[2]["messages"].as_array().unwrap().last().unwrap()["content"]
            .as_str()
            .unwrap()
            .contains("already loaded")
    );
    assert!(
        system(3).contains("Old description"),
        "resume must not reread the changed file"
    );
    assert!(!system(3).contains("New description"));
    assert!(system(4).contains("New description") && system(4).contains("code-review"));
    assert!(
        requests[4]["messages"].as_array().unwrap().last().unwrap()["content"]
            .as_str()
            .unwrap()
            .contains("RULE_V2")
    );
    assert!(
        requests[6]["messages"].as_array().unwrap().last().unwrap()["content"]
            .as_str()
            .unwrap()
            .contains("already loaded")
    );
    let state = read_snapshot(&dir).unwrap();
    state.skills.validate().unwrap();
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn h9_snapshot_without_skills_still_restores() {
    let root = project();
    let store = Store::open(&root, None).unwrap();
    store
        .save(
            &config(),
            &messages(),
            &Meter::default(),
            &Reader::default(),
            &Skills::default(),
        )
        .unwrap();
    let path = store.root.join(SNAPSHOT);
    let mut snapshot: Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    snapshot.as_object_mut().unwrap().remove("skills");
    fs::write(&path, snapshot.to_string()).unwrap();
    let state = store
        .restore(Some(messages()[0].clone()), &config())
        .unwrap();
    assert_eq!(state.messages, messages());
    assert!(!state.skills.initialized);
    drop(store);
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn actual_cli_hook_rejection_reaches_model_then_read_unlocks_edit() {
    use sha2::{Digest, Sha256};
    let root = project().canonicalize().unwrap();
    fs::create_dir_all(root.join(".hel")).unwrap();
    fs::create_dir(root.join("hooks")).unwrap();
    let config =
        include_bytes!("../../../evals/tasks/hook-read-before-edit-01/fixture/.hel/hooks.json");
    fs::write(root.join(".hel/hooks.json"), config).unwrap();
    fs::write(
        root.join("hooks/read_guard.py"),
        include_str!("../../../evals/tasks/hook-read-before-edit-01/fixture/hooks/read_guard.py"),
    )
    .unwrap();
    fs::write(root.join("config.ini"), "port=7000\n").unwrap();
    // Register exactly the reviewed fixture through the same persisted trust contract as the CLI.
    fs::write(
        root.join(".hel/hooks-trust.json"),
        json!({
            "schema_version":1,"project":root,"sha256":format!("{:x}",Sha256::digest(config))
        })
        .to_string(),
    )
    .unwrap();
    let tool = |id: &str, name: &str, args: Value| {
        json!({
            "role":"assistant","content":null,"tool_calls":[{
                "id":id,"type":"function","function":{"name":name,"arguments":args.to_string()}
            }]
        })
    };
    let edit = json!({"path":"config.ini","search":"port=7000","replace":"port=8000"});
    let (url, server) = mock_responses(vec![
        tool("edit-first", "search_replace", edit.clone()),
        tool(
            "read-after-feedback",
            "read_file",
            json!({"path":"config.ini"}),
        ),
        tool("edit-retry", "search_replace", edit),
        json!({"role":"assistant","content":"done"}),
    ]);
    successful(
        child(
            &root,
            &[
                "--instruction",
                "edit first",
                "--tools",
                "read_file,search_replace",
                "--access",
                "auto",
                "--no-compaction",
            ],
            Some(&url),
        )
        .output()
        .unwrap(),
    );
    let requests = server.join().unwrap();
    assert_eq!(requests.len(), 4);
    let second = requests[1]["messages"].as_array().unwrap().last().unwrap();
    assert_eq!(second["tool_call_id"], "edit-first");
    assert!(
        second["content"]
            .as_str()
            .unwrap()
            .contains("먼저 read_file")
    );
    let third = requests[2]["messages"].as_array().unwrap().last().unwrap();
    assert_eq!(third["content"], "port=7000\n");
    let fourth = requests[3]["messages"].as_array().unwrap().last().unwrap();
    assert_eq!(fourth["content"], "replaced 1 occurrence in config.ini");
    assert_eq!(
        fs::read_to_string(root.join("config.ini")).unwrap(),
        "port=8000\n"
    );
    let events: Vec<Value> = fs::read_to_string(root.join(".hel/read-guard-events.jsonl"))
        .unwrap()
        .lines()
        .map(|s| serde_json::from_str(s).unwrap())
        .collect();
    assert_eq!(
        events
            .iter()
            .map(|v| v["result"].as_str().unwrap())
            .collect::<Vec<_>>(),
        ["blocked", "read-recorded", "allowed"]
    );
    fs::remove_dir_all(root).unwrap();
}

#[test]
fn actual_cli_skips_untrusted_hooks_without_interactive_input() {
    let root = project();
    fs::create_dir_all(root.join(".hel")).unwrap();
    fs::write(
        root.join(".hel/hooks.json"),
        json!({"hooks":{"PreToolUse":[{"hooks":[{"type":"command","command":"touch hook-ran"}]}]}})
            .to_string(),
    )
    .unwrap();
    fs::write(root.join("a"), "data").unwrap();
    let (url, server) = mock_responses(vec![
        json!({"role":"assistant","content":null,"tool_calls":[{"id":"r","type":"function","function":{"name":"read_file","arguments":"{\"path\":\"a\"}"}}]}),
        json!({"role":"assistant","content":"done"}),
    ]);
    let output = child(
        &root,
        &["--instruction", "read a", "--tools", "read_file"],
        Some(&url),
    )
    .output()
    .unwrap();
    assert!(String::from_utf8_lossy(&output.stderr).contains("hooks skipped"));
    successful(output);
    let requests = server.join().unwrap();
    assert_eq!(
        requests[1]["messages"].as_array().unwrap().last().unwrap()["content"],
        "data"
    );
    assert!(!root.join("hook-ran").exists());
    assert!(!root.join(".hel/hooks-trust.json").exists());
    fs::remove_dir_all(root).unwrap();
}
