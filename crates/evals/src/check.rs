//! Validate records against the schema and judge them with task checks (SPEC §4.1, §5, §6).

use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};

use record::Record;
use serde::Serialize;
use serde_json::Value;

use crate::runner::RunSidecar;
use crate::spec::{CheckKind, Plan, Task};

#[derive(Debug, Serialize)]
pub struct Verdict {
    pub schema_version: String,
    pub run_id: String,
    pub checks: Vec<CheckResult>,
    /// `pass`, `fail` or `invalid`.
    pub overall: String,
}

#[derive(Debug, Serialize)]
pub struct CheckResult {
    pub id: String,
    /// `pass`, `fail` or `n/a`.
    pub result: String,
    pub detail: String,
}

/// A judged run, used by the report.
pub struct Judged {
    pub record: Option<Record>,
    pub condition: String,
    pub task_id: String,
    pub run_id: String,
    pub overridden: bool,
    pub verdict: Verdict,
}

/// Result of `check_all`: judged runs of the plan's experiment, plus other experiments found.
pub struct Checked {
    pub judged: Vec<Judged>,
    pub other_experiments: Vec<String>,
}

fn validator(root: &Path) -> Result<jsonschema::Validator, Box<dyn Error>> {
    let schema: Value = serde_json::from_str(&fs::read_to_string(
        root.join("evals/schema/record.schema.json"),
    )?)?;
    Ok(jsonschema::validator_for(&schema)?)
}

pub fn check_all(root: &Path, plan: &Plan, tasks: &[Task]) -> Result<Checked, Box<dyn Error>> {
    let validator = validator(root)?;
    let results_dir = root.join(&plan.results);
    let mut other_experiments = Vec::new();
    if !results_dir.exists() {
        return Ok(Checked {
            judged: Vec::new(),
            other_experiments,
        });
    }
    let mut run_dirs: Vec<PathBuf> = fs::read_dir(&results_dir)?
        .filter_map(|entry| entry.ok().map(|e| e.path()))
        .filter(|path| path.join("record.json").exists() && path.join("run.json").exists())
        .collect();
    run_dirs.sort();

    let mut judged = Vec::new();
    for run_dir in run_dirs {
        let raw: Value = serde_json::from_str(&fs::read_to_string(run_dir.join("record.json"))?)?;
        let experiment = raw["run"]["experiment_id"].as_str().unwrap_or_default();
        if experiment != plan.experiment {
            if !other_experiments.iter().any(|e| e == experiment) {
                other_experiments.push(experiment.to_string());
            }
            continue;
        }
        let sidecar: RunSidecar =
            serde_json::from_str(&fs::read_to_string(run_dir.join("run.json"))?)?;
        let Some(task) = tasks.iter().find(|t| t.id == sidecar.task_id) else {
            continue;
        };
        judged.push(judge_raw(&validator, &run_dir, raw, sidecar, task)?);
    }
    Ok(Checked {
        judged,
        other_experiments,
    })
}

/// Validates and judges one run directory (used by `evals try`).
pub fn check_run(root: &Path, run_dir: &Path, task: &Task) -> Result<Judged, Box<dyn Error>> {
    let validator = validator(root)?;
    let raw: Value = serde_json::from_str(&fs::read_to_string(run_dir.join("record.json"))?)?;
    let sidecar: RunSidecar = serde_json::from_str(&fs::read_to_string(run_dir.join("run.json"))?)?;
    judge_raw(&validator, run_dir, raw, sidecar, task)
}

fn judge_raw(
    validator: &jsonschema::Validator,
    run_dir: &Path,
    raw: Value,
    sidecar: RunSidecar,
    task: &Task,
) -> Result<Judged, Box<dyn Error>> {
    let schema_errors: Vec<String> = validator.iter_errors(&raw).map(|e| e.to_string()).collect();
    let record: Option<Record> = serde_json::from_value(raw).ok();

    let verdict = match (&record, schema_errors.is_empty()) {
        (Some(record), true) if record.validity.valid => judge(record, task, &sidecar),
        (Some(record), true) => invalid(&sidecar.run_id, record.validity.reasons.join("; ")),
        _ => invalid(
            &sidecar.run_id,
            format!("schema: {}", schema_errors.join("; ")),
        ),
    };
    fs::write(
        run_dir.join("verdict.json"),
        serde_json::to_string_pretty(&verdict)? + "\n",
    )?;
    Ok(Judged {
        record,
        condition: sidecar.condition,
        task_id: sidecar.task_id,
        run_id: sidecar.run_id,
        overridden: sidecar.overridden,
        verdict,
    })
}

