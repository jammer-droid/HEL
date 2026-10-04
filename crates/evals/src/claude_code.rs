//! Driver and collector for Claude Code (evals/harnesses/claude-code/PROFILE.md).
//!
//! Runs the unmodified Claude Code CLI headless against DeepSeek's Anthropic-compatible
//! endpoint with an isolated config dir and home, then converts its stream-json output
//! into a record.

use std::collections::{BTreeSet, HashMap};
use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Duration;

use record::{
    Artifacts, ComparisonClass, Harness, Metric, MetricStatus, Model, Outcome, Record, Termination,
    ToolCategory, ToolEvent, Usage, Validity,
};
use serde_json::{Value, json};

use crate::runner::{self, Finished, RunJob};
use crate::spec::Condition;

const DEFAULT_BASE_URL: &str = "https://api.deepseek.com/anthropic";
const STREAM_FILE: &str = "raw/stream.jsonl";

pub fn run(job: &RunJob) -> Result<Record, Box<dyn Error>> {
    let settings = &job.condition.settings;
    let binary = binary(job.condition);
    let base_url = settings
        .get("base_url")
        .and_then(Value::as_str)
        .unwrap_or(DEFAULT_BASE_URL);
    let tools = settings
        .get("tools")
        .and_then(Value::as_array)
        .map(|tools| {
            tools
                .iter()
                .filter_map(Value::as_str)
                .collect::<Vec<_>>()
                .join(",")
        })
        .unwrap_or_else(|| "Read".to_string());

    // Isolated home and config so no user settings, plugins, memory or credentials are read.
    let scratch = std::env::temp_dir()
        .join("hel-lab")
        .join(format!("{}-claude", job.ctx.run.run_id));
    if scratch.exists() {
        fs::remove_dir_all(&scratch)?;
    }
    let home = scratch.join("home");
    let config = scratch.join("config");
    fs::create_dir_all(&home)?;
    fs::create_dir_all(&config)?;

    let session_id = uuid::Uuid::new_v4().to_string();
    let model = &job.ctx.model.requested;
    let mut command = Command::new(&binary);
    command
        .args(["-p", job.instruction])
        .args(["--bare", "--session-id", &session_id])
        .args(["--tools", &tools, "--allowedTools", &tools])
        .args(["--strict-mcp-config", "--permission-prompts", "none"])
        .args(["--output-format", "stream-json", "--verbose"])
        .current_dir(&job.workdir)
        .env_clear()
        .env("HOME", &home)
        .env("PATH", "/usr/bin:/bin:/usr/sbin:/sbin")
        .env("TMPDIR", &scratch)
        .env("ANTHROPIC_BASE_URL", base_url)
        // --bare reads only ANTHROPIC_API_KEY (x-api-key); DeepSeek accepts it.
        .env("ANTHROPIC_API_KEY", job.api_key)
        .env("ANTHROPIC_MODEL", model)
        .env("ANTHROPIC_DEFAULT_OPUS_MODEL", model)
        .env("ANTHROPIC_DEFAULT_SONNET_MODEL", model)
        .env("ANTHROPIC_DEFAULT_HAIKU_MODEL", model)
        .env("CLAUDE_CODE_SUBAGENT_MODEL", model)
        .env("CLAUDE_CONFIG_DIR", &config)
        .env("CLAUDE_CODE_PROJECT_DIR_NAME", &job.ctx.run.run_id)
        .env("CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC", "1")
        .env(
            "CLAUDE_CODE_MAX_OUTPUT_TOKENS",
            job.ctx.budget.max_output_tokens.to_string(),
        )
        .stdin(Stdio::null())
        .stdout(fs::File::create(job.run_dir.join(STREAM_FILE))?)
        .stderr(fs::File::create(job.run_dir.join("raw/stderr.txt"))?);

    let budget = Duration::from_secs(job.ctx.budget.timeout_seconds);
    let finished = runner::run_with_timeout(command, budget)?;

    // Keep the session transcript next to the stream for later inspection.
    let transcript = config
        .join("projects")
        .join(&job.ctx.run.run_id)
        .join(format!("{session_id}.jsonl"));
    if transcript.exists() {
        fs::copy(&transcript, job.run_dir.join("raw/session.jsonl"))?;
    }

    let stream = fs::read_to_string(job.run_dir.join(STREAM_FILE)).unwrap_or_default();
    Ok(collect(job, &stream, &finished))
}

/// Claude Code executable: `settings.binary`, or the default install location.
pub fn binary(condition: &Condition) -> PathBuf {
    match condition.settings.get("binary").and_then(Value::as_str) {
        Some(path) => PathBuf::from(path),
        None => {
            let home = std::env::var("HOME").unwrap_or_default();
            Path::new(&home).join(".local/bin/claude")
        }
    }
}

