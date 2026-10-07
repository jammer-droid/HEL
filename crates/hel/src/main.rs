//! hel — the harness built in this project.
//!
//! A model ↔ tool loop. The model is called repeatedly; each tool call it makes is executed and
//! its result sent back, until the model answers without a tool call or the turn budget runs out.
//! Tools: `bash`, `read_file` (H1), `write_file`, `search_replace` (H2); `--tools` chooses which
//! ones the model gets (default: bash). H4 adds ripgrep-backed `glob` and `grep`.
//! A system message (H3) carries the execution environment
//! and `HEL.md` from the working directory; `--no-env` and `--no-context-file` leave them out, and
//! `--context-file <name>` reads another file instead of `HEL.md`.
//!
//! Usage:
//!   hel [--tools <a,b>] [--no-env] [--context-file <name> | --no-context-file]
//!   hel --instruction <TEXT> [--tools <a,b>] [--no-env] [--context-file <name> | --no-context-file]
//!       [--context <run-context.json> --record <record.json>]
//!   hel --turns-file <turns.json> [same options as --instruction]
//!
//! Without `--instruction`, hel runs interactively: each line typed is sent to the model with the
//! conversation so far. `--context` and `--record` are used by the eval runner. Without them, hel
//! only prints the answer. `--turns-file` (H6) reads a JSON list of instructions and sends them one
//! after another in one session, writing a single record whose final output is the last answer.
//! Context management (H6) is on by default with DeepSeek Harness's thresholds for a 1M window:
//! large tool results go to a file, and over the threshold old history is pruned and summarized by
//! the model. `--compact-at <tokens> --keep-recent <tokens>` change the thresholds and
//! `--no-compaction` turns it off.
//! `--delegate full|no-tools|task-only` (H11) offers `delegate_task`: a subagent runs this loop
//! on its own conversation, starting from as much of the parent's as the mode allows, and its
//! final answer becomes the tool result.
//! `--parallel` (H12) runs the read-only calls (`read_file`, `glob`, `grep`) of one response at
//! the same time, and several `delegate_task` calls of one response as concurrent subagents.
//! Results enter the conversation in the order the model asked for them.

mod api;
mod context;
mod delegate;
mod hooks;
mod mcp;
mod output;
mod permissions;
mod prompt;
mod read_file;
mod runtime;
mod sandbox;
mod search;
mod sessions;
mod shared;
mod skills;
mod tools;

use std::env;
use std::error::Error;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::{Duration, Instant, SystemTime};

use record::{
    Budget, ComparisonClass, Harness, ModelRequest, RunContext, RunInfo, Termination, ToolEvent,
};
use serde_json::{Value, json};

use output::RunLog;
use permissions::{Access, Approval};
use tools::Toolset;

enum Management {
    List,
    Delete(String),
}

struct Args {
    resume: Option<String>,
    management: Option<Management>,
    instruction: Option<String>,
    turns_file: Option<PathBuf>,
    tools: Toolset,
    env: bool,
    context_file: Option<String>,
    context: Option<PathBuf>,
    record: Option<PathBuf>,
    compaction: Compaction,
    access: Access,
    approval_input: Option<PathBuf>,
    parallel: bool,
}

/// How context management is chosen on the command line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Compaction {
    /// DeepSeek Harness's thresholds for the model window and output limit.
    Default,
    Custom(context::Policy),
    Off,
}

impl Compaction {
    fn policy(self, max_output_tokens: u32) -> Option<context::Policy> {
        match self {
            Compaction::Default => Some(context::Policy::deepseek_default(
                context::DEFAULT_WINDOW,
                u64::from(max_output_tokens),
            )),
            Compaction::Custom(policy) => Some(policy),
            Compaction::Off => None,
        }
    }
}

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("hel: {err}");
            ExitCode::from(2)
        }
    }
}

fn run() -> Result<(), Box<dyn Error>> {
    run_args(parse_args(env::args().skip(1))?)
}

