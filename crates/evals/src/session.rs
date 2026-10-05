//! H9 measurement driver: one continuous process, or two processes with/without resume.
//! This never reconstructs messages for hel. Only hel's public resume CLI may restore them.

use std::error::Error;
use std::fs;
use std::io::Write;
use std::path::Path;
use std::time::{Duration, Instant};

use record::{Metric, MetricStatus, Record, Termination};
use serde_json::{Value, json};

use crate::hel;
use crate::runner::RunJob;

pub fn run(job: &RunJob) -> Result<Record, Box<dyn Error>> {
    let mode = job.condition.settings["session_mode"]
        .as_str()
        .filter(|v| matches!(*v, "continuous" | "restart" | "resume"))
        .ok_or("settings.session_mode must be continuous, restart, or resume")?;
    // This bounded driver measures recall, not arbitrary tool loops or retry policies.
    if job.turns.len() != 2
        || job.ctx.budget.max_turns != 1
        || job.ctx.model.params["tool_choice"] != "none"
        || job.condition.settings["tools"] != json!(["bash"])
        || job.condition.settings["access"] != "read-only"
        || job.condition.settings["compaction"] != false
    {
        return Err("session_mode requires two turns, max_turns=1, tool_choice=none, tools=[bash], access=read-only, compaction=false".into());
    }
    let budget = Duration::from_secs(job.ctx.budget.timeout_seconds);
    let clock = Instant::now();
    fs::write(
        job.run_dir.join("raw/turns.json"),
        serde_json::to_string_pretty(job.turns)? + "\n",
    )?;
    if mode == "continuous" {
        let mut record = hel::run_process(job, &[], Some(budget))?;
        check_calls(&mut record, 2);
        return Ok(record);
    }
    let store = job.workdir.join(".hel/sessions");
    if mode == "resume" && store.exists() {
        return Err("resume measurement requires a fresh fixture without saved sessions".into());
    }
    let mut records = Vec::new();
    for (index, instruction) in job.turns.iter().enumerate() {
        let extra = if index == 1 && mode == "resume" {
            match only_session_id(&store) {
                Ok(id) => vec!["--resume".to_string(), id],
                Err(error) => {
                    let previous = records.last_mut().unwrap();
                    fail(previous, format!("cannot resume first stage: {error}"));
                    break;
                }
            }
        } else {
            Vec::new()
        };
        let remaining = budget.saturating_sub(clock.elapsed());
        if remaining.is_zero() {
            if let Some(previous) = records.last_mut() {
                fail(
                    previous,
                    "session timeout before second process".to_string(),
                );
                previous.outcome.termination = Termination::Timeout;
                break;
            }
            return Err("session timeout budget must be positive".into());
        }
        let run_dir = job.run_dir.join(format!("stages/{:02}", index + 1));
        fs::create_dir_all(run_dir.join("raw"))?;
        let mut ctx = job.ctx.clone();
        // The outer deadline is precise; the API timeout field accepts whole seconds.
        ctx.budget.timeout_seconds = remaining.as_secs().max(1);
        let stage = RunJob {
            ctx,
            instruction,
            turns: &[],
            hel: job.hel,
            condition: job.condition,
            run_dir,
            workdir: job.workdir.clone(),
            api_key: job.api_key,
        };
        let mut record = hel::run_process(&stage, &extra, Some(remaining))?;
        check_calls(&mut record, 1);
        fs::write(
            stage.run_dir.join("record.json"),
            serde_json::to_string_pretty(&record)? + "\n",
        )?;
        let completed =
            record.validity.valid && record.outcome.termination == Termination::Completed;
        records.push(record);
        if !completed {
            break;
        }
    }
    let mut combined = combine(&records);
    combined.usage.wall_time_ms = Metric {
        value: Some(clock.elapsed().as_secs_f64() * 1000.0),
        status: MetricStatus::Measured,
    };
    write_transcript(&job.run_dir, records.len())?;
    combined.artifacts.raw_transcript = Some("raw/requests.jsonl".to_string());
    Ok(combined)
}

fn only_session_id(store: &Path) -> Result<String, Box<dyn Error>> {
    let mut ids = Vec::new();
    for entry in fs::read_dir(store)? {
        let entry = entry?;
        if entry.file_type()?.is_dir() {
            let id = entry
                .file_name()
                .into_string()
                .map_err(|_| "non-UTF8 session ID")?;
            if id.is_empty() || !id.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-') {
                return Err("unexpected session directory name".into());
            }
            ids.push(id);
        }
    }
    if ids.len() != 1 {
        return Err(format!("expected one new session, found {}", ids.len()).into());
    }
    Ok(ids.pop().unwrap())
}

fn fail(record: &mut Record, reason: String) {
    record.outcome.final_output = None;
    record.outcome.termination = Termination::Error;
    record.outcome.error = Some(reason.clone());
    record.validity.valid = false;
    record.validity.reasons.push(reason);
}

