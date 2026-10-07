//! Collects what happened during a run and writes `record.json` and `raw/requests.jsonl`.

use std::error::Error;
use std::fs;
use std::io::Write;
use std::path::Path;
use std::time::SystemTime;

use record::{
    Artifacts, Metric, MetricStatus, Model, Outcome, Record, Run, RunContext, Termination,
    ToolEvent, Usage, Validity,
};
use serde_json::{Value, json};

use crate::api::{ApiError, Exchange};

const RAW_FILE: &str = "raw/requests.jsonl";

pub struct RunLog {
    raw: Vec<Value>,
    input_tokens: u64,
    output_tokens: u64,
    usage_complete: bool,
    /// Prompt tokens and cache hits of each successful call, in order.
    contexts: Vec<Context>,
    /// Cache hits of the harness's own requests (e.g. compaction summaries); counted in the
    /// run's cost but not in the conversation's context sizes. `None` once one was not reported.
    aux_cache_hits: Option<u64>,
    model_calls: u32,
    actual_model: Option<String>,
    pub events: Vec<ToolEvent>,
    pub permissions: Vec<crate::permissions::Trace>,
    /// Session turn (1-based) written next to each raw entry; `None` outside session runs.
    pub turn: Option<u32>,
    /// `"child"` for a subagent's log (H11), written next to each raw entry.
    pub agent: Option<&'static str>,
    /// One entry per `delegate_task` call (H11), written to `raw/delegations.jsonl`.
    pub delegations: Vec<Value>,
    pub final_output: Option<String>,
    pub termination: Termination,
    pub error: Option<String>,
}

impl RunLog {
    pub fn new() -> Self {
        Self {
            raw: Vec::new(),
            input_tokens: 0,
            output_tokens: 0,
            usage_complete: true,
            contexts: Vec::new(),
            aux_cache_hits: Some(0),
            model_calls: 0,
            actual_model: None,
            events: Vec::new(),
            permissions: Vec::new(),
            turn: None,
            agent: None,
            delegations: Vec::new(),
            final_output: None,
            termination: Termination::Completed,
            error: None,
        }
    }

    /// Records a successful model call.
    pub fn exchange(&mut self, exchange: &Exchange) {
        self.model_calls += 1;
        match &exchange.response.usage {
            Some(usage) => {
                self.input_tokens += usage.prompt_tokens;
                self.output_tokens += usage.completion_tokens;
                self.contexts.push(Context {
                    tokens: usage.prompt_tokens,
                    cache_hit: usage.prompt_cache_hit_tokens,
                });
            }
            None => self.usage_complete = false,
        }
        if self.actual_model.is_none() {
            self.actual_model = Some(exchange.response.model.clone());
        }
        let mut entry = json!({
            "request": exchange.request,
            "response": exchange.response_json,
        });
        self.tag(&mut entry);
        self.raw.push(entry);
    }

    /// Records a request the harness made for itself, tagged with `purpose` in the raw log.
    /// It counts toward model calls and tokens, not toward the conversation's context sizes.
    pub fn auxiliary(&mut self, exchange: &Exchange, purpose: &str) {
        self.model_calls += 1;
        match &exchange.response.usage {
            Some(usage) => {
                self.input_tokens += usage.prompt_tokens;
                self.output_tokens += usage.completion_tokens;
                self.aux_cache_hits = self
                    .aux_cache_hits
                    .zip(usage.prompt_cache_hit_tokens)
                    .map(|(a, b)| a + b);
            }
            None => self.usage_complete = false,
        }
        let mut entry = json!({
            "purpose": purpose,
            "request": exchange.request,
            "response": exchange.response_json,
        });
        self.tag(&mut entry);
        self.raw.push(entry);
    }

    /// A failed auxiliary request: logged, but the run goes on.
    pub fn auxiliary_failed(&mut self, purpose: &str, error: &str) {
        self.model_calls += 1;
        self.usage_complete = false;
        let mut entry = json!({ "purpose": purpose, "error": error });
        self.tag(&mut entry);
        self.raw.push(entry);
    }