fn run_args(args: Args) -> Result<(), Box<dyn Error>> {
    let workdir = env::current_dir()?;
    if let Some(command) = &args.management {
        match command {
            Management::List => {
                let rows = sessions::list(&workdir)?;
                if rows.is_empty() {
                    println!("No sessions in this project.");
                }
                for row in rows {
                    println!("{row}");
                }
            }
            Management::Delete(id) => {
                sessions::delete(&workdir, id)?;
                println!("Deleted session {id}.");
            }
        }
        return Ok(());
    }
    let ctx = match &args.context {
        Some(path) => serde_json::from_str(&fs::read_to_string(path)?)?,
        None => manual_context(),
    };
    let api_key = env::var("DEEPSEEK_API_KEY").map_err(|_| "DEEPSEEK_API_KEY is not set")?;
    let client = api::Client::new(
        api_key,
        &ctx.model.requested,
        ctx.budget.max_output_tokens,
        &ctx.model.params,
        Duration::from_secs(ctx.budget.timeout_seconds),
    )?;
    let mut approval = permissions::Input::new(args.approval_input.as_deref())?;
    let system = prompt::system_message(args.env, args.context_file.as_deref(), &workdir);
    let compaction = args.compaction.policy(ctx.budget.max_output_tokens);
    let config = sessions::RequestConfig {
        model: ctx.model.clone(),
        max_output_tokens: ctx.budget.max_output_tokens,
        tools: args.tools.definitions().clone(),
        compaction,
    };
    let store = sessions::Store::open(&workdir, args.resume.as_deref())?;
    let mut runtime = store.runtime()?;
    // H10: servers start fresh in every process, so a resumed session reloads MCP tools.
    runtime.mcp = mcp::Mcp::load(&runtime);
    let system = runtime.mcp.extend_system(system);
    let mut restored = if args.resume.is_some() {
        store.restore(system.clone(), &config)?
    } else {
        sessions::Restored {
            messages: system.clone().into_iter().collect(),
            meter: context::Meter::default(),
            reader: read_file::Reader::default(),
            skills: skills::Skills::default(),
        }
    };
    let skills_enabled = config.tools.as_array().is_some_and(|tools| {
        tools
            .iter()
            .any(|t| t["function"]["name"] == tools::READ_FILE)
    });
    restored.skills.configure(system, skills_enabled);
    if !restored.skills.initialized && skills_enabled {
        restored.skills.discover(&runtime);
    }
    restored
        .skills
        .sync_system(&mut restored.messages, &mut restored.meter);
    runtime.skills = shared::Shared::new(restored.skills);
    runtime.reader = restored.reader;
    runtime.hooks = hooks::Hooks::load(&runtime.project, &store.id);
    let mut messages = restored.messages;
    let mut meter = restored.meter;
    eprintln!("session: {}", store.id);
    let inputs: Vec<String> = match (&args.instruction, &args.turns_file) {
        (Some(instruction), _) => vec![instruction.clone()],
        (None, Some(path)) => read_turns(path)?,
        (None, None) => {
            return chat(
                &client,
                &runtime,
                &args.tools,
                messages,
                meter,
                &store,
                &config,
                ctx.budget.max_turns,
                compaction.as_ref(),
                args.access,
                &mut approval,
                args.parallel,
            );
        }
    };
    let session = args.turns_file.is_some();

    let started = SystemTime::now();
    let clock = Instant::now();
    let mut log = RunLog::new();

    for (index, input) in inputs.iter().enumerate() {
        if session {
            log.turn = Some(index as u32 + 1);
        }
        // Only the last turn's answer is the run's output; a failed turn must not inherit one.
        log.final_output = None;
        messages.push(json!({ "role": "user", "content": input }));
        run_loop(
            &mut Session {
                client: &client,
                runtime: &runtime,
                tools: &args.tools,
                compaction: compaction.as_ref(),
                access: args.access,
                approval: &mut approval,
                depth: 0,
                parallel: args.parallel,
            },
            &mut messages,
            &mut meter,
            ctx.budget.max_turns,
            &mut log,
        );
        sessions::save_completed(
            &store,
            &config,
            &messages,
            &meter,
            &runtime.reader,
            &runtime.skills.borrow(),
            &log,
        );
        if session && let Some(output) = &log.final_output {
            println!("[turn {}] {output}", index + 1);
        }
        if log.termination != Termination::Completed || log.error.is_some() {
            break;
        }
    }

    let wall_time_ms = clock.elapsed().as_millis();
    let ended = SystemTime::now();

    if !session && let Some(output) = &log.final_output {
        println!("{output}");
    }
    if let Some(error) = &log.error {
        eprintln!("hel: {error}");
    }
    if let Some(path) = &args.record {
        log.write(&ctx, started, ended, wall_time_ms, path)?;
        runtime.hooks.write_trace(path)?;
        runtime.mcp.write_trace(path)?;
    }
    Ok(())
}

