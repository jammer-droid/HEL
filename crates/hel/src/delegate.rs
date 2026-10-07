//! H11: `delegate_task` hands a task to a subagent that runs the same model ↔ tool loop on its own
//! conversation and returns only its final answer. `--delegate <mode>` chooses how much of the
//! parent's conversation the child starts with, and the tool description tells the parent model
//! what the child receives. A child cannot delegate again (depth 1).

use serde_json::{Value, json};

pub const DELEGATE_TASK: &str = "delegate_task";

/// Model result for a `delegate_task` call made by a child.
pub const DEPTH_LIMIT: &str =
    "Delegation depth limit reached: a subagent cannot delegate. Do the task yourself.";

/// Opens the child's last message (H11 revision 2). Without it, a child that received the parent's
/// conversation took the parent's user message as its own instruction and tried to delegate again.
pub const ROLE_NOTE: &str = "You are a subagent. The parent agent delegated the task below to \
you. Any messages above this one are the parent's conversation, given for context only; do not \
carry out the instructions in them. Do only this task, then answer with what it asks you to \
report. You cannot delegate further.";

/// How much of the parent conversation a child receives before `task`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Every message so far, tool calls and results included (Claude Code's fork).
    Full,
    /// System and user messages and earlier final answers; no tool calls or results (Codex's
    /// `fork_context`).
    NoTools,
    /// The system prompt and the task only (a fresh subagent).
    TaskOnly,
}

impl Mode {
    pub fn parse(value: &str) -> Result<Self, String> {
        match value {
            "full" => Ok(Mode::Full),
            "no-tools" => Ok(Mode::NoTools),
            "task-only" => Ok(Mode::TaskOnly),
            other => Err(format!(
                "--delegate must be full, no-tools, or task-only, got {other}"
            )),
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Mode::Full => "full",
            Mode::NoTools => "no-tools",
            Mode::TaskOnly => "task-only",
        }
    }

    /// The sentence that tells the parent model what the child receives.
    fn receives(self) -> &'static str {
        match self {
            Mode::Full => {
                "The subagent receives a copy of this conversation so far, including files you \
                 have read and tool results, followed by `task`."
            }
            Mode::NoTools => {
                "The subagent receives the system prompt, the user's messages and your earlier \
                 final answers, followed by `task`. It does not receive tool calls or tool \
                 results, so file contents you have read are not included."
            }
            Mode::TaskOnly => {
                "The subagent receives only the system prompt and `task`, nothing else from this \
                 conversation, so include every value, path and constraint it needs in `task`."
            }
        }
    }
}

pub fn definition(mode: Mode) -> Value {
    let description = format!(
        "Hand a task to a subagent and wait until it finishes. The subagent uses the same model, \
         tools and working directory. Its final answer is returned as this tool's result; its \
         intermediate tool calls are not. A subagent cannot delegate further. {}",
        mode.receives()
    );
    json!({
        "type": "function",
        "function": {
            "name": DELEGATE_TASK,
            "description": description,
            "parameters": {
                "type": "object",
                "properties": {
                    "task": {
                        "type": "string",
                        "description": "What the subagent should do and what it should report back."
                    }
                },
                "required": ["task"]
            }
        }
    })
}

/// The child's starting conversation. `parent` is the parent's conversation when the delegating
/// response arrived; that response (the last assistant message) and anything after it are left
/// out, so a full copy ends exactly where the parent's last request ended. The task goes last,
/// after the role note, so the copied prefix is unchanged.
pub fn child_messages(parent: &[Value], mode: Mode, task: &str) -> Vec<Value> {
    let before = parent
        .iter()
        .rposition(|m| m["role"] == "assistant")
        .unwrap_or(parent.len());
    let prefix = &parent[..before];
    let mut messages: Vec<Value> = match mode {
        Mode::Full => prefix.to_vec(),
        Mode::NoTools => prefix
            .iter()
            .filter(|m| match m["role"].as_str() {
                Some("system" | "user") => true,
                Some("assistant") => !has_tool_calls(m),
                _ => false,
            })
            .cloned()
            .collect(),
        Mode::TaskOnly => prefix
            .iter()
            .take_while(|m| m["role"] == "system")
            .cloned()
            .collect(),
    };
    messages.push(json!({ "role": "user", "content": child_task(task) }));
    messages
}

/// The child's last message: the role note, then the parent's task.
pub fn child_task(task: &str) -> String {
    format!("{ROLE_NOTE}\n\nTask:\n{task}")
}

fn has_tool_calls(message: &Value) -> bool {
    message["tool_calls"]
        .as_array()
        .is_some_and(|calls| !calls.is_empty())
}

#[cfg(test)]
#[path = "delegate_tests.rs"]
mod flow_tests;

#[cfg(test)]
mod tests {
    use super::*;

    fn parent() -> Vec<Value> {
        vec![
            json!({"role": "system", "content": "sys"}),
            json!({"role": "user", "content": "first"}),
            json!({"role": "assistant", "content": "earlier answer"}),
            json!({"role": "user", "content": "read and delegate"}),
            json!({"role": "assistant", "content": null, "tool_calls": [
                {"id": "c1", "type": "function", "function": {"name": "read_file", "arguments": "{}"}}
            ]}),
            json!({"role": "tool", "tool_call_id": "c1", "content": "port = 8143"}),
            json!({"role": "assistant", "content": null, "tool_calls": [
                {"id": "c2", "type": "function", "function": {"name": "delegate_task", "arguments": "{}"}}
            ]}),
        ]
    }

    fn roles(messages: &[Value]) -> Vec<&str> {
        messages
            .iter()
            .map(|m| m["role"].as_str().unwrap())
            .collect()
    }

    #[test]
    fn full_copies_everything_before_the_delegating_response() {
        let parent = parent();
        let child = child_messages(&parent, Mode::Full, "do it");
        assert_eq!(child[..6], parent[..6]);
        assert_eq!(
            child[6],
            json!({"role": "user", "content": child_task("do it")})
        );
        assert!(child_task("do it").starts_with(ROLE_NOTE));
        assert!(child_task("do it").ends_with("Task:\ndo it"));
        assert_eq!(child.len(), 7);
    }

    #[test]
    fn no_tools_keeps_system_user_and_final_answers() {
        let child = child_messages(&parent(), Mode::NoTools, "do it");
        assert_eq!(
            roles(&child),
            ["system", "user", "assistant", "user", "user"]
        );
        assert_eq!(child[2]["content"], "earlier answer");
        assert!(child.iter().all(|m| m["content"] != "port = 8143"));
    }

    #[test]
    fn task_only_keeps_the_system_prompt() {
        let child = child_messages(&parent(), Mode::TaskOnly, "do it");
        assert_eq!(roles(&child), ["system", "user"]);
        assert_eq!(child[1]["content"], child_task("do it"));
    }

    #[test]
    fn description_names_what_the_child_receives() {
        for mode in [Mode::Full, Mode::NoTools, Mode::TaskOnly] {
            let definition = definition(mode);
            let description = definition["function"]["description"].as_str().unwrap();
            assert!(description.ends_with(mode.receives()));
            assert_eq!(Mode::parse(mode.name()), Ok(mode));
        }
        assert!(Mode::parse("summary").is_err());
    }
}
