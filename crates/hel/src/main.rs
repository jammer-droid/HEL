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

mod api;
mod context;
mod output;
mod permissions;
mod prompt;
mod search;
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

struct Args {
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
    let args = parse_args(env::args().skip(1))?;
    // Spilled tool results from earlier runs are kept for a day, then removed here.
    context::clean_spills(
        &context::spill_root(),
        context::SPILL_RETENTION,
        SystemTime::now(),
    );
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
    let workdir = env::current_dir()?;
    let mut approval = permissions::Input::new(args.approval_input.as_deref())?;
    let system = prompt::system_message(args.env, args.context_file.as_deref(), &workdir);
    let compaction = args.compaction.policy(ctx.budget.max_output_tokens);
    let inputs: Vec<String> = match (&args.instruction, &args.turns_file) {
        (Some(instruction), _) => vec![instruction.clone()],
        (None, Some(path)) => read_turns(path)?,
        (None, None) => {
            return chat(
                &client,
                &workdir,
                &args.tools,
                system,
                ctx.budget.max_turns,
                compaction.as_ref(),
                args.access,
                &mut approval,
            );
        }
    };
    let session = args.turns_file.is_some();

    let started = SystemTime::now();
    let clock = Instant::now();
    let mut log = RunLog::new();

    let mut messages: Vec<Value> = system.into_iter().collect();
    let mut meter = context::Meter::default();
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
                workdir: &workdir,
                tools: &args.tools,
                compaction: compaction.as_ref(),
                access: args.access,
                approval: &mut approval,
            },
            &mut messages,
            &mut meter,
            ctx.budget.max_turns,
            &mut log,
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
    }
    Ok(())
}

/// Interactive mode: reads one line at a time and runs the loop on it with the conversation so
/// far. `max_turns` applies to each input. Ends on `/exit` or end of input (Ctrl-D).
#[allow(clippy::too_many_arguments)]
fn chat(
    client: &api::Client,
    workdir: &Path,
    tools: &Toolset,
    system: Option<Value>,
    max_turns: u32,
    compaction: Option<&context::Policy>,
    access: Access,
    approval: &mut dyn Approval,
) -> Result<(), Box<dyn Error>> {
    let stdin = io::stdin();
    let mut messages: Vec<Value> = system.into_iter().collect();
    let mut meter = context::Meter::default();
    let mut session = Session {
        client,
        workdir,
        tools,
        compaction,
        access,
        approval,
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
    workdir: &'a Path,
    tools: &'a Toolset,
    compaction: Option<&'a context::Policy>,
    access: Access,
    approval: &'a mut dyn Approval,
}

fn run_loop(
    session: &mut Session,
    messages: &mut Vec<Value>,
    meter: &mut context::Meter,
    max_turns: u32,
    log: &mut RunLog,
) {
    let Session {
        client,
        workdir,
        tools,
        compaction,
        access,
        approval,
    } = session;
    for _ in 0..max_turns {
        if let Some(policy) = compaction {
            context::before_request(messages, policy, meter, log, &mut |request| {
                client.complete(request, Some(tools.definitions()))
            });
        }
        let exchange = match client.complete(messages, Some(tools.definitions())) {
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

        for call in &calls {
            let id = call["id"].as_str().unwrap_or_default();
            let name = call["function"]["name"].as_str().unwrap_or_default();
            // `arguments` is a JSON object encoded as a string.
            let args: Value = call["function"]["arguments"]
                .as_str()
                .and_then(|raw| serde_json::from_str(raw).ok())
                .unwrap_or_else(|| json!({}));
            let execution = tools.call(workdir, name, &args, *access, *approval);
            log.permissions.push(execution.trace);
            let result = execution.result;
            log.events.push(ToolEvent {
                seq: log.events.len() as u32 + 1,
                category: tools::category(name),
                name: name.to_string(),
                args,
                ok: Some(result.is_ok()),
            });
            let mut content = result.unwrap_or_else(|err| format!("error: {err}"));
            if compaction.is_some() {
                content = context::spill(content, name);
            }
            messages.push(json!({ "role": "tool", "tool_call_id": id, "content": content }));
        }
    }
    log.termination = Termination::MaxTurns;
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

fn parse_args(mut args: impl Iterator<Item = String>) -> Result<Args, String> {
    let mut access = Access::Confirm;
    let mut approval_input = None;
    let mut instruction = None;
    let mut turns_file = None;
    let mut compact_at = None;
    let mut keep_recent = None;
    let mut no_compaction = false;
    let mut tools = None;
    let mut env = true;
    let mut context_file = Some(prompt::DEFAULT_CONTEXT_FILE.to_string());
    let mut context = None;
    let mut record = None;
    while let Some(flag) = args.next() {
        let mut value = || args.next().ok_or(format!("{flag} needs a value"));
        match flag.as_str() {
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
    let tools = match tools {
        Some(tools) => tools,
        None => Toolset::new(tools::DEFAULT)?,
    };
    Ok(Args {
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