/// Converts Claude Code stream-json output into a record (SPEC §7.5).
pub fn collect(job: &RunJob, stream: &str, finished: &Finished) -> Record {
    let mut version = None;
    let mut message_ids = BTreeSet::new();
    let mut models = BTreeSet::new();
    let mut tool_uses: Vec<(String, String, Value)> = Vec::new();
    let mut tool_errors: HashMap<String, bool> = HashMap::new();
    let mut result: Option<Value> = None;

    for line in stream.lines() {
        let Ok(event) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        match event.get("type").and_then(Value::as_str) {
            Some("system") if event.get("subtype").and_then(Value::as_str) == Some("init") => {
                version = event
                    .get("claude_code_version")
                    .and_then(Value::as_str)
                    .map(str::to_string);
            }
            Some("assistant") => {
                let message = &event["message"];
                if let Some(id) = message.get("id").and_then(Value::as_str) {
                    message_ids.insert(id.to_string());
                }
                if let Some(model) = message.get("model").and_then(Value::as_str) {
                    models.insert(model.to_string());
                }
                for block in content_blocks(message) {
                    if block.get("type").and_then(Value::as_str) == Some("tool_use") {
                        tool_uses.push((
                            block["id"].as_str().unwrap_or_default().to_string(),
                            block["name"].as_str().unwrap_or_default().to_string(),
                            block.get("input").cloned().unwrap_or(json!({})),
                        ));
                    }
                }
            }
            Some("user") => {
                for block in content_blocks(&event["message"]) {
                    if block.get("type").and_then(Value::as_str) == Some("tool_result") {
                        let id = block["tool_use_id"].as_str().unwrap_or_default();
                        let is_error = block
                            .get("is_error")
                            .and_then(Value::as_bool)
                            .unwrap_or(false);
                        tool_errors.insert(id.to_string(), is_error);
                    }
                }
            }
            Some("result") => result = Some(event),
            _ => {}
        }
    }

    let (termination, error) = termination(result.as_ref(), finished);
    let final_output = result
        .as_ref()
        .and_then(|r| r.get("result"))
        .and_then(Value::as_str)
        .map(str::to_string);

    let usage = result.as_ref().and_then(|r| r.get("usage"));
    let input_tokens = usage.map(|u| {
        [
            "input_tokens",
            "cache_read_input_tokens",
            "cache_creation_input_tokens",
        ]
        .iter()
        .filter_map(|key| u.get(key).and_then(Value::as_f64))
        .sum::<f64>()
    });
    let output_tokens = usage.and_then(|u| u.get("output_tokens").and_then(Value::as_f64));

    let events = tool_uses
        .iter()
        .enumerate()
        .map(|(index, (id, name, input))| ToolEvent {
            seq: index as u32 + 1,
            category: category(name),
            name: name.clone(),
            args: input.clone(),
            ok: tool_errors.get(id).map(|is_error| !is_error),
        })
        .collect();

    let actual = models.iter().next().cloned();
    let mut reasons = Vec::new();
    match &actual {
        Some(actual) if *actual != job.ctx.model.requested => reasons.push(format!(
            "model mismatch: requested {}, actual {actual}",
            job.ctx.model.requested
        )),
        None => reasons.push("model unknown: no assistant message".to_string()),
        _ => {}
    }
    if models.len() > 1 {
        reasons.push(format!("multiple models in one run: {models:?}"));
    }

    let comparison_class = match job
        .condition
        .settings
        .get("comparison_class")
        .and_then(Value::as_str)
    {
        Some("reference") => ComparisonClass::Reference,
        _ => ComparisonClass::SameModel,
    };

    Record {
        schema_version: "record-v0".to_string(),
        run: runner::run_block(&job.ctx, finished),
        harness: Harness {
            name: "claude-code".to_string(),
            version: version.unwrap_or_else(|| "unknown".to_string()),
            comparison_class,
        },
        model: Model {
            provider: job.ctx.model.provider.clone(),
            requested: job.ctx.model.requested.clone(),
            actual,
            // Claude Code chooses its own request parameters; record the condition settings instead.
            params: job.condition.settings.clone(),
        },
        outcome: Outcome {
            final_output,
            termination,
            error,
        },
        usage: Usage {
            input_tokens: metric(input_tokens, MetricStatus::Measured),
            output_tokens: metric(output_tokens, MetricStatus::Measured),
            model_calls: metric(
                (!message_ids.is_empty()).then_some(message_ids.len() as f64),
                MetricStatus::Derived,
            ),
            wall_time_ms: metric(Some(finished.wall_time_ms as f64), MetricStatus::Measured),
        },
        events,
        validity: Validity {
            valid: reasons.is_empty(),
            reasons,
        },
        artifacts: Artifacts {
            raw_transcript: Some(STREAM_FILE.to_string()),
        },
    }
}