/// Interactive mode: reads one line at a time and runs the loop on it with the conversation so
/// far. `max_turns` applies to each input. Ends on `/exit` or end of input (Ctrl-D).
#[allow(clippy::too_many_arguments)]
fn chat(
    client: &api::Client,
    runtime: &runtime::Runtime,
    tools: &Toolset,
    mut messages: Vec<Value>,
    mut meter: context::Meter,
    store: &sessions::Store,
    config: &sessions::RequestConfig,
    max_turns: u32,
    compaction: Option<&context::Policy>,
    access: Access,
    approval: &mut (dyn Approval + Send),
    parallel: bool,
) -> Result<(), Box<dyn Error>> {
    let stdin = io::stdin();
    let mut session = Session {
        client,
        runtime,
        tools,
        compaction,
        access,
        approval,
        depth: 0,
        parallel,
    };
    eprintln!("hel — type /exit or press Ctrl-D to quit");
    loop {
        print!("> ");
        io::stdout().flush()?;
        let mut line = String::new();
        if stdin.read_line(&mut line)? == 0 {
            println!();
            return Ok(());
        }
        let input = line.trim();
        if input.is_empty() {
            continue;
        }
        if input == "/exit" {
            return Ok(());
        }

        messages.push(json!({ "role": "user", "content": input }));
        let mut log = RunLog::new();
        run_loop(&mut session, &mut messages, &mut meter, max_turns, &mut log);

        for event in &log.events {
            let status = if event.ok == Some(false) {
                " (failed)"
            } else {
                ""
            };
            eprintln!("[tool] {} {}{status}", event.name, event.args);
        }
        if let Some(output) = &log.final_output {
            println!("{output}");
        }
        if let Some(error) = &log.error {
            eprintln!("hel: {error}");
        }
        if log.termination == Termination::MaxTurns {
            eprintln!("hel: stopped after {max_turns} model calls");
        }
        sessions::save_completed(
            store,
            config,
            &messages,
            &meter,
            &runtime.reader,
            &runtime.skills.borrow(),
            &log,
        );
        if let Some(summary) = log.context_summary() {
            eprintln!("[{summary}]");
        }
    }
}

/// Calls the model until it answers without a tool call, an error occurs, or `max_turns`
/// model calls have been made.
/// What one model ↔ tool loop needs besides the conversation.
struct Session<'a> {
    client: &'a api::Client,
    runtime: &'a runtime::Runtime,
    tools: &'a Toolset,
    compaction: Option<&'a context::Policy>,
    access: Access,
    approval: &'a mut (dyn Approval + Send),
    /// 0 for the agent the user talks to, 1 for a subagent (H11).
    depth: u32,
    /// H12: run read-only calls and subagents of one response at the same time.
    parallel: bool,
}

