//! hel — the harness built in this project.
//!
//! H0: a model ↔ tool loop with one read-only tool (`read_file`). The model is called
//! repeatedly; each tool call it makes is executed and its result sent back, until the model
//! answers without a tool call or the turn budget runs out.
//!
//! Usage:
//!   hel
//!   hel --instruction <TEXT> [--context <run-context.json> --record <record.json>]
//!
//! Without `--instruction`, hel runs interactively: each line typed is sent to the model with the
//! conversation so far. `--context` and `--record` are used by the eval runner. Without them, hel
//! only prints the answer.

mod api;
mod output;
mod tools;

use std::env;
use std::error::Error;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::{Duration, Instant, SystemTime};

use record::{
    Budget, ComparisonClass, Harness, ModelRequest, RunContext, RunInfo, Termination, ToolCategory,
    ToolEvent,
};
use serde_json::{Value, json};

use output::RunLog;

struct Args {
    instruction: Option<String>,
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
    let tool_defs = tools::definitions();

    let Some(instruction) = &args.instruction else {
        return chat(&client, &workdir, &tool_defs, ctx.budget.max_turns);
    };

    let started = SystemTime::now();
    let clock = Instant::now();
    let mut log = RunLog::new();

    let mut messages = vec![json!({ "role": "user", "content": instruction })];
    run_loop(
        &client,
        &workdir,
        &tool_defs,
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
    tool_defs: &Value,
    max_turns: u32,
) -> Result<(), Box<dyn Error>> {
    let stdin = io::stdin();
    let mut messages = Vec::new();
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
        run_loop(
            client,
            workdir,
            tool_defs,
            &mut messages,
            max_turns,
            &mut log,
        );

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
    }
}

/// Calls the model until it answers without a tool call, an error occurs, or `max_turns`
/// model calls have been made.
fn run_loop(
    client: &api::Client,
    workdir: &Path,
    tool_defs: &Value,
    messages: &mut Vec<Value>,
    max_turns: u32,
    log: &mut RunLog,
) {
    for _ in 0..max_turns {
        let exchange = match client.complete(messages, Some(tool_defs)) {
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
            let result = tools::execute(workdir, name, &args);
            log.events.push(ToolEvent {
                seq: log.events.len() as u32 + 1,
                category: if name == tools::READ_FILE {
                    ToolCategory::Read
                } else {
                    ToolCategory::Other
                },
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
    let mut context = None;
    let mut record = None;
    while let Some(flag) = args.next() {
        let mut value = || args.next().ok_or(format!("{flag} needs a value"));
        match flag.as_str() {
            "--instruction" => instruction = Some(value()?),
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
    Ok(Args {
        instruction,
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