fn check_calls(record: &mut Record, expected: usize) {
    if !record.events.is_empty() {
        fail(
            record,
            "recall measurement unexpectedly attempted a tool call".to_string(),
        );
    }
    if record.outcome.termination == Termination::Completed
        && record.usage.model_calls.value != Some(expected as f64)
    {
        fail(
            record,
            format!("recall measurement expected {expected} model calls"),
        );
    }
}

fn aggregate(metrics: impl Iterator<Item = Metric>, maximum: bool) -> Metric {
    let values: Option<Vec<f64>> = metrics.map(|m| m.value).collect();
    match values {
        Some(values) => Metric {
            value: Some(if maximum {
                values.into_iter().fold(0.0, f64::max)
            } else {
                values.iter().sum()
            }),
            status: MetricStatus::Derived,
        },
        None => Metric::unavailable(),
    }
}

fn combine(records: &[Record]) -> Record {
    let mut result = records.last().expect("at least one stage").clone();
    result.run.started_at = records[0].run.started_at.clone();
    result.validity.valid = records.iter().all(|r| r.validity.valid);
    result.validity.reasons = records
        .iter()
        .flat_map(|r| r.validity.reasons.clone())
        .collect();
    result.events = records
        .iter()
        .flat_map(|r| r.events.clone())
        .enumerate()
        .map(|(i, mut event)| {
            event.seq = i as u32 + 1;
            event
        })
        .collect();
    result.usage.input_tokens =
        aggregate(records.iter().map(|r| r.usage.input_tokens.clone()), false);
    result.usage.output_tokens =
        aggregate(records.iter().map(|r| r.usage.output_tokens.clone()), false);
    result.usage.model_calls =
        aggregate(records.iter().map(|r| r.usage.model_calls.clone()), false);
    result.usage.cached_input_tokens = aggregate(
        records.iter().map(|r| r.usage.cached_input_tokens.clone()),
        false,
    );
    result.usage.peak_context_tokens = aggregate(
        records.iter().map(|r| r.usage.peak_context_tokens.clone()),
        true,
    );
    // last_context_tokens belongs to the final process; it is not summed.
    result
}

fn write_transcript(run_dir: &Path, stages: usize) -> Result<(), Box<dyn Error>> {
    let mut output = fs::File::create(run_dir.join("raw/requests.jsonl"))?;
    let mut permissions = fs::File::create(run_dir.join("raw/permissions.jsonl"))?;
    let mut seq = 0_u64;
    for stage in 1..=stages {
        let dir = run_dir.join(format!("stages/{stage:02}/raw"));
        for line in read_lines(&dir.join("requests.jsonl"))? {
            let mut entry: Value = serde_json::from_str(&line)?;
            entry["turn"] = json!(stage);
            entry["process"] = json!(stage);
            writeln!(output, "{entry}")?;
        }
        for line in read_lines(&dir.join("permissions.jsonl"))? {
            let mut entry: Value = serde_json::from_str(&line)?;
            seq += 1;
            entry["seq"] = json!(seq);
            entry["process"] = json!(stage);
            writeln!(permissions, "{entry}")?;
        }
    }
    Ok(())
}

