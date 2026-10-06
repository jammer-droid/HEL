use super::*;
use crate::permissions::{Access, Input};
use crate::tools::{READ_FILE, Toolset};
use std::path::PathBuf;

struct Project {
    root: PathBuf,
    runtime: Runtime,
}
impl Project {
    fn new() -> Self {
        let root = std::env::temp_dir().join(format!("hel-skills-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&root).unwrap();
        let root = root.canonicalize().unwrap();
        let runtime = Runtime::new(&root).unwrap();
        Self { root, runtime }
    }
    fn write(&self, dir: &str, name: &str, body: &str) -> String {
        let path = format!("{ROOT}/{dir}/SKILL.md");
        fs::create_dir_all(self.root.join(ROOT).join(dir)).unwrap();
        fs::write(
            self.root.join(&path),
            format!("---\nname: {name}\ndescription: Read project rules\n---\n{body}"),
        )
        .unwrap();
        path
    }
    fn start(&self) -> Vec<Value> {
        let mut skills = self.runtime.skills.borrow_mut();
        skills.configure(
            Some(json!({"role":"system", "content":"environment"})),
            true,
        );
        skills.discover(&self.runtime);
        skills.system_message().into_iter().collect()
    }
    fn read(&self, messages: &mut Vec<Value>, id: &str, args: Value) -> String {
        messages.push(json!({"role":"assistant","tool_calls":[{"id":id,"type":"function","function":{"name":READ_FILE,"arguments":args.to_string()}}]}));
        self.runtime.skills.borrow_mut().reconcile(messages);
        let result = Toolset::new(&[READ_FILE])
            .unwrap()
            .call(
                &self.runtime,
                READ_FILE,
                &args,
                Access::ReadOnly,
                &mut Input::Unavailable,
            )
            .result;
        self.runtime
            .skills
            .borrow()
            .sync_system(messages, &mut context::Meter::default());
        let content = result.unwrap_or_else(|e| format!("error: {e}"));
        self.runtime.skills.borrow_mut().delivered(id, &content);
        messages.push(json!({"role":"tool","tool_call_id":id,"content":content}));
        content
    }
}
impl Drop for Project {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

#[test]
fn catalog_load_dedup_metadata_and_body_updates_use_the_same_path() {
    let p = Project::new();
    let path = p.write("review", "review", "rule v1");
    let mut messages = p.start();
    assert!(messages[0]["content"].as_str().unwrap().contains("review"));
    assert!(!messages[0]["content"].as_str().unwrap().contains("rule v1"));
    assert!(
        p.read(&mut messages, "a", json!({"path":path}))
            .contains("rule v1")
    );
    let system = messages[0].clone();
    assert!(
        p.read(&mut messages, "b", json!({"path":path}))
            .contains("already loaded")
    );
    assert_eq!(messages[0], system);
    assert_eq!(p.runtime.skills.borrow().loaded[&path].last_call, 2);
    p.write("review", "code-review", "rule v1");
    assert!(
        p.read(&mut messages, "c", json!({"path":path}))
            .contains("already loaded")
    );
    assert!(
        messages[0]["content"]
            .as_str()
            .unwrap()
            .contains("code-review")
    );
    p.write("review", "code-review", "rule v2");
    assert!(
        p.read(&mut messages, "d", json!({"path":path}))
            .contains("rule v2")
    );
    assert_eq!(p.runtime.skills.borrow().loaded[&path].body, "rule v2");
}

#[test]
fn renamed_directory_returns_an_error_and_refreshes_only_the_project_catalog() {
    let p = Project::new();
    let old = p.write("review", "review", "old rules");
    let mut messages = p.start();
    p.read(&mut messages, "a", json!({"path":old}));
    fs::rename(
        p.root.join(ROOT).join("review"),
        p.root.join(ROOT).join("code-review"),
    )
    .unwrap();
    let new = p.write("code-review", "code-review", "new rules");
    let result = p.read(&mut messages, "b", json!({"path":old}));
    assert!(result.starts_with("error:") && result.contains("refreshed"));
    let system = messages[0]["content"].as_str().unwrap();
    assert!(!system.contains(&format!("\"path\": \"{old}\"")));
    assert!(system.contains(&new));
    assert!(!p.runtime.skills.borrow().loaded.contains_key(&old));
    assert!(
        p.read(&mut messages, "c", json!({"path":new}))
            .contains("new rules")
    );
}

#[test]
fn long_skill_keeps_cursor_limits_and_only_deduplicates_complete_visible_body() {
    let p = Project::new();
    let body = "instructions🙂\n".repeat(2500);
    let path = p.write("long", "long", &body);
    let mut messages = p.start();
    let first = p.read(&mut messages, "first", json!({"path":path}));
    assert!(first.len() <= crate::read_file::MAX_BYTES);
    assert!(!first.contains("already loaded"));
    let mut result = first;
    let mut i = 0;
    while let Some((_, notice)) = result.split_once("\"cursor\":\"") {
        let cursor = notice.split('"').next().unwrap().to_string();
        i += 1;
        result = p.read(
            &mut messages,
            &format!("chunk-{i}"),
            json!({"cursor":cursor}),
        );
        assert!(result.len() <= crate::read_file::MAX_BYTES);
        assert!(i < 20);
    }
    assert!(
        p.read(&mut messages, "again", json!({"path":path}))
            .contains("already loaded")
    );
    context::prune_tool_results(&mut messages);
    // Partial ranges also cannot masquerade as a complete read.
    messages.retain(|m| m["tool_call_id"] != "first");
    assert!(
        !p.read(
            &mut messages,
            "missing",
            json!({"path":path, "max_lines":1})
        )
        .contains("already loaded")
    );
    assert!(
        !p.read(&mut messages, "retry", json!({"path":path}))
            .contains("already loaded")
    );
}

#[test]
fn preservation_orders_recent_calls_caps_bodies_and_keeps_full_originals() {
    let p = Project::new();
    let body = "x".repeat(24_000);
    for i in 0..7 {
        p.write(&format!("s{i}"), &format!("skill-{i}"), &body);
    }
    let mut messages = p.start();
    for i in 0..7 {
        p.read(
            &mut messages,
            &format!("call-{i}"),
            json!({"path":format!("{ROOT}/s{i}/SKILL.md")}),
        );
    }
    let mut skills = p.runtime.skills.borrow_mut();
    let region = skills.preservation(TOTAL_TOKENS).unwrap();
    assert!(context::estimate(std::slice::from_ref(&region.message)) <= TOTAL_TOKENS);
    let text = region.message["content"].as_str().unwrap();
    assert!(text.find("skill-6").unwrap() < text.find("skill-5").unwrap());
    assert!(!text.contains("skill-0"));
    assert!(!text.contains(&"x".repeat(20_001)));
    assert_eq!(
        skills.loaded[&format!("{ROOT}/s6/SKILL.md")].body.len(),
        24_000
    );
    messages = vec![messages[0].clone(), region.message.clone()];
    skills.set_preserved(Some(region));
    skills.reconcile(&messages);
    assert!(skills.visible.is_empty(), "only prefixes survived");
    drop(skills);
    assert!(
        !p.read(
            &mut messages,
            "reload",
            json!({"path":format!("{ROOT}/s6/SKILL.md")})
        )
        .contains("already loaded")
    );
}

#[test]
fn complete_retained_body_deduplicates_but_changed_file_reloads_after_compaction() {
    let p = Project::new();
    let path = p.write("review", "review", "before");
    let mut messages = p.start();
    p.read(&mut messages, "a", json!({"path":path}));
    let region = p
        .runtime
        .skills
        .borrow()
        .preservation(TOTAL_TOKENS)
        .unwrap();
    messages = vec![
        messages[0].clone(),
        json!({"role":"user","content":"summary"}),
        region.message.clone(),
    ];
    p.runtime.skills.borrow_mut().set_preserved(Some(region));
    assert!(
        p.read(&mut messages, "b", json!({"path":path}))
            .contains("already loaded")
    );
    p.write("review", "review", "after");
    assert!(
        p.read(&mut messages, "c", json!({"path":path}))
            .contains("after")
    );
}

#[test]
fn malformed_and_outside_skills_are_not_discovered() {
    let p = Project::new();
    p.write("valid", "valid", "ok");
    let bad = p.write("bad", "bad", "bad");
    fs::write(p.root.join(&bad), "not frontmatter").unwrap();
    let outside = p.root.join("outside");
    fs::create_dir(&outside).unwrap();
    fs::write(
        outside.join("SKILL.md"),
        "---\nname: outside\ndescription: outside\n---\nsecret",
    )
    .unwrap();
    std::os::unix::fs::symlink(&outside, p.root.join(ROOT).join("link")).unwrap();
    let messages = p.start();
    let text = messages[0]["content"].as_str().unwrap();
    assert!(text.contains("valid"));
    assert!(!text.contains("outside") && !text.contains("bad/SKILL"));
}

fn summary_reply(text: &str) -> crate::api::Exchange {
    let response_json = json!({"model":"test", "choices":[{"message":{"role":"assistant","content":text},"finish_reason":"stop"}]});
    crate::api::Exchange {
        request: json!({}),
        response: serde_json::from_value(response_json.clone()).unwrap(),
        response_json,
    }
}

#[test]
fn compaction_rebuilds_one_region_before_recent_history_within_remaining_budget() {
    let p = Project::new();
    let path = p.write("review", "review", &"rule\n".repeat(1000));
    let mut messages = p.start();
    p.read(&mut messages, "a", json!({"path":path}));
    messages.push(json!({"role":"assistant","content":"read complete"}));
    messages.push(json!({"role":"user","content":"q".repeat(32_000)}));
    let tail = json!({"role":"assistant","content":"recent answer"});
    messages.push(tail.clone());
    let system = messages[0].clone();
    let mut meter = context::Meter::default();
    let mut log = crate::output::RunLog::new();
    for _ in 0..2 {
        context::before_request(
            &mut messages,
            &context::Policy {
                at: 1000,
                keep_recent: 100,
            },
            &mut meter,
            &mut log,
            &mut p.runtime.skills.borrow_mut(),
            &mut |_| Ok(summary_reply("work in progress")),
        );
        assert_eq!(messages[0], system);
        assert!(
            messages[1]["content"]
                .as_str()
                .unwrap()
                .contains("<compacted-summary>")
        );
        assert!(
            messages[2]["content"]
                .as_str()
                .unwrap()
                .starts_with("<retained-skills>")
        );
        assert_eq!(messages[3], tail);
        assert!(context::estimate(&messages) <= 1000);
        assert!(
            messages[2]["content"]
                .as_str()
                .unwrap()
                .contains("truncated")
        );
        // A second compaction replaces, rather than accumulates, the retained region.
        messages.push(json!({"role":"user","content":"q".repeat(32_000)}));
        messages.push(tail.clone());
    }
    assert_eq!(
        p.runtime.skills.borrow().loaded[&path].body,
        "rule\n".repeat(1000)
    );
}

#[test]
fn rejected_summary_preserves_skill_region_and_reload_information() {
    let p = Project::new();
    let path = p.write("review", "review", "rules");
    let mut messages = p.start();
    p.read(&mut messages, "a", json!({"path":path}));
    let region = p
        .runtime
        .skills
        .borrow()
        .preservation(TOTAL_TOKENS)
        .unwrap();
    messages.insert(1, region.message.clone());
    p.runtime.skills.borrow_mut().set_preserved(Some(region));
    messages.push(json!({"role":"user","content":"x".repeat(10_000)}));
    messages.push(json!({"role":"assistant","content":"last"}));
    let before = messages.clone();
    context::before_request(
        &mut messages,
        &context::Policy {
            at: 1000,
            keep_recent: 50,
        },
        &mut context::Meter::default(),
        &mut crate::output::RunLog::new(),
        &mut p.runtime.skills.borrow_mut(),
        &mut |_| Err(crate::api::ApiError::Http("test failure".into())),
    );
    assert_eq!(messages, before);
    assert!(
        p.read(&mut messages, "b", json!({"path":path}))
            .contains("already loaded")
    );
}

#[test]
fn retained_body_survives_snapshot_and_deduplicates_after_resume() {
    let p = Project::new();
    let path = p.write("review", "review", "saved rule");
    let mut messages = p.start();
    p.read(&mut messages, "a", json!({"path":path}));
    let region = p
        .runtime
        .skills
        .borrow()
        .preservation(TOTAL_TOKENS)
        .unwrap();
    messages = vec![
        messages[0].clone(),
        json!({"role":"user","content":"summary"}),
        region.message.clone(),
        json!({"role":"assistant","content":"done"}),
    ];
    p.runtime.skills.borrow_mut().set_preserved(Some(region));
    let config = crate::sessions::RequestConfig {
        model: record::ModelRequest {
            provider: "deepseek".into(),
            requested: "test".into(),
            params: json!({}),
        },
        max_output_tokens: 8192,
        tools: Toolset::new(&[READ_FILE]).unwrap().definitions().clone(),
        compaction: None,
    };
    let store = crate::sessions::Store::open(&p.root, None).unwrap();
    store
        .save(
            &config,
            &messages,
            &context::Meter::default(),
            &p.runtime.reader,
            &p.runtime.skills.borrow(),
        )
        .unwrap();
    let id = store.id.clone();
    drop(store);
    let store = crate::sessions::Store::open(&p.root, Some(&id)).unwrap();
    let restored = store
        .restore(
            Some(json!({"role":"system","content":"environment"})),
            &config,
        )
        .unwrap();
    assert_eq!(restored.messages, messages);
    p.runtime.skills.replace(restored.skills);
    assert!(
        p.read(&mut messages, "b", json!({"path":path}))
            .contains("already loaded")
    );
    p.write("review", "review", "changed after resume");
    assert!(
        p.read(&mut messages, "c", json!({"path":path}))
            .contains("changed after resume")
    );
}
