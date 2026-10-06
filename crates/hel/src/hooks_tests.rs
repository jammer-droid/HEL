use super::*;
use crate::permissions::{Access, Approval, Input, Response};
use crate::tools::Toolset;
use std::os::unix::fs::symlink;

struct Project {
    root: PathBuf,
    runtime: Runtime,
}
impl Project {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!("hel-hooks-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&root).unwrap();
        let root = root.canonicalize().unwrap();
        let runtime = Runtime::new(&root).unwrap();
        fs::create_dir(root.join(".hel")).unwrap();
        Self { root, runtime }
    }
    fn config(&self, value: Value) {
        fs::write(
            self.root.join(".hel/hooks.json"),
            serde_json::to_vec(&value).unwrap(),
        )
        .unwrap();
    }
    fn activate(&mut self, value: Value) {
        self.config(value);
        self.runtime.hooks = Hooks::load_with_review(&self.root, "session-test", |_| true);
        assert!(!self.runtime.hooks.handlers.is_empty());
    }
    fn call(&self, tools: &Toolset, name: &str, args: Value) -> crate::permissions::Execution {
        tools.call_with_id(
            &self.runtime,
            "call-test",
            name,
            &args,
            Access::Auto,
            &mut Input::Unavailable,
        )
    }
    fn text(&self, name: &str) -> String {
        fs::read_to_string(self.root.join(name)).unwrap()
    }
}
impl Drop for Project {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}
fn command(text: &str) -> Value {
    json!({"type":"command", "command":text, "timeout":10})
}
fn group(matcher: &str, handlers: Vec<Value>) -> Value {
    json!({"matcher":matcher, "hooks":handlers})
}
fn print_json(value: Value) -> String {
    format!("printf '%s' '{}'", value.to_string().replace('\'', "'\\''"))
}
fn deny(reason: &str) -> Value {
    json!({"hookSpecificOutput":{"hookEventName":"PreToolUse","permissionDecision":"deny","permissionDecisionReason":reason}})
}

#[test]
fn trust_matches_exact_definition_and_project_and_keeps_reviewed_snapshot() {
    let mut p = Project::new();
    let config =
        json!({"hooks":{"PreToolUse":[group("read_file",vec![command("printf old > marker")])]}});
    p.config(config.clone());
    let denied = Hooks::load_with_review(&p.root, "s", |_| false);
    assert!(denied.handlers.is_empty());
    assert!(!p.root.join(".hel/hooks-trust.json").exists());
    let mut reviews = 0;
    p.runtime.hooks = Hooks::load_with_review(&p.root, "s", |shown| {
        reviews += 1;
        assert!(shown.contains("printf old > marker"));
        // Changing the on-disk definition during the review cannot change this invocation.
        p.config(json!({"hooks":{"PreToolUse":[group("read_file",vec![command("printf new > marker")])]}}));
        true
    });
    assert_eq!(reviews, 1);
    let tools = Toolset::new(&["read_file"]).unwrap();
    fs::write(p.root.join("a"), "data").unwrap();
    p.call(&tools, "read_file", json!({"path":"a"}))
        .result
        .unwrap();
    assert_eq!(p.text("marker"), "old");
    let changed = Hooks::load_with_review(&p.root, "s", |_| false);
    assert!(changed.handlers.is_empty());
    p.config(config);
    let trusted = Hooks::load_with_review(&p.root, "s", |_| {
        panic!("same approved bytes prompted again")
    });
    assert_eq!(trusted.handlers.len(), 1);
    let other = Project::new();
    fs::copy(
        p.root.join(".hel/hooks.json"),
        other.root.join(".hel/hooks.json"),
    )
    .unwrap();
    fs::copy(
        p.root.join(".hel/hooks-trust.json"),
        other.root.join(".hel/hooks-trust.json"),
    )
    .unwrap();
    assert!(
        Hooks::load_with_review(&other.root, "s", |_| false)
            .handlers
            .is_empty()
    );
}

#[test]
fn trusted_command_can_change_script_body_without_retrusting() {
    let mut p = Project::new();
    fs::write(p.root.join("script.sh"), "echo one > marker\n").unwrap();
    p.activate(
        json!({"hooks":{"PreToolUse":[group("read_file",vec![command("/bin/sh script.sh")])]}}),
    );
    fs::write(p.root.join("script.sh"), "echo two > marker\n").unwrap();
    p.runtime.hooks = Hooks::load_with_review(&p.root, "s", |_| {
        panic!("script body is not a trusted config field")
    });
    p.call(
        &Toolset::new(&["read_file"]).unwrap(),
        "read_file",
        json!({"path":"missing"}),
    );
    assert_eq!(p.text("marker"), "two\n");
}

