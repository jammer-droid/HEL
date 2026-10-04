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
    model_calls: u32,
    actual_model: Option<String>,
    pub events: Vec<ToolEvent>,
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
            model_calls: 0,
            actual_model: None,
            events: Vec::new(),
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
            }
            None => self.usage_complete = false,
        }
        if self.actual_model.is_none() {
            self.actual_model = Some(exchange.response.model.clone());
        }
        self.raw.push(json!({
            "request": exchange.request,
            "response": exchange.response_json,
        }));
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
        self.raw.push(json!({ "error": err.to_string() }));
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
            Metric {
                value: None,
                status: MetricStatus::Unavailable,
            }
        }
    }
}

fn measured(value: f64) -> Metric {
    Metric {
        value: Some(value),
        status: MetricStatus::Measured,
    }
}