fn run_loop(
    session: &mut Session,
    messages: &mut Vec<Value>,
    meter: &mut context::Meter,
    max_turns: u32,
    log: &mut RunLog,
) {
    for _ in 0..max_turns {
        let Session {
            client,
            runtime,
            tools,
            compaction,
            ..
        } = session;
        // Loaded MCP tools join the definitions from the request after loading (H10).
        let definitions = tools.request_definitions(runtime);
        if let Some(policy) = compaction {
            context::before_request(
                messages,
                policy,
                meter,
                log,
                &mut runtime.skills.borrow_mut(),
                &mut |request| client.complete(request, Some(&definitions)),
            );
        }
        let exchange = match client.complete(messages, Some(&definitions)) {
            Ok(exchange) => exchange,
            Err(err) => {
                log.failed(&err);
                return;
            }
        };
        log.exchange(&exchange);
        if let Some(usage) = &exchange.response.usage {
            meter.observed(messages.len(), usage.prompt_tokens);
        }
        let Some(choice) = exchange.response.choices.first() else {
            log.termination = Termination::Error;
            log.error = Some("response has no choices".to_string());
            return;
        };

        // Send the assistant message back unchanged, including reasoning_content and tool_calls.
        messages.push(choice.message.clone());

        let calls = choice
            .message
            .get("tool_calls")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        if calls.is_empty() {
            log.final_output = choice
                .message
                .get("content")
                .and_then(Value::as_str)
                .map(str::to_string);
            if choice.finish_reason.as_deref() == Some("length") {
                log.termination = Termination::MaxOutputTokens;
            }
            return;
        }

        let calls: Vec<Call> = calls.iter().map(Call::parse).collect();
        let mut next = 0;
        while next < calls.len() {
            let rest = &calls[next..];
            let together = concurrent_run(session, rest);
            if together >= 2 {
                let batch = &rest[..together];
                let results = if batch[0].name == delegate::DELEGATE_TASK {
                    delegate_tasks(session, messages, max_turns, batch, log)
                } else {
                    read_together(session, messages, meter, batch, log)
                };
                for (call, result) in batch.iter().zip(results) {
                    deliver(session, messages, call, result, log);
                }
            } else {
                let call = &rest[0];
                let result =
                    if call.name == delegate::DELEGATE_TASK && session.tools.delegate().is_some() {
                        let mut outcomes = delegate_tasks(
                            session,
                            messages,
                            max_turns,
                            std::slice::from_ref(call),
                            log,
                        );
                        outcomes.remove(0)
                    } else {
                        let Session {
                            runtime,
                            tools,
                            access,
                            approval,
                            ..
                        } = session;
                        runtime.skills.borrow_mut().reconcile(messages);
                        let execution = tools.call_with_id(
                            runtime,
                            &call.id,
                            &call.name,
                            &call.args,
                            *access,
                            &mut **approval,
                        );
                        runtime.skills.borrow().sync_system(messages, meter);
                        log.permissions.push(execution.trace);
                        execution.result
                    };
                deliver(session, messages, call, result, log);
            }
            next += together.max(1);
        }
    }
    log.termination = Termination::MaxTurns;
}

/// One tool call from a model response, with its JSON-string arguments decoded.
struct Call {
    id: String,
    name: String,
    args: Value,
}

impl Call {
    fn parse(call: &Value) -> Self {
        Self {
            id: call["id"].as_str().unwrap_or_default().to_string(),
            name: call["function"]["name"]
                .as_str()
                .unwrap_or_default()
                .to_string(),
            // `arguments` is a JSON object encoded as a string.
            args: call["function"]["arguments"]
                .as_str()
                .and_then(|raw| serde_json::from_str(raw).ok())
                .unwrap_or_else(|| json!({})),
        }
    }
}

/// H12: how many calls from the start of `calls` run at the same time, 0 or 1 meaning one by one.
/// With `--parallel`, consecutive read-only builtin calls (`read_file`, `glob`, `grep`) form one
/// group and consecutive `delegate_task` calls of the top-level agent another. Calls run one by
/// one while hooks are configured or skills are registered, since both keep per-call state.
fn concurrent_run(session: &Session, calls: &[Call]) -> usize {
    if !session.parallel
        || !session.runtime.hooks.is_empty()
        || !session.runtime.skills.borrow().idle()
    {
        return 0;
    }
    let delegate = |c: &Call| {
        c.name == delegate::DELEGATE_TASK
            && session.tools.delegate().is_some()
            && session.depth == 0
    };
    let read = |c: &Call| session.tools.offers_read_only(&c.name);
    let same: &dyn Fn(&Call) -> bool = match calls.first() {
        Some(c) if delegate(c) => &delegate,
        Some(c) if read(c) => &read,
        _ => return 0,
    };
    calls.iter().take_while(|c| same(c)).count()
}

/// Runs read-only calls at the same time (H12). Reads never ask for approval, so each thread
/// gets an unavailable approval input. Results come back in the order of `calls`.
fn read_together(
    session: &mut Session,
    messages: &mut Vec<Value>,
    meter: &mut context::Meter,
    calls: &[Call],
    log: &mut RunLog,
) -> Vec<Result<String, String>> {
    let Session {
        runtime,
        tools,
        access,
        ..
    } = session;
    let (runtime, tools, access) = (*runtime, *tools, *access);
    runtime.skills.borrow_mut().reconcile(messages);
    let executions: Vec<permissions::Execution> = std::thread::scope(|scope| {
        let handles: Vec<_> = calls
            .iter()
            .map(|call| {
                scope.spawn(move || {
                    let mut approval = permissions::Input::Unavailable;
                    tools.call_with_id(
                        runtime,
                        &call.id,
                        &call.name,
                        &call.args,
                        access,
                        &mut approval,
                    )
                })
            })
            .collect();
        handles
            .into_iter()
            .map(|h| {
                h.join()
                    .unwrap_or_else(|panic| std::panic::resume_unwind(panic))
            })
            .collect()
    });
    runtime.skills.borrow().sync_system(messages, meter);
    executions
        .into_iter()
        .map(|execution| {
            log.permissions.push(execution.trace);
            execution.result
        })
        .collect()
}

