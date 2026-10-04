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
//!
//! Without `--instruction`, hel runs interactively: each line typed is sent to the model with the
//! conversation so far. `--context` and `--record` are used by the eval runner. Without them, hel
//! only prints the answer.

mod api;
mod output;
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
use tools::Toolset;

struct Args {
    instruction: Option<String>,
    tools: Toolset,
    env: bool,
    context_file: Option<String>,
    context: Option<PathBuf>,
    record: Option<PathBuf>,
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
    let system = prompt::system_message(args.env, args.context_file.as_deref(), &workdir);
    let Some(instruction) = &args.instruction else {
        return chat(&client, &workdir, &args.tools, system, ctx.budget.max_turns);
    };

    let started = SystemTime::now();
    let clock = Instant::now();
    let mut log = RunLog::new();

    let mut messages: Vec<Value> = system.into_iter().collect();
    messages.push(json!({ "role": "user", "content": instruction }));
    run_loop(
        &client,
        &workdir,
        &args.tools,
        &mut messages,
        ctx.budget.max_turns,
        &mut log,
    );

    let wall_time_ms = clock.elapsed().as_millis();
    let ended = SystemTime::now();

    if let Some(output) = &log.final_output {
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
fn chat(
    client: &api::Client,
    workdir: &Path,
    tools: &Toolset,
    system: Option<Value>,
    max_turns: u32,
) -> Result<(), Box<dyn Error>> {
    let stdin = io::stdin();
    let mut messages: Vec<Value> = system.into_iter().collect();
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
        run_loop(client, workdir, tools, &mut messages, max_turns, &mut log);

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
fn run_loop(
    client: &api::Client,
    workdir: &Path,
    tools: &Toolset,
    messages: &mut Vec<Value>,
    max_turns: u32,
    log: &mut RunLog,
) {
    for _ in 0..max_turns {
        let exchange = match client.complete(messages, Some(tools.definitions())) {
            Ok(exchange) => exchange,
            Err(err) => {
                log.failed(&err);
                return;
            }
        };
        log.exchange(&exchange);
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
            let result = tools.execute(workdir, name, &args);
            log.events.push(ToolEvent {
                seq: log.events.len() as u32 + 1,
                category: tools::category(name),
                name: name.to_string(),
                args,
                ok: Some(result.is_ok()),
            });
            let content = result.unwrap_or_else(|err| format!("error: {err}"));
            messages.push(json!({ "role": "tool", "tool_call_id": id, "content": content }));
        }
    }
    log.termination = Termination::MaxTurns;
}

fn parse_args(mut args: impl Iterator<Item = String>) -> Result<Args, String> {
    let mut instruction = None;
    let mut tools = None;
    let mut env = true;
    let mut context_file = Some(prompt::DEFAULT_CONTEXT_FILE.to_string());
    let mut context = None;
    let mut record = None;
    while let Some(flag) = args.next() {
        let mut value = || args.next().ok_or(format!("{flag} needs a value"));
        match flag.as_str() {
            "--instruction" => instruction = Some(value()?),
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
    if context.is_some() && instruction.is_none() {
        return Err("--context needs --instruction".to_string());
    }
    let tools = match tools {
        Some(tools) => tools,
        None => Toolset::new(tools::DEFAULT)?,
    };
    Ok(Args {
        instruction,
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
}