    fn tag(&self, entry: &mut Value) {
        if let Some(turn) = self.turn {
            entry["turn"] = json!(turn);
        }
        if let Some(agent) = self.agent {
            entry["agent"] = json!(agent);
        }
    }

    /// Adds a finished child's requests to this run (H11): its tokens, calls and cache hits count
    /// toward the run's cost, its raw entries keep their `"agent": "child"` tag, and its context
    /// sizes and tool events stay out of the parent's conversation measures.
    pub fn absorb_child(&mut self, child: &RunLog) {
        self.input_tokens += child.input_tokens;
        self.output_tokens += child.output_tokens;
        self.usage_complete &= child.usage_complete;
        self.model_calls += child.model_calls;
        let child_hits: Option<u64> = child.contexts.iter().map(|c| c.cache_hit).sum();
        self.aux_cache_hits = self
            .aux_cache_hits
            .zip(child_hits)
            .zip(child.aux_cache_hits)
            .map(|((a, b), c)| a + b + c);
        self.raw.extend(child.raw.iter().cloned());
    }

    /// Prompt and cache-hit tokens of each successful call, for the delegation trace.
    pub fn requests(&self) -> Vec<Value> {
        self.contexts
            .iter()
            .map(|c| json!({ "prompt_tokens": c.tokens, "cache_hit_tokens": c.cache_hit }))
            .collect()
    }

    /// Why the last compaction result was not used, attached to its raw entry.
    pub fn compaction_note(&mut self, reason: &str) {
        if let Some(entry) = self.raw.last_mut() {
            entry["skipped"] = json!(reason);
        }
    }

    /// Records a failed model call and ends the run.
    pub fn failed(&mut self, err: &ApiError) {
        self.model_calls += 1;
        self.usage_complete = false;
        self.termination = match err {
            ApiError::Timeout(_) => Termination::Timeout,
            ApiError::Http(_) => Termination::Error,
        };
        self.error = Some(err.to_string());
        let mut entry = json!({ "error": err.to_string() });
        self.tag(&mut entry);
        self.raw.push(entry);
    }

    /// One line describing the context of the last call, e.g.
    /// `context: 4,591 tokens · cache hit 4,352 (95%)`. `None` before any successful call.
    pub fn context_summary(&self) -> Option<String> {
        let last = self.contexts.last()?;
        let mut line = format!("context: {} tokens", thousands(last.tokens));
        if let Some(hit) = last.cache_hit {
            let percent = (hit * 100).checked_div(last.tokens).unwrap_or(0);
            line.push_str(&format!(" · cache hit {} ({percent}%)", thousands(hit)));
        }
        Some(line)
    }

    pub fn write(
        &self,
        ctx: &RunContext,
        started: SystemTime,
        ended: SystemTime,
        wall_time_ms: u128,
        record_path: &Path,
    ) -> Result<(), Box<dyn Error>> {
        let run_dir = record_path.parent().unwrap_or(Path::new("."));
        fs::create_dir_all(run_dir.join("raw"))?;
        let mut raw = fs::File::create(run_dir.join(RAW_FILE))?;
        for entry in &self.raw {
            writeln!(raw, "{}", serde_json::to_string(entry)?)?;
        }

        let mut permissions = fs::File::create(run_dir.join("raw/permissions.jsonl"))?;
        for (index, entry) in self.permissions.iter().enumerate() {
            let mut value = serde_json::to_value(entry)?;
            value["seq"] = json!(index + 1);
            writeln!(permissions, "{}", serde_json::to_string(&value)?)?;
        }
        if !self.delegations.is_empty() {
            let mut delegations = fs::File::create(run_dir.join("raw/delegations.jsonl"))?;
            for entry in &self.delegations {
                writeln!(delegations, "{}", serde_json::to_string(entry)?)?;
            }
        }
        let record = self.to_record(ctx, started, ended, wall_time_ms);
        fs::write(record_path, serde_json::to_string_pretty(&record)? + "\n")?;
        Ok(())
    }