/// Records one call's result and adds it to the conversation as a tool message.
fn deliver(
    session: &Session,
    messages: &mut Vec<Value>,
    call: &Call,
    result: Result<String, String>,
    log: &mut RunLog,
) {
    log.events.push(ToolEvent {
        seq: log.events.len() as u32 + 1,
        category: tools::category(&call.name),
        name: call.name.clone(),
        args: call.args.clone(),
        ok: Some(result.is_ok()),
    });
    let mut content = result.unwrap_or_else(|err| format!("error: {err}"));
    if session.compaction.is_some() {
        content = context::spill(session.runtime, content);
    }
    session
        .runtime
        .skills
        .borrow_mut()
        .delivered(&call.id, &content);
    messages.push(json!({ "role": "tool", "tool_call_id": call.id, "content": content }));
}

/// Approval shared by subagents running at the same time (H12): one request at a time.
struct OneAtATime<'a, 'b, 'c>(&'a std::sync::Mutex<&'b mut (dyn Approval + Send + 'c)>);

impl Approval for OneAtATime<'_, '_, '_> {
    fn request(&mut self, name: &str, args: &Value) -> permissions::Response {
        let mut approval = self.0.lock().unwrap_or_else(|p| p.into_inner());
        approval.request(name, args)
    }
}

fn now_ms() -> u128 {
    SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map_or(0, |d| d.as_millis())
}

/// Runs `delegate_task` calls (H11): each a child loop on its own conversation, with the
/// parent's client, runtime, tools, access level and approval. The child's skill state starts as
/// a copy of the parent's and the parent's is restored afterwards. Only the child's final answer
/// is returned; its requests join the run's raw log and usage, and the call is traced. More than
/// one call (H12, `--parallel`) runs the children at the same time and traces when each started
/// and ended; approvals are then asked one at a time. Results follow the order of `calls`.
fn delegate_tasks(
    session: &mut Session,
    messages: &[Value],
    max_turns: u32,
    calls: &[Call],
    log: &mut RunLog,
) -> Vec<Result<String, String>> {
    let mode = session
        .tools
        .delegate()
        .expect("delegate_task is offered only with a mode");
    let parent_skills = session.runtime.skills.borrow().clone();
    struct Child {
        trace: Value,
        messages: Vec<Value>,
        log: RunLog,
        refused: Option<String>,
    }
    let mut children: Vec<Child> = calls
        .iter()
        .enumerate()
        .map(|(i, call)| {
            let task = call.args["task"].as_str().unwrap_or_default();
            let mut trace = json!({
                "seq": log.delegations.len() + i + 1,
                "call_id": call.id,
                "mode": mode.name(),
                "depth": session.depth,
                "task": task,
                "concurrent": calls.len(),
            });
            let refused = if session.depth >= 1 {
                Some(delegate::DEPTH_LIMIT.to_string())
            } else if task.trim().is_empty() {
                Some("delegate_task needs a non-empty task".to_string())
            } else {
                None
            };
            let messages = if refused.is_none() {
                let messages = delegate::child_messages(messages, mode, task);
                trace["child_start_messages"] = json!(messages.len());
                messages
            } else {
                Vec::new()
            };
            let mut child_log = RunLog::new();
            child_log.agent = Some("child");
            child_log.turn = log.turn;
            Child {
                trace,
                messages,
                log: child_log,
                refused,
            }
        })
        .collect();
    let (client, runtime, tools) = (session.client, session.runtime, session.tools);
    let (compaction, access, depth, parallel) = (
        session.compaction,
        session.access,
        session.depth + 1,
        session.parallel,
    );
    let run = |child: &mut Child, approval: &mut (dyn Approval + Send)| {
        if child.refused.is_some() {
            return;
        }
        child.trace["started_at_ms"] = json!(now_ms());
        let mut session = Session {
            client,
            runtime,
            tools,
            compaction,
            access,
            approval,
            depth,
            parallel,
        };
        run_loop(
            &mut session,
            &mut child.messages,
            &mut context::Meter::default(),
            max_turns,
            &mut child.log,
        );
        child.trace["ended_at_ms"] = json!(now_ms());
    };
    if children.len() == 1 {
        run(&mut children[0], &mut *session.approval);
    } else {
        let shared = std::sync::Mutex::new(&mut *session.approval);
        std::thread::scope(|scope| {
            for child in &mut children {
                let (run, shared) = (&run, &shared);
                scope.spawn(move || run(child, &mut OneAtATime(shared)));
            }
        });
    }
    *session.runtime.skills.borrow_mut() = parent_skills;
    children
        .into_iter()
        .map(
            |Child {
                 mut trace,
                 log: child_log,
                 refused,
                 ..
             }| {
                let result = match refused {
                    Some(reason) => Err(reason),
                    None => {
                        log.absorb_child(&child_log);
                        trace["child_requests"] = json!(child_log.requests());
                        trace["child_events"] = json!(child_log.events);
                        trace["child_permissions"] = json!(child_log.permissions);
                        trace["child_delegations"] = json!(child_log.delegations);
                        trace["termination"] = json!(child_log.termination);
                        trace["error"] = json!(child_log.error);
                        match (&child_log.final_output, child_log.termination) {
                            (Some(answer), Termination::Completed) if child_log.error.is_none() => {
                                Ok(answer.clone())
                            }
                            _ => Err(format!(
                                "the subagent ended without a final answer ({})",
                                child_log
                                    .error
                                    .clone()
                                    .unwrap_or_else(|| format!("{:?}", child_log.termination))
                            )),
                        }
                    }
                };
                trace["result"] = match &result {
                    Ok(answer) => json!({ "ok": answer }),
                    Err(err) => json!({ "error": err }),
                };
                log.delegations.push(trace);
                result
            },
        )
        .collect()
}