#[test]
fn invalid_or_symlinked_configuration_never_runs_or_requests_trust() {
    let p = Project::new();
    for config in [
        json!({"hooks":{"PreToolUse":[group("[",vec![command("true")])]}}),
        json!({"hooks":{"Stop":[]}}),
        json!({"hooks":{"PreToolUse":[group("*",vec![json!({"type":"prompt","command":"true"})])]}}),
        json!({"hooks":{"PreToolUse":[group("*",vec![json!({"type":"command","command":"true","timeout":0})])]}}),
    ] {
        p.config(config);
        assert!(
            Hooks::load_with_review(&p.root, "s", |_| panic!("invalid config was reviewed"))
                .handlers
                .is_empty()
        );
    }
    fs::remove_file(p.root.join(".hel/hooks.json")).unwrap();
    fs::write(p.root.join("outside.json"), "{\"hooks\":{}}").unwrap();
    symlink(p.root.join("outside.json"), p.root.join(".hel/hooks.json")).unwrap();
    assert!(
        Hooks::load_with_review(&p.root, "s", |_| panic!("symlink config was reviewed"))
            .handlers
            .is_empty()
    );
}

#[test]
fn missing_hooks_leave_tool_contract_unchanged() {
    let mut p = Project::new();
    p.runtime.hooks = Hooks::load_with_review(&p.root, "s", |_| panic!("no config"));
    fs::write(p.root.join("a"), "old").unwrap();
    let out = p.call(
        &Toolset::new(&["search_replace"]).unwrap(),
        "search_replace",
        json!({"path":"a","search":"old","replace":"new"}),
    );
    assert!(out.result.is_ok());
    assert!(out.trace.executed);
    assert!(!out.trace.hook_blocked);
    assert_eq!(p.text("a"), "new");
}

#[test]
fn read_guard_fixture_blocks_then_records_a_real_read_then_allows_edit() {
    let mut p = Project::new();
    fs::create_dir(p.root.join("hooks")).unwrap();
    fs::write(
        p.root.join("hooks/read_guard.py"),
        include_str!("../../../evals/tasks/hook-read-before-edit-01/fixture/hooks/read_guard.py"),
    )
    .unwrap();
    let config: Value = serde_json::from_str(include_str!(
        "../../../evals/tasks/hook-read-before-edit-01/fixture/.hel/hooks.json"
    ))
    .unwrap();
    p.activate(config);
    fs::write(p.root.join("config.ini"), "port=7000\n").unwrap();
    let tools = Toolset::new(&["read_file", "search_replace"]).unwrap();
    let edit = json!({"path":"config.ini","search":"port=7000","replace":"port=8000"});
    let first = p.call(&tools, "search_replace", edit.clone());
    assert!(first.result.unwrap_err().contains("먼저 read_file"));
    assert!(first.trace.hook_blocked);
    assert!(!first.trace.executed);
    assert_eq!(p.text("config.ini"), "port=7000\n");
    assert!(!p.root.join(".hel/read-guard-state.json").exists());
    p.call(&tools, "read_file", json!({"path":"config.ini"}))
        .result
        .unwrap();
    p.call(&tools, "search_replace", edit).result.unwrap();
    assert_eq!(p.text("config.ini"), "port=8000\n");
    assert!(
        p.runtime
            .read_open(Path::new(".hel/read-guard-state.json"))
            .is_err()
    );
    let events: Vec<Value> = p
        .text(".hel/read-guard-events.jsonl")
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    assert_eq!(
        events
            .iter()
            .map(|v| v["result"].as_str().unwrap())
            .collect::<Vec<_>>(),
        ["blocked", "read-recorded", "allowed"]
    );
}

#[test]
fn duplicate_matching_handlers_run_in_order_and_deny_skips_rest_and_post() {
    let mut p = Project::new();
    p.activate(json!({"hooks":{
        "PreToolUse":[
            group("^read_file$",vec![command("echo wrong >> order")]),
            group("^write_file$",vec![command("printf A >> order"),command("printf A >> order")]),
            group("*",vec![command("printf B >> order; echo error >&2; exit 1"),command("printf malformed"),command(&print_json(deny("no edit"))),command("printf C >> order")]),
            group("write_file",vec![command("printf D >> order")])
        ],
        "PostToolUse":[group("*",vec![command("printf POST >> order")])]
    }}));
    let out = p.call(
        &Toolset::new(&["write_file"]).unwrap(),
        "write_file",
        json!({"path":"a","content":"new"}),
    );
    assert!(out.result.unwrap_err().contains("no edit"));
    assert!(!p.root.join("a").exists());
    assert_eq!(p.text("order"), "AAB");
    let trace = p.runtime.hooks.trace.borrow();
    let status: Vec<_> = trace
        .iter()
        .filter(|v| v["event"] == "PreToolUse")
        .map(|v| v["status"].as_str().unwrap())
        .collect();
    assert_eq!(
        status,
        ["continued", "continued", "error", "error", "blocked"]
    );
    assert!(!trace.iter().any(|v| v["event"] == "PostToolUse"));
}