    fn to_record(
        &self,
        ctx: &RunContext,
        started: SystemTime,
        ended: SystemTime,
        wall_time_ms: u128,
    ) -> Record {
        let mut reasons = Vec::new();
        match &self.actual_model {
            Some(actual) if *actual != ctx.model.requested => reasons.push(format!(
                "model mismatch: requested {}, actual {actual}",
                ctx.model.requested
            )),
            None => reasons.push("model unknown: no successful response".to_string()),
            _ => {}
        }

        Record {
            schema_version: "record-v0".to_string(),
            run: Run {
                run_id: ctx.run.run_id.clone(),
                experiment_id: ctx.run.experiment_id.clone(),
                lab: ctx.run.lab.clone(),
                task_id: ctx.run.task_id.clone(),
                condition: ctx.run.condition.clone(),
                repetition: ctx.run.repetition,
                started_at: humantime::format_rfc3339_millis(started).to_string(),
                ended_at: humantime::format_rfc3339_millis(ended).to_string(),
            },
            harness: ctx.harness.clone(),
            model: Model {
                provider: ctx.model.provider.clone(),
                requested: ctx.model.requested.clone(),
                actual: self.actual_model.clone(),
                params: ctx.model.params.clone(),
            },
            outcome: Outcome {
                final_output: self.final_output.clone(),
                termination: self.termination,
                error: self.error.clone(),
            },
            usage: Usage {
                input_tokens: self.token_metric(self.input_tokens),
                output_tokens: self.token_metric(self.output_tokens),
                model_calls: measured(self.model_calls as f64),
                wall_time_ms: measured(wall_time_ms as f64),
                cached_input_tokens: self.cached_metric(),
                peak_context_tokens: self
                    .context_metric(self.contexts.iter().map(|c| c.tokens).max()),
                last_context_tokens: self.context_metric(self.contexts.last().map(|c| c.tokens)),
            },
            events: self.events.clone(),
            validity: Validity {
                valid: reasons.is_empty(),
                reasons,
            },
            artifacts: Artifacts {
                raw_transcript: Some(RAW_FILE.to_string()),
            },
        }
    }

    fn token_metric(&self, value: u64) -> Metric {
        if self.usage_complete {
            measured(value as f64)
        } else {
            Metric::unavailable()
        }
    }

    /// Unavailable unless every successful call reported its cache hits and none failed.
    fn cached_metric(&self) -> Metric {
        let hits: Option<u64> = self.contexts.iter().map(|c| c.cache_hit).sum();
        match hits.zip(self.aux_cache_hits).map(|(a, b)| a + b) {
            Some(hits) if self.usage_complete => measured(hits as f64),
            _ => Metric::unavailable(),
        }
    }

    fn context_metric(&self, value: Option<u64>) -> Metric {
        value.map_or_else(Metric::unavailable, |v| measured(v as f64))
    }
}

struct Context {
    tokens: u64,
    cache_hit: Option<u64>,
}

/// Formats 4591 as `4,591`.
fn thousands(value: u64) -> String {
    let digits = value.to_string();
    let mut out = String::new();
    for (i, ch) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(ch);
    }
    out
}