/// A positive token count for `flag`.
fn tokens(flag: &str, value: &str) -> Result<u64, String> {
    value.parse::<u64>().ok().filter(|n| *n > 0).ok_or(format!(
        "{flag} needs a positive number of tokens, got {value:?}"
    ))
}

/// Reads a JSON list of non-empty instructions.
fn read_turns(path: &Path) -> Result<Vec<String>, Box<dyn Error>> {
    let turns: Vec<String> = serde_json::from_str(&fs::read_to_string(path)?)
        .map_err(|e| format!("{}: expected a JSON list of strings: {e}", path.display()))?;
    if turns.is_empty() || turns.iter().any(|t| t.trim().is_empty()) {
        return Err(format!(
            "{}: turns must be a non-empty list of instructions",
            path.display()
        )
        .into());
    }
    Ok(turns)
}

fn parse_args(args: impl Iterator<Item = String>) -> Result<Args, String> {
    let mut args = args.peekable();
    let management = if args.peek().is_some_and(|a| a == "sessions") {
        args.next();
        let command = match args.next().as_deref() {
            None => Management::List,
            Some("delete") => Management::Delete(args.next().ok_or("sessions delete needs an ID")?),
            _ => return Err("usage: hel sessions [delete <id>]".into()),
        };
        if args.next().is_some() {
            return Err("usage: hel sessions [delete <id>]".into());
        }
        Some(command)
    } else {
        None
    };
    let mut resume = None;
    let mut access = Access::Confirm;
    let mut approval_input = None;
    let mut instruction = None;
    let mut turns_file = None;
    let mut compact_at = None;
    let mut keep_recent = None;
    let mut no_compaction = false;
    let mut tools = None;
    let mut delegate = None;
    let mut parallel = false;
    let mut env = true;
    let mut context_file = Some(prompt::DEFAULT_CONTEXT_FILE.to_string());
    let mut context = None;
    let mut record = None;
    while let Some(flag) = args.next() {
        let mut value = || args.next().ok_or(format!("{flag} needs a value"));
        match flag.as_str() {
            "--resume" => resume = Some(value()?),
            "--access" => access = Access::parse(&value()?)?,
            "--approval-input" => approval_input = Some(PathBuf::from(value()?)),
            "--instruction" => instruction = Some(value()?),
            "--turns-file" => turns_file = Some(PathBuf::from(value()?)),
            "--compact-at" => compact_at = Some(tokens(&flag, &value()?)?),
            "--keep-recent" => keep_recent = Some(tokens(&flag, &value()?)?),
            "--no-compaction" => no_compaction = true,
            "--tools" => {
                let list = value()?;
                let names: Vec<&str> = list.split(',').collect();
                tools = Some(Toolset::new(&names)?);
            }
            "--delegate" => delegate = Some(delegate::Mode::parse(&value()?)?),
            "--parallel" => parallel = true,
            "--env" => env = true,
            "--no-env" => env = false,
            "--context-file" => context_file = Some(value()?),
            "--no-context-file" => context_file = None,
            "--context" => context = Some(PathBuf::from(value()?)),
            "--record" => record = Some(PathBuf::from(value()?)),
            other => return Err(format!("unknown argument: {other}")),
        }
    }
    if record.is_some() && context.is_none() {
        return Err("--record needs --context".to_string());
    }
    let compaction = match (compact_at, keep_recent) {
        (None, None) if no_compaction => Compaction::Off,
        (None, None) => Compaction::Default,
        _ if no_compaction => {
            return Err("--no-compaction cannot be combined with --compact-at".to_string());
        }
        (Some(at), Some(keep_recent)) if keep_recent < at => {
            Compaction::Custom(context::Policy { at, keep_recent })
        }
        (Some(_), Some(_)) => return Err("--keep-recent must be below --compact-at".to_string()),
        _ => return Err("--compact-at and --keep-recent go together".to_string()),
    };
    if instruction.is_some() && turns_file.is_some() {
        return Err("use either --instruction or --turns-file".to_string());
    }
    if context.is_some() && instruction.is_none() && turns_file.is_none() {
        return Err("--context needs --instruction or --turns-file".to_string());
    }
    if approval_input.is_some()
        && (context.is_none() || record.is_none() || access != Access::Confirm)
    {
        return Err(
            "--approval-input requires --context, --record and --access confirm (eval only)"
                .to_string(),
        );
    }
    let mut tools = match tools {
        Some(tools) => tools,
        None => Toolset::new(tools::DEFAULT)?,
    };
    if let Some(mode) = delegate {
        tools = tools.with_delegate(mode);
    }
    Ok(Args {
        resume,
        management,
        access,
        approval_input,
        instruction,
        turns_file,
        compaction,
        tools,
        env,
        context_file,
        context,
        record,
        parallel,
    })
}