#[test]
fn pre_runs_before_permission_and_post_only_after_success() {
    struct CheckApproval<'a> {
        root: &'a Path,
        calls: usize,
    }
    impl Approval for CheckApproval<'_> {
        fn request(&mut self, _: &str, _: &Value) -> Response {
            self.calls += 1;
            assert_eq!(fs::read_to_string(self.root.join("order")).unwrap(), "pre");
            assert!(!self.root.join("a").exists());
            Response::Approved
        }
    }
    let mut p = Project::new();
    p.activate(json!({"hooks":{
        "PreToolUse":[group("*",vec![command("printf pre > order")])],
        "PostToolUse":[group("*",vec![command("test -f a && printf post >> order")])]
    }}));
    let tools = Toolset::new(&["write_file", "search_replace"]).unwrap();
    let mut approval = CheckApproval {
        root: &p.root,
        calls: 0,
    };
    let out = tools.call_with_id(
        &p.runtime,
        "c",
        "write_file",
        &json!({"path":"a","content":"x"}),
        Access::Confirm,
        &mut approval,
    );
    assert!(out.result.is_ok());
    assert_eq!(approval.calls, 1);
    assert_eq!(p.text("order"), "prepost");
    let out = tools.call(
        &p.runtime,
        "write_file",
        &json!({"path":"a","content":"bad"}),
        Access::ReadOnly,
        &mut Input::Unavailable,
    );
    assert!(out.result.is_err());
    assert!(!out.trace.executed);
    assert_eq!(p.text("order"), "pre");
    assert_eq!(p.text("a"), "x");
    let out = p.call(
        &tools,
        "search_replace",
        json!({"path":"a","search":"absent","replace":"y"}),
    );
    assert!(out.result.is_err());
    assert!(out.trace.executed);
    assert_eq!(p.text("order"), "pre");
    fs::remove_file(p.root.join("order")).unwrap();
    assert!(p.call(&tools, "unknown", json!({})).result.is_err());
    assert!(!p.root.join("order").exists());
}

#[test]
fn post_handlers_share_original_input_and_feedback_replaces_output_in_order() {
    let mut p = Project::new();
    fs::write(
        p.root.join("inspect.py"),
        r#"import json,sys
from pathlib import Path
x=json.load(sys.stdin)
with Path('inputs.jsonl').open('a') as f:f.write(json.dumps(x)+'\n')
print(json.dumps({'decision':'block','reason':sys.argv[1]}))
"#,
    )
    .unwrap();
    p.activate(json!({"hooks":{"PostToolUse":[group("read_file",vec![command("python3 inspect.py first"),command("exit 1"),command("true"),command("python3 inspect.py second")])]}}));
    fs::write(p.root.join("a"), "original contents").unwrap();
    let out = p.call(
        &Toolset::new(&["read_file"]).unwrap(),
        "read_file",
        json!({"path":"a"}),
    );
    assert!(out.trace.executed);
    let feedback = out.result.unwrap();
    assert!(feedback.find("first").unwrap() < feedback.find("second").unwrap());
    assert!(!feedback.contains("original contents"));
    let inputs: Vec<Value> = p
        .text("inputs.jsonl")
        .lines()
        .map(|v| serde_json::from_str(v).unwrap())
        .collect();
    assert_eq!(inputs.len(), 2);
    assert_eq!(inputs[0], inputs[1]);
    assert_eq!(inputs[0]["tool_response"], "original contents");
    assert_eq!(inputs[0]["tool_use_id"], "call-test");
    assert_eq!(inputs[0]["session_id"], "session-test");
    assert_eq!(inputs[0]["cwd"], json!(p.root));
    assert!(inputs[0].get("tool_success").is_none());
}

#[test]
fn post_exit_two_replaces_result_and_tool_error_has_no_post() {
    let mut p = Project::new();
    p.activate(json!({"hooks":{"PostToolUse":[group("read_file",vec![command("printf feedback >&2; echo ran >> marker; exit 2")])]}}));
    let tools = Toolset::new(&["read_file"]).unwrap();
    assert!(
        p.call(&tools, "read_file", json!({"path":"missing"}))
            .result
            .is_err()
    );
    assert!(!p.root.join("marker").exists());
    fs::write(p.root.join("a"), "data").unwrap();
    assert!(
        p.call(&tools, "read_file", json!({"path":"a"}))
            .result
            .unwrap()
            .contains("feedback")
    );
    assert_eq!(p.text("marker"), "ran\n");
}