fn measured(value: f64) -> Metric {
    Metric {
        value: Some(value),
        status: MetricStatus::Measured,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::ChatResponse;
    use record::{Budget, ComparisonClass, Harness, ModelRequest, RunInfo};

    fn exchange(usage: Value) -> Exchange {
        let response_json = json!({
            "model": "deepseek-flash",
            "choices": [{ "message": { "role": "assistant", "content": "" }, "finish_reason": "stop" }],
            "usage": usage,
        });
        Exchange {
            request: json!({}),
            response: serde_json::from_value::<ChatResponse>(response_json.clone()).unwrap(),
            response_json,
        }
    }

    fn usage(prompt: u64, hit: Option<u64>) -> Value {
        let mut usage = json!({ "prompt_tokens": prompt, "completion_tokens": 10 });
        if let Some(hit) = hit {
            usage["prompt_cache_hit_tokens"] = json!(hit);
        }
        usage
    }

    fn record(log: &RunLog) -> Record {
        let ctx = RunContext {
            run: RunInfo {
                run_id: "r".into(),
                experiment_id: "e".into(),
                lab: "h05".into(),
                task_id: "t".into(),
                condition: "baseline".into(),
                repetition: 1,
            },
            harness: Harness {
                name: "hel".into(),
                version: "0".into(),
                comparison_class: ComparisonClass::Subject,
            },
            model: ModelRequest {
                provider: "deepseek".into(),
                requested: "deepseek-flash".into(),
                params: json!({}),
            },
            budget: Budget {
                max_turns: 5,
                timeout_seconds: 60,
                max_output_tokens: 8192,
            },
        };
        log.to_record(&ctx, SystemTime::UNIX_EPOCH, SystemTime::UNIX_EPOCH, 0)
    }

    #[test]
    fn records_cache_hits_and_context_sizes() {
        let mut log = RunLog::new();
        log.exchange(&exchange(usage(462, Some(0))));
        log.exchange(&exchange(usage(4409, Some(640))));
        log.exchange(&exchange(usage(4591, Some(4352))));

        let usage = record(&log).usage;
        assert_eq!(usage.input_tokens.value, Some(9462.0));
        assert_eq!(usage.cached_input_tokens, measured(4992.0));
        assert_eq!(usage.peak_context_tokens, measured(4591.0));
        assert_eq!(usage.last_context_tokens, measured(4591.0));
        assert_eq!(
            log.context_summary().as_deref(),
            Some("context: 4,591 tokens · cache hit 4,352 (94%)")
        );
    }

    #[test]
    fn peak_context_can_come_before_the_last_call() {
        let mut log = RunLog::new();
        log.exchange(&exchange(usage(5000, Some(0))));
        log.exchange(&exchange(usage(1200, Some(1024))));

        let usage = record(&log).usage;
        assert_eq!(usage.peak_context_tokens, measured(5000.0));
        assert_eq!(usage.last_context_tokens, measured(1200.0));
    }

    #[test]
    fn cache_hits_are_unavailable_when_a_response_omits_them() {
        let mut log = RunLog::new();
        log.exchange(&exchange(usage(462, Some(0))));
        log.exchange(&exchange(usage(900, None)));

        let usage = record(&log).usage;
        assert_eq!(usage.cached_input_tokens, Metric::unavailable());
        assert_eq!(usage.last_context_tokens, measured(900.0));
        assert_eq!(
            log.context_summary().as_deref(),
            Some("context: 900 tokens")
        );
    }

    #[test]
    fn a_failed_call_makes_cache_hits_unavailable() {
        let mut log = RunLog::new();
        log.exchange(&exchange(usage(462, Some(0))));
        log.failed(&ApiError::Timeout("slow".into()));

        let usage = record(&log).usage;
        assert_eq!(usage.cached_input_tokens, Metric::unavailable());
        assert_eq!(usage.peak_context_tokens, measured(462.0));
    }

    #[test]
    fn no_context_before_the_first_successful_call() {
        let log = RunLog::new();
        assert_eq!(log.context_summary(), None);
        assert_eq!(
            record(&log).usage.peak_context_tokens,
            Metric::unavailable()
        );
    }

    #[test]
    fn formats_thousands() {
        assert_eq!(thousands(0), "0");
        assert_eq!(thousands(999), "999");
        assert_eq!(thousands(4591), "4,591");
        assert_eq!(thousands(1_000_000), "1,000,000");
    }

    #[test]
    fn raw_entries_carry_the_session_turn() {
        let mut log = RunLog::new();
        log.exchange(&exchange(usage(462, Some(0))));
        log.turn = Some(2);
        log.exchange(&exchange(usage(900, Some(512))));
        log.failed(&ApiError::Timeout("slow".into()));

        assert_eq!(log.raw[0].get("turn"), None);
        assert_eq!(log.raw[1]["turn"], 2);
        assert_eq!(log.raw[2]["turn"], 2);
    }
}