/// Defaults for running hel by hand without the eval runner.
fn manual_context() -> RunContext {
    RunContext {
        run: RunInfo {
            run_id: "manual".to_string(),
            experiment_id: "manual".to_string(),
            lab: "h00".to_string(),
            task_id: "manual".to_string(),
            condition: "variant".to_string(),
            repetition: 1,
        },
        harness: Harness {
            name: "hel".to_string(),
            version: "dev".to_string(),
            comparison_class: ComparisonClass::Subject,
        },
        model: ModelRequest {
            provider: "deepseek".to_string(),
            requested: "deepseek-flash".to_string(),
            params: json!({}),
        },
        budget: Budget {
            max_turns: 5,
            timeout_seconds: 120,
            max_output_tokens: 8192,
        },
    }
}

#[cfg(test)]
#[path = "parallel_tests.rs"]
mod parallel_tests;

#[cfg(test)]
mod tests {
    use super::*;

    fn tool_names(args: &Args) -> Vec<String> {
        args.tools
            .definitions()
            .as_array()
            .unwrap()
            .iter()
            .map(|d| d["function"]["name"].as_str().unwrap().to_string())
            .collect()
    }

    fn parse(list: &[&str]) -> Result<Args, String> {
        parse_args(list.iter().map(|s| s.to_string()))
    }