#[test]
fn hook_process_can_access_outside_project_but_tool_remains_restricted() {
    let outer = std::env::temp_dir().join(format!("hel-hook-outside-{}", uuid::Uuid::new_v4()));
    fs::write(&outer, "external").unwrap();
    let mut p = Project::new();
    let cmd = format!("cat '{}' > marker", outer.display());
    p.activate(json!({"hooks":{"PreToolUse":[group("read_file",vec![command(&cmd)])]}}));
    let out = p.call(
        &Toolset::new(&["read_file"]).unwrap(),
        "read_file",
        json!({"path":outer}),
    );
    assert_eq!(p.text("marker"), "external");
    assert!(out.result.is_err());
    fs::remove_file(outer).unwrap();
}

fn handler(command: &str, timeout: Duration) -> Handler {
    Handler {
        event: Event::Pre,
        matcher: None,
        command: command.into(),
        timeout,
        index: 0,
    }
}

#[test]
fn timeout_handles_unread_stdin_and_inherited_output_pipes() {
    let p = Project::new();
    let started = Instant::now();
    let result = run_command(
        &p.runtime,
        &handler("sleep 5", Duration::from_millis(40)),
        &vec![b'x'; MAX_BYTES * 2],
    );
    assert!(result.err().unwrap().contains("timed out"));
    assert!(started.elapsed() < Duration::from_secs(2));
    // Shell exits first; the child keeps the pipes open. Timeout must still cover that child.
    let result = run_command(
        &p.runtime,
        &handler(
            "(sleep 2; touch survived) & exit 0",
            Duration::from_millis(40),
        ),
        b"{}",
    );
    assert!(result.err().unwrap().contains("timed out"));
    std::thread::sleep(Duration::from_millis(2100));
    assert!(!p.root.join("survived").exists());
}

#[test]
fn oversized_stdout_or_stderr_is_bounded_and_general_error_continues() {
    let mut p = Project::new();
    for stream in ["stdout", "stderr"] {
        let cmd = format!("python3 -c 'import sys; sys.{stream}.write(\"x\"*1100000)'");
        let error = run_command(&p.runtime, &handler(&cmd, Duration::from_secs(5)), b"{}")
            .err()
            .unwrap();
        assert!(error.contains("capture limit"), "{error}");
    }
    p.activate(json!({"hooks":{"PreToolUse":[group("read_file",vec![command("echo bad >&2; exit 1"),command("echo reached > marker")])]}}));
    fs::write(p.root.join("a"), "data").unwrap();
    assert_eq!(
        p.call(
            &Toolset::new(&["read_file"]).unwrap(),
            "read_file",
            json!({"path":"a"})
        )
        .result
        .unwrap(),
        "data"
    );
    assert_eq!(p.text("marker"), "reached\n");
}

#[test]
fn feedback_spills_full_text_and_stays_bounded_even_on_save_failure() {
    let p = Project::new();
    let full = "한국어\n\t\\\"".repeat(8000);
    let shown = model_feedback(&p.runtime, &full);
    assert!(crate::context::estimate(&[json!(shown)]) <= FEEDBACK_TOKENS);
    assert!(shown.contains("Full feedback:"));
    let saved = fs::read_dir(&p.runtime.spill)
        .unwrap()
        .next()
        .unwrap()
        .unwrap()
        .path();
    assert_eq!(fs::read_to_string(saved).unwrap(), full);
    fs::remove_dir_all(&p.runtime.spill).unwrap();
    let shown = model_feedback(&p.runtime, &full);
    assert!(crate::context::estimate(&[json!(shown)]) <= FEEDBACK_TOKENS);
    assert!(shown.contains("could not save"));
}

#[test]
fn default_timeout_and_omitted_matcher_work_without_name_deduplication() {
    let cfg:Config=serde_json::from_value(json!({"hooks":{"PreToolUse":[{"hooks":[{"type":"command","command":"true"},{"type":"command","command":"true"}]}]}})).unwrap();
    let handlers = compile(cfg).unwrap();
    assert_eq!(handlers.len(), 2);
    assert_eq!(handlers[0].timeout, Duration::from_secs(DEFAULT_TIMEOUT));
    assert!(handlers[0].matches(Event::Pre, "any-tool"));
    assert!(!handlers[0].matches(Event::Post, "any-tool"));
}