fn invalid(run_id: &str, detail: String) -> Verdict {
    Verdict {
        schema_version: "verdict-v0".to_string(),
        run_id: run_id.to_string(),
        checks: vec![CheckResult {
            id: "validity".to_string(),
            result: "n/a".to_string(),
            detail,
        }],
        overall: "invalid".to_string(),
    }
}

fn judge(record: &Record, task: &Task, sidecar: &RunSidecar) -> Verdict {
    let checks: Vec<CheckResult> = task
        .checks
        .iter()
        .map(|check| {
            if check.not_applicable.contains(&sidecar.condition) {
                return CheckResult {
                    id: check.id.clone(),
                    result: "n/a".to_string(),
                    detail: format!("not applicable to {}", sidecar.condition),
                };
            }
            let (pass, detail) = match &check.kind {
                CheckKind::OutputExactMatch { expected_file } => {
                    output_exact_match(record, &task.dir.join(expected_file))
                }
                CheckKind::ToolCalls {
                    category,
                    count,
                    path,
                } => tool_calls(record, *category, *count, path.as_deref(), &sidecar.workdir),
            };
            CheckResult {
                id: check.id.clone(),
                result: if pass { "pass" } else { "fail" }.to_string(),
                detail,
            }
        })
        .collect();

    let overall = if checks.iter().all(|c| c.result != "fail") {
        "pass"
    } else {
        "fail"
    };
    Verdict {
        schema_version: "verdict-v0".to_string(),
        run_id: sidecar.run_id.clone(),
        checks,
        overall: overall.to_string(),
    }
}

/// Final output equals the expected file, ignoring at most one trailing newline on each side.
fn output_exact_match(record: &Record, expected_file: &Path) -> (bool, String) {
    let expected = match fs::read_to_string(expected_file) {
        Ok(text) => text,
        Err(e) => {
            return (
                false,
                format!("cannot read {}: {e}", expected_file.display()),
            );
        }
    };
    let Some(actual) = &record.outcome.final_output else {
        return (false, "no final output".to_string());
    };
    let (a, e) = (strip_one_newline(actual), strip_one_newline(&expected));
    if a == e {
        return (true, String::new());
    }
    (false, line_diff(e, a))
}

/// First differing line, with spaces shown as `·` and tabs as `→` so invisible differences show.
fn line_diff(expected: &str, actual: &str) -> String {
    let (exp, act): (Vec<&str>, Vec<&str>) =
        (expected.split('\n').collect(), actual.split('\n').collect());
    let line = exp
        .iter()
        .zip(act.iter())
        .position(|(e, a)| e != a)
        .unwrap_or(exp.len().min(act.len()));
    let show = |lines: &[&str]| match lines.get(line) {
        Some(text) => visible(text),
        None => "(missing)".to_string(),
    };
    format!(
        "line {} differs (expected {} lines, got {})\nexpected: {}\nactual:   {}",
        line + 1,
        exp.len(),
        act.len(),
        show(&exp),
        show(&act)
    )
}

fn visible(text: &str) -> String {
    text.replace(' ', "·").replace('\t', "→")
}

fn strip_one_newline(text: &str) -> &str {
    text.strip_suffix("\r\n")
        .or_else(|| text.strip_suffix('\n'))
        .unwrap_or(text)
}

/// Exactly `count` calls in `category`; with `path`, every such call targets that file.
fn tool_calls(
    record: &Record,
    category: record::ToolCategory,
    count: usize,
    path: Option<&str>,
    workdir: &Path,
) -> (bool, String) {
    let calls: Vec<_> = record
        .events
        .iter()
        .filter(|e| e.category == category)
        .collect();
    if calls.len() != count {
        return (
            false,
            format!("{category:?} calls: {} (expected {count})", calls.len()),
        );
    }
    if let Some(expected) = path {
        let expected_path = canonical(&workdir.join(expected));
        for call in &calls {
            // Harnesses name the path argument differently (hel: path, Claude Code: file_path).
            let arg = ["path", "file_path"]
                .iter()
                .find_map(|key| call.args.get(key).and_then(Value::as_str));
            let Some(arg) = arg else {
                return (false, format!("call {} has no path argument", call.seq));
            };
            let target = Path::new(arg);
            let resolved = if target.is_absolute() {
                canonical(target)
            } else {
                canonical(&workdir.join(target))
            };
            if resolved != expected_path {
                return (
                    false,
                    format!("call {} read {arg}, expected {expected}", call.seq),
                );
            }
        }
    }
    let target = path.map(|p| format!(", {p}")).unwrap_or_default();
    (
        true,
        format!(
            "{} calls: {count}{target}",
            format!("{category:?}").to_lowercase()
        ),
    )
}

fn canonical(path: &Path) -> PathBuf {
    fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}