fn read_lines(path: &Path) -> Result<Vec<String>, Box<dyn Error>> {
    match fs::read_to_string(path) {
        Ok(text) => Ok(text.lines().map(str::to_string).collect()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(e) => Err(e.into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{check, runner, spec};
    use std::os::unix::fs::PermissionsExt;

    fn measured(
        mode: &str,
        config: Value,
        budget: u64,
    ) -> (std::path::PathBuf, Record, check::Judged) {
        let repo = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let dir = std::env::temp_dir().join(format!("evals-session-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(dir.join("fixture")).unwrap();
        fs::write(dir.join("fixture/fake-config.json"), config.to_string()).unwrap();
        let fake = dir.join("fake-hel");
        fs::write(&fake, include_str!("../testdata/fake_hel.py")).unwrap();
        fs::set_permissions(&fake, fs::Permissions::from_mode(0o755)).unwrap();
        fs::write(
            dir.join("template.json"),
            include_str!("../../../evals/schema/examples/record-v0.json"),
        )
        .unwrap();
        let hel = hel::HelBinary {
            path: fake,
            version: "offline-test".to_string(),
            installed: false,
        };
        // Keep Track E tests independent of the separately committed Lab definition.
        let plan = spec::Plan {
            experiment: "offline".into(),
            lab: "h09".into(),
            model: spec::LabModel {
                provider: "deepseek".into(),
                id: "deepseek-flash".into(),
                params: json!({"tools": [], "tool_choice": "none"}),
            },
            conditions: Vec::new(),
            tasks: vec!["session-resume-01".into()],
            repetitions: 3,
            budget: record::Budget {
                max_turns: 1,
                timeout_seconds: budget,
                max_output_tokens: 8192,
            },
            results: dir.join("results"),
            rubric: Default::default(),
        };
        let condition = spec::Condition {
            name: if mode == "restart" {
                "baseline"
            } else {
                "variant"
            }
            .into(),
            harness: "hel".into(),
            optional: false,
            settings: json!({"session_mode": mode, "tools": ["bash"], "access": "read-only", "compaction": false, "env": false, "context_file": false}),
        };
        let task = spec::load_task(&repo, "session-resume-01").unwrap();
        let run_dir = dir.join("run");
        let run_id = format!("offline-{}", uuid::Uuid::new_v4());
        let spec = runner::RunSpec {
            run_id,
            experiment: plan.experiment.clone(),
            repetition: 2,
            task: &task,
            condition: &condition,
            instruction: "",
            turns: &task.turns,
            fixture: dir.join("fixture"),
            run_dir: run_dir.clone(),
            overridden: false,
        };
        let record = runner::execute("offline-not-a-key", &plan, &spec, Some(&hel)).unwrap();
        let judged = check::check_run(&repo, &run_dir, &task).unwrap();
        // Own fixture copy only; the evidence used by assertions remains under `dir`.
        fs::remove_dir_all(std::env::temp_dir().join("hel-lab").join(&spec.run_id)).unwrap();
        (dir, record, judged)
    }

    #[test]
    fn public_driver_separates_processes_and_only_resume_restores_history() {
        for mode in ["continuous", "restart", "resume"] {
            let (dir, record, judged) = measured(mode, json!({}), 10);
            let run = dir.join("run");
            assert!(
                record.validity.valid,
                "{mode}: {:?}",
                record.validity.reasons
            );
            assert_eq!(record.usage.model_calls.value, Some(2.0));
            assert_eq!(
                judged.verdict.overall,
                if mode == "restart" { "fail" } else { "pass" }
            );
            let entries: Vec<Value> = read_lines(&run.join("raw/requests.jsonl"))
                .unwrap()
                .into_iter()
                .map(|line| serde_json::from_str(&line).unwrap())
                .collect();
            assert_eq!(entries.len(), 2);
            let expected = if mode == "restart" { 1 } else { 3 };
            assert_eq!(
                entries[1]["request"]["messages"].as_array().unwrap().len(),
                expected
            );
            let instruction = entries[1]["request"]["messages"]
                .as_array()
                .unwrap()
                .last()
                .unwrap()["content"]
                .as_str()
                .unwrap();
            assert!(!instruction.contains("HEL-"));
            for entry in entries {
                assert_eq!(entry["request"]["tools"], json!([]));
                assert_eq!(entry["request"]["tool_choice"], "none");
            }
            if mode != "continuous" {
                let first: Value = serde_json::from_str(
                    &fs::read_to_string(run.join("stages/01/raw/invocation.json")).unwrap(),
                )
                .unwrap();
                let second: Value = serde_json::from_str(
                    &fs::read_to_string(run.join("stages/02/raw/invocation.json")).unwrap(),
                )
                .unwrap();
                assert_ne!(first["pid"], second["pid"]);
                assert_eq!(second["args"].get("--resume").is_some(), mode == "resume");
                assert_eq!(record.usage.input_tokens.value, Some(300.0));
                assert_eq!(record.usage.input_tokens.status, MetricStatus::Derived);
                assert_eq!(record.usage.peak_context_tokens.value, Some(200.0));
                assert_eq!(record.usage.last_context_tokens.value, Some(200.0));
            }
            fs::remove_dir_all(dir).unwrap();
        }
    }

    #[test]
    fn first_stage_failure_or_missing_snapshot_never_launches_followup() {
        for config in [
            json!({"fail_first": true}),
            json!({"no_session": true}),
            json!({"exit_failure": true}),
        ] {
            let (dir, record, judged) = measured("resume", config, 10);
            assert_eq!(record.outcome.termination, Termination::Error);
            let error = record.outcome.error.as_deref().unwrap();
            assert!(
                error == "fake first failure"
                    || error == "session process exited unsuccessfully"
                    || error.starts_with("cannot resume first stage:"),
                "{error}"
            );
            assert_eq!(record.usage.model_calls.value, Some(1.0));
            assert!(!dir.join("run/stages/02").exists());
            assert_ne!(judged.verdict.overall, "pass");
            assert!(record.outcome.final_output.is_none());
            fs::remove_dir_all(dir).unwrap();
        }
    }

    #[test]
    fn restart_shares_one_deadline_and_preserves_unavailable_usage() {
        let (dir, record, judged) = measured("restart", json!({"delay": 0.7}), 1);
        assert_eq!(record.outcome.termination, Termination::Timeout);
        assert_eq!(judged.verdict.overall, "invalid");
        assert!(record.usage.wall_time_ms.value.unwrap() < 1800.0);
        fs::remove_dir_all(dir).unwrap();
        let (dir, record, _) = measured("restart", json!({"missing_usage": true}), 10);
        assert_eq!(record.usage.input_tokens, Metric::unavailable());
        fs::remove_dir_all(dir).unwrap();
    }
}
