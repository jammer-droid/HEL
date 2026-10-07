//! Access levels decide whether a requested action may reach its tool implementation.
//! Tools describe the action; this module owns policy, approvals and the execution gate.

use std::collections::VecDeque;
use std::fs;
use std::io::{self, IsTerminal, Write};
use std::path::Path;

use crate::runtime::Runtime;
use serde::Serialize;
use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Access {
    ReadOnly,
    Confirm,
    Auto,
}

impl Access {
    pub fn parse(value: &str) -> Result<Self, String> {
        match value {
            "read-only" => Ok(Self::ReadOnly),
            "confirm" => Ok(Self::Confirm),
            "auto" => Ok(Self::Auto),
            _ => Err(format!(
                "unknown access level: {value} (read-only, confirm, auto)"
            )),
        }
    }

    pub fn decide(self, action: Action) -> Decision {
        match (self, action) {
            (_, Action::Read) | (Self::Auto, _) => Decision::Allow,
            (Self::ReadOnly, _) => Decision::Deny,
            (Self::Confirm, _) => Decision::Ask,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Action {
    Read,
    Write,
    Execute,
}

/// Required of every registered tool. The request is available for future input-aware policy.
/// Implementations must describe the action without performing it.
pub trait Approvable {
    fn action(&self, args: &Value) -> Action;
}

/// The gate is the only production caller of tool execution.
pub trait Tool: Approvable + Send + Sync {
    fn name(&self) -> &str;
    fn run(&self, runtime: &Runtime, args: &Value) -> Result<String, String>;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Decision {
    Allow,
    Ask,
    Deny,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Response {
    Approved,
    Denied,
    Unavailable,
}

pub trait Approval {
    /// A response applies only to the exact call passed here, never to later calls.
    fn request(&mut self, name: &str, args: &Value) -> Response;
}

pub enum Input {
    Terminal,
    Script(VecDeque<bool>),
    Unavailable,
}

impl Input {
    pub fn new(script: Option<&Path>) -> Result<Self, String> {
        if let Some(path) = script {
            let text = fs::read_to_string(path).map_err(|e| format!("approval input: {e}"))?;
            let responses: Vec<bool> = serde_json::from_str(&text)
                .map_err(|e| format!("approval input must be a JSON boolean list: {e}"))?;
            Ok(Self::Script(responses.into()))
        } else if io::stdin().is_terminal() {
            Ok(Self::Terminal)
        } else {
            Ok(Self::Unavailable)
        }
    }
}

impl Approval for Input {
    fn request(&mut self, name: &str, args: &Value) -> Response {
        match self {
            Self::Unavailable => Response::Unavailable,
            Self::Script(responses) => match responses.pop_front() {
                Some(true) => Response::Approved,
                Some(false) => Response::Denied,
                None => Response::Unavailable,
            },
            Self::Terminal => {
                // JSON escapes control characters in model-provided arguments.
                eprint!("permission: {name} {args}\nAllow this call once? [y/N] ");
                if io::stderr().flush().is_err() {
                    return Response::Unavailable;
                }
                let mut answer = String::new();
                match io::stdin().read_line(&mut answer) {
                    Ok(0) | Err(_) => Response::Unavailable,
                    Ok(_) if matches!(answer.trim().to_ascii_lowercase().as_str(), "y" | "yes") => {
                        Response::Approved
                    }
                    Ok(_) => Response::Denied,
                }
            }
        }
    }
}

#[derive(Debug, Serialize)]
pub struct Trace {
    pub name: String,
    pub args: Value,
    pub access: Access,
    pub action: Option<Action>,
    pub decision: Decision,
    pub approval: Option<Response>,
    /// True means the tool implementation was entered, not that its operation succeeded.
    pub executed: bool,
    /// A hook rejected the invocation before the permission policy was consulted.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub hook_blocked: bool,
}

pub struct Execution {
    pub result: Result<String, String>,
    pub trace: Trace,
}

pub fn execute(
    tool: Option<&dyn Tool>,
    name: &str,
    args: &Value,
    runtime: &Runtime,
    access: Access,
    approval: &mut dyn Approval,
) -> Execution {
    let mut trace = Trace {
        name: name.to_string(),
        args: args.clone(),
        access,
        action: None,
        decision: Decision::Deny,
        approval: None,
        executed: false,
        hook_blocked: false,
    };
    let Some(tool) = tool else {
        return Execution {
            result: Err(format!("unknown tool: {name}")),
            trace,
        };
    };
    let action = tool.action(args);
    trace.action = Some(action);
    trace.decision = access.decide(action);
    let allowed = match trace.decision {
        Decision::Allow => true,
        Decision::Deny => false,
        Decision::Ask => {
            let response = approval.request(name, args);
            trace.approval = Some(response);
            response == Response::Approved
        }
    };
    let result = if allowed {
        trace.executed = true;
        tool.run(runtime, args)
    } else {
        Err(format!(
            "permission denied: {name} (access={access:?}, approval={:?})",
            trace.approval
        ))
    };
    Execution { result, trace }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct NewTool<'a> {
        calls: &'a AtomicUsize,
    }
    impl Approvable for NewTool<'_> {
        fn action(&self, _: &Value) -> Action {
            Action::Write
        }
    }
    impl Tool for NewTool<'_> {
        fn name(&self) -> &str {
            "new-tool"
        }
        fn run(&self, _: &Runtime, _: &Value) -> Result<String, String> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            Ok("done".into())
        }
    }
    struct Observe<'a> {
        calls: &'a AtomicUsize,
        response: Response,
        prompts: usize,
    }
    impl Approval for Observe<'_> {
        fn request(&mut self, _: &str, _: &Value) -> Response {
            assert_eq!(
                self.calls.load(Ordering::SeqCst),
                0,
                "execution must wait for approval"
            );
            self.prompts += 1;
            self.response
        }
    }

    #[test]
    fn new_tool_uses_common_gate_and_waits_for_explicit_approval() {
        for response in [Response::Approved, Response::Denied, Response::Unavailable] {
            let calls = AtomicUsize::new(0);
            let tool = NewTool { calls: &calls };
            let mut approval = Observe {
                calls: &calls,
                response,
                prompts: 0,
            };
            let out = execute(
                Some(&tool),
                tool.name(),
                &json!({}),
                &Runtime::new(Path::new(".")).unwrap(),
                Access::Confirm,
                &mut approval,
            );
            assert_eq!(approval.prompts, 1);
            assert_eq!(
                calls.load(Ordering::SeqCst),
                usize::from(response == Response::Approved)
            );
            assert_eq!(out.trace.executed, response == Response::Approved);
            assert_eq!(out.result.is_ok(), response == Response::Approved);
        }
    }

    #[test]
    fn automatic_and_read_only_do_not_consume_approval() {
        for access in [Access::Auto, Access::ReadOnly] {
            let calls = AtomicUsize::new(0);
            let tool = NewTool { calls: &calls };
            let mut approval = Observe {
                calls: &calls,
                response: Response::Approved,
                prompts: 0,
            };
            let out = execute(
                Some(&tool),
                tool.name(),
                &json!({}),
                &Runtime::new(Path::new(".")).unwrap(),
                access,
                &mut approval,
            );
            assert_eq!(approval.prompts, 0);
            assert_eq!(out.trace.executed, access == Access::Auto);
            assert_eq!(
                calls.load(Ordering::SeqCst),
                usize::from(access == Access::Auto)
            );
        }
    }

    #[test]
    fn approval_is_one_call_only_and_exhaustion_fails_closed() {
        let calls = AtomicUsize::new(0);
        let tool = NewTool { calls: &calls };
        let mut input = Input::Script(VecDeque::from([true, false]));
        for expected in [true, false, false] {
            let out = execute(
                Some(&tool),
                tool.name(),
                &json!({}),
                &Runtime::new(Path::new(".")).unwrap(),
                Access::Confirm,
                &mut input,
            );
            assert_eq!(out.trace.executed, expected);
        }
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn script_input_is_strict_and_empty_input_does_not_approve() {
        let path = std::env::temp_dir().join(format!("hel-approval-{}.json", std::process::id()));
        for invalid in ["{}", "[1]", "[\"yes\"]", "not json"] {
            fs::write(&path, invalid).unwrap();
            assert!(Input::new(Some(&path)).is_err());
        }
        fs::write(&path, "[]").unwrap();
        assert_eq!(
            Input::new(Some(&path))
                .unwrap()
                .request("write", &json!({})),
            Response::Unavailable
        );
        fs::remove_file(path).unwrap();
    }

    #[test]
    fn unknown_tool_never_executes_even_in_auto() {
        let mut input = Input::Script(VecDeque::from([true]));
        let out = execute(
            None,
            "unknown",
            &json!({}),
            &Runtime::new(Path::new(".")).unwrap(),
            Access::Auto,
            &mut input,
        );
        assert!(!out.trace.executed);
        assert_eq!(out.trace.decision, Decision::Deny);
        assert_eq!(input.request("next", &json!({})), Response::Approved);
    }
}