fn content_blocks(message: &Value) -> impl Iterator<Item = &Value> {
    message
        .get("content")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
}

fn termination(result: Option<&Value>, finished: &Finished) -> (Termination, Option<String>) {
    if finished.timed_out {
        return (
            Termination::Timeout,
            Some("killed by runner after timeout".to_string()),
        );
    }
    let Some(result) = result else {
        return (
            Termination::Error,
            Some(format!(
                "no result event; exit status {:?}",
                finished.status
            )),
        );
    };
    let subtype = result.get("subtype").and_then(Value::as_str).unwrap_or("");
    let stop_reason = result.get("stop_reason").and_then(Value::as_str);
    match subtype {
        "success" if stop_reason == Some("max_tokens") => (Termination::MaxOutputTokens, None),
        "success" => (Termination::Completed, None),
        s if s.contains("max_turns") => (Termination::MaxTurns, None),
        s => (Termination::Error, Some(format!("result subtype: {s}"))),
    }
}

/// Claude Code tool name → common category (PROFILE §5).
fn category(name: &str) -> ToolCategory {
    match name {
        "Read" => ToolCategory::Read,
        "Grep" | "Glob" => ToolCategory::Search,
        "Edit" | "Write" | "NotebookEdit" => ToolCategory::Edit,
        "Bash" => ToolCategory::Exec,
        _ => ToolCategory::Other,
    }
}

fn metric(value: Option<f64>, status: MetricStatus) -> Metric {
    match value {
        Some(value) => Metric {
            value: Some(value),
            status,
        },
        None => Metric {
            value: None,
            status: MetricStatus::Unavailable,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use record::{Budget, ModelRequest, RunContext, RunInfo};
    use std::time::SystemTime;

    const STREAM: &str =
        include_str!("../../../evals/harnesses/claude-code/fixtures/read-echo-01.stream.jsonl");

    /// Conformance test (SPEC §7.8): a recorded stream converts into the expected record fields.
    #[test]
    fn collects_record_from_stream() {
        let condition = Condition {
            name: "external-claude-code".to_string(),
            harness: "claude-code".to_string(),
            optional: false,
            settings: json!({ "tools": ["Read"] }),
        };
        let ctx = RunContext {
            run: RunInfo {
                run_id: "test".to_string(),
                experiment_id: "test".to_string(),
                lab: "h00".to_string(),
                task_id: "read-echo-01".to_string(),
                condition: "external-claude-code".to_string(),
                repetition: 1,
            },
            harness: Harness {
                name: "claude-code".to_string(),
                version: String::new(),
                comparison_class: ComparisonClass::SameModel,
            },
            model: ModelRequest {
                provider: "deepseek".to_string(),
                requested: "deepseek-flash".to_string(),
                params: json!({}),
            },
            budget: Budget {
                max_turns: 5,
                timeout_seconds: 120,
                max_output_tokens: 2048,
            },
        };
        let job = RunJob {
            ctx,
            instruction: "",
            hel: None,
            condition: &condition,
            run_dir: PathBuf::new(),
            workdir: PathBuf::new(),
            api_key: "",
        };
        let finished = Finished {
            status: None,
            timed_out: false,
            started: SystemTime::UNIX_EPOCH,
            ended: SystemTime::UNIX_EPOCH,
            wall_time_ms: 1000,
        };

        let record = collect(&job, STREAM, &finished);

        assert_eq!(record.harness.version, "2.1.288");
        assert_eq!(record.model.actual.as_deref(), Some("deepseek-flash"));
        assert!(record.validity.valid);
        assert_eq!(record.outcome.termination, Termination::Completed);
        assert_eq!(
            record.outcome.final_output.as_deref(),
            Some("Hello, harness!")
        );
        assert_eq!(record.usage.input_tokens.value, Some(1296.0));
        assert_eq!(record.usage.output_tokens.value, Some(189.0));
        assert_eq!(record.usage.model_calls.value, Some(2.0));
        assert_eq!(record.usage.model_calls.status, MetricStatus::Derived);
        assert_eq!(record.events.len(), 1);
        assert_eq!(record.events[0].category, ToolCategory::Read);
        assert_eq!(record.events[0].ok, Some(true));
    }
}