    #[test]
    fn access_defaults_to_confirmation_and_eval_input_requires_explicit_context() {
        assert_eq!(parse(&[]).unwrap().access, Access::Confirm);
        assert_eq!(parse(&["--access", "auto"]).unwrap().access, Access::Auto);
        assert_eq!(
            parse(&["--access", "read-only"]).unwrap().access,
            Access::ReadOnly
        );
        assert!(parse(&["--access", "unknown"]).is_err());
        assert!(parse(&["--approval-input", "answers.json"]).is_err());
        let common = [
            "--instruction",
            "edit",
            "--context",
            "ctx.json",
            "--record",
            "record.json",
            "--approval-input",
            "answers.json",
        ];
        assert!(parse(&common).is_ok());
        let mut bad = common.to_vec();
        bad.extend(["--access", "auto"]);
        assert!(parse(&bad).is_err());
        assert_eq!(manual_context().budget.max_output_tokens, 8192);
    }

    #[test]
    fn tools_default_to_bash() {
        let args = parse(&["--instruction", "hi"]).unwrap();
        assert_eq!(tool_names(&args), ["bash"]);
    }

    #[test]
    fn tools_flag_chooses_the_toolset() {
        let args = parse(&["--tools", "bash,read_file", "--instruction", "hi"]).unwrap();
        assert_eq!(tool_names(&args), ["bash", "read_file"]);
        let args = parse(&["--tools", "read_file"]).unwrap();
        assert_eq!(tool_names(&args), ["read_file"]);
    }

    #[test]
    fn tools_flag_rejects_unknown_names() {
        let err = parse(&["--tools", "bash,unknown"]).err().unwrap();
        assert!(err.contains("unknown tool: unknown"), "{err}");
    }

    #[test]
    fn env_is_on_by_default() {
        assert!(parse(&[]).unwrap().env);
        assert!(parse(&["--env"]).unwrap().env);
        assert!(!parse(&["--no-env"]).unwrap().env);
    }

    #[test]
    fn context_file_flag_takes_a_name() {
        assert_eq!(parse(&[]).unwrap().context_file.as_deref(), Some("HEL.md"));
        let args = parse(&["--context-file", "NOTES.md"]).unwrap();
        assert_eq!(args.context_file.as_deref(), Some("NOTES.md"));
        assert_eq!(parse(&["--no-context-file"]).unwrap().context_file, None);
        assert!(parse(&["--context-file"]).is_err());
    }

    #[test]
    fn turns_file_replaces_instruction_for_session_runs() {
        let args = parse(&["--turns-file", "turns.json", "--context", "c.json"]).unwrap();
        assert_eq!(args.turns_file, Some(PathBuf::from("turns.json")));
        let err = parse(&["--instruction", "hi", "--turns-file", "t.json"])
            .err()
            .unwrap();
        assert!(
            err.contains("either --instruction or --turns-file"),
            "{err}"
        );
        assert!(parse(&["--context", "c.json"]).is_err());
    }

    #[test]
    fn reads_a_json_list_of_turns() {
        let dir = env::temp_dir().join(format!("hel-turns-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("turns.json");

        fs::write(&path, r#"["Read a.txt", "What was in it?"]"#).unwrap();
        assert_eq!(
            read_turns(&path).unwrap(),
            ["Read a.txt", "What was in it?"]
        );

        for bad in ["[]", r#"["ok", " "]"#, r#"{"turns": []}"#] {
            fs::write(&path, bad).unwrap();
            assert!(read_turns(&path).is_err(), "{bad}");
        }
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn compaction_flags_go_together() {
        assert_eq!(parse(&[]).unwrap().compaction, Compaction::Default);
        assert_eq!(
            parse(&["--no-compaction"]).unwrap().compaction,
            Compaction::Off
        );
        let args = parse(&["--compact-at", "4000", "--keep-recent", "1500"]).unwrap();
        assert_eq!(
            args.compaction,
            Compaction::Custom(context::Policy {
                at: 4000,
                keep_recent: 1500
            })
        );
        assert!(parse(&["--no-compaction", "--compact-at", "2", "--keep-recent", "1"]).is_err());
        assert_eq!(
            Compaction::Default.policy(8192),
            Some(context::Policy::deepseek_default(
                context::DEFAULT_WINDOW,
                8192
            ))
        );
        assert_eq!(Compaction::Off.policy(8192), None);
        assert!(parse(&["--compact-at", "4000"]).is_err());
        assert!(parse(&["--compact-at", "1000", "--keep-recent", "1000"]).is_err());
        assert!(parse(&["--compact-at", "0", "--keep-recent", "0"]).is_err());
    }
}
