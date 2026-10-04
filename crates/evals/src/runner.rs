//! Runs plan conditions × tasks × repetitions (evals/SPEC.md §8), and single runs for `evals try`.

use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus};
use std::thread;
use std::time::{Duration, Instant, SystemTime};

use record::{
    Artifacts, Metric, MetricStatus, Model, Outcome, Record, Run, RunContext, RunInfo, Termination,
    Usage, Validity,
};
use serde::{Deserialize, Serialize};

use crate::claude_code;
use crate::hel::{self, HelBinary};
use crate::spec::{Condition, Plan, Task};

/// Extra time the runner waits beyond the budget before killing a harness process.
const KILL_GRACE: Duration = Duration::from_secs(10);

/// Per-run facts the checker needs that are not part of the record (written as `run.json`).
#[derive(Debug, Serialize, Deserialize)]
pub struct RunSidecar {
    pub run_id: String,
    pub task_id: String,
    pub condition: String,
    pub harness: String,
    pub workdir: PathBuf,
    /// The instruction or fixture differed from the task's defaults (`evals try` overrides).
    #[serde(default)]
    pub overridden: bool,
}

/// Everything a driver needs to execute one run.
pub struct RunJob<'a> {
    pub ctx: RunContext,
    pub instruction: &'a str,
    pub hel: Option<&'a HelBinary>,
    pub condition: &'a Condition,
    pub run_dir: PathBuf,
    pub workdir: PathBuf,
    pub api_key: &'a str,
}

/// One run to execute.
pub struct RunSpec<'a> {
    pub run_id: String,
    pub experiment: String,
    pub repetition: u32,
    pub task: &'a Task,
    pub condition: &'a Condition,
    pub instruction: &'a str,
    pub fixture: PathBuf,
    pub run_dir: PathBuf,
    pub overridden: bool,
}

pub struct Options {
    pub conditions: Option<Vec<String>>,
    pub force: bool,
    /// Build hel from the working tree even if it is installed.
    pub build: bool,
}

pub fn api_key() -> Result<String, Box<dyn Error>> {
    std::env::var("DEEPSEEK_API_KEY").map_err(|_| "DEEPSEEK_API_KEY is not set".into())
}

pub fn run_all(
    root: &Path,
    plan: &Plan,
    tasks: &[Task],
    options: &Options,
) -> Result<(), Box<dyn Error>> {
    let api_key = api_key()?;
    let mut selected: Vec<&Condition> = Vec::new();
    for condition in &plan.conditions {
        if let Some(names) = &options.conditions
            && !names.contains(&condition.name)
        {
            continue;
        }
        if condition.optional && !harness_available(condition) {
            println!(
                "skip  condition {} ({} is not installed)",
                condition.name, condition.harness
            );
            continue;
        }
        selected.push(condition);
    }
    if selected.is_empty() {
        return Err("no conditions selected".into());
    }

    let hel = if selected.iter().any(|c| c.harness == "hel") {
        let hel = hel::resolve(root, options.build)?;
        println!("hel   {} ({})", hel.version, describe(&hel));
        Some(hel)
    } else {
        None
    };

    let results_dir = root.join(&plan.results);
    for task in tasks {
        for repetition in 1..=plan.repetitions {
            // Conditions in one invocation are interleaved per (task, repetition).
            for condition in &selected {
                let run_id = format!(
                    "{}-{}-{}-{:02}",
                    plan.experiment, task.id, condition.name, repetition
                );
                let run_dir = results_dir.join(&run_id);
                if run_dir.join("record.json").exists() && !options.force {
                    println!("skip  {run_id} (record exists)");
                    continue;
                }
                let spec = RunSpec {
                    run_id: run_id.clone(),
                    experiment: plan.experiment.clone(),
                    repetition,
                    task,
                    condition,
                    instruction: &task.instruction,
                    fixture: task.dir.join(&task.fixture),
                    run_dir,
                    overridden: false,
                };
                let record = execute(&api_key, plan, &spec, hel.as_ref())?;
                println!(
                    "run   {run_id}  {:?}{}",
                    record.outcome.termination,
                    if record.validity.valid {
                        ""
                    } else {
                        "  (invalid)"
                    }
                );
            }
        }
    }
    Ok(())
}

/// Prepares directories, runs the harness through its driver and writes `record.json`.
pub fn execute(
    api_key: &str,
    plan: &Plan,
    spec: &RunSpec,
    hel: Option<&HelBinary>,
) -> Result<Record, Box<dyn Error>> {
    let workdir = std::env::temp_dir().join("hel-lab").join(&spec.run_id);
    reset_dir(&spec.run_dir)?;
    reset_dir(&workdir)?;
    copy_dir(&spec.fixture, &workdir)?;
    fs::create_dir_all(spec.run_dir.join("raw"))?;

    let sidecar = RunSidecar {
        run_id: spec.run_id.clone(),
        task_id: spec.task.id.clone(),
        condition: spec.condition.name.clone(),
        harness: spec.condition.harness.clone(),
        workdir: workdir.clone(),
        overridden: spec.overridden,
    };
    fs::write(
        spec.run_dir.join("run.json"),
        serde_json::to_string_pretty(&sidecar)? + "\n",
    )?;

    let ctx = RunContext {
        run: RunInfo {
            run_id: spec.run_id.clone(),
            experiment_id: spec.experiment.clone(),
            lab: plan.lab.clone(),
            task_id: spec.task.id.clone(),
            condition: spec.condition.name.clone(),
            repetition: spec.repetition,
        },
        harness: record::Harness {
            name: spec.condition.harness.clone(),
            version: hel.map(|h| h.version.clone()).unwrap_or_default(),
            comparison_class: record::ComparisonClass::Subject,
        },
        model: record::ModelRequest {
            provider: plan.model.provider.clone(),
            requested: plan.model.id.clone(),
            params: plan.model.params.clone(),
        },
        budget: plan.budget,
    };
    let job = RunJob {
        ctx,
        instruction: spec.instruction,
        hel,
        condition: spec.condition,
        run_dir: spec.run_dir.clone(),
        workdir,
        api_key,
    };

    let record = match spec.condition.harness.as_str() {
        "hel" => hel::run(&job)?,
        "claude-code" => claude_code::run(&job)?,
        other => return Err(format!("unknown harness: {other}").into()),
    };
    fs::write(
        spec.run_dir.join("record.json"),
        serde_json::to_string_pretty(&record)? + "\n",
    )?;
    Ok(record)
}

/// `installed: <path>` or `built: target/debug/hel`, with the home directory shortened.
pub fn describe(hel: &HelBinary) -> String {
    let path = hel.path.display().to_string();
    let home = std::env::var("HOME").unwrap_or_default();
    let path = match path.strip_prefix(&home) {
        Some(rest) if !home.is_empty() => format!("~{rest}"),
        _ => path,
    };
    if hel.installed {
        format!("installed: {path}")
    } else {
        "built: target/debug/hel".to_string()
    }
}

/// Whether the harness of an optional condition can be run on this machine.
pub fn harness_available(condition: &Condition) -> bool {
    match condition.harness.as_str() {
        "claude-code" => claude_code::binary(condition).exists(),
        _ => true,
    }
}

/// Outcome of waiting for a harness process.
pub struct Finished {
    pub status: Option<ExitStatus>,
    pub timed_out: bool,
    pub started: SystemTime,
    pub ended: SystemTime,
    pub wall_time_ms: u128,
}

/// Spawns `command` and kills it if it runs longer than `budget + KILL_GRACE`.
pub fn run_with_timeout(
    mut command: Command,
    budget: Duration,
) -> Result<Finished, Box<dyn Error>> {
    let started = SystemTime::now();
    let clock = Instant::now();
    let mut child: Child = command.spawn()?;
    let limit = budget + KILL_GRACE;
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break Some(status);
        }
        if clock.elapsed() > limit {
            let _ = child.kill();
            let _ = child.wait();
            break None;
        }
        thread::sleep(Duration::from_millis(100));
    };
    Ok(Finished {
        timed_out: status.is_none(),
        status,
        started,
        ended: SystemTime::now(),
        wall_time_ms: clock.elapsed().as_millis(),
    })
}

/// Record for a run whose harness produced no usable result (killed, crashed, no output).
pub fn failure_record(
    ctx: &RunContext,
    finished: &Finished,
    termination: Termination,
    error: String,
    raw_transcript: Option<String>,
) -> Record {
    let unavailable = || Metric {
        value: None,
        status: MetricStatus::Unavailable,
    };
    Record {
        schema_version: "record-v0".to_string(),
        run: run_block(ctx, finished),
        harness: ctx.harness.clone(),
        model: Model {
            provider: ctx.model.provider.clone(),
            requested: ctx.model.requested.clone(),
            actual: None,
            params: ctx.model.params.clone(),
        },
        outcome: Outcome {
            final_output: None,
            termination,
            error: Some(error.clone()),
        },
        usage: Usage {
            input_tokens: unavailable(),
            output_tokens: unavailable(),
            model_calls: unavailable(),
            wall_time_ms: Metric {
                value: Some(finished.wall_time_ms as f64),
                status: MetricStatus::Measured,
            },
        },
        events: Vec::new(),
        validity: Validity {
            valid: false,
            reasons: vec![format!("no harness result: {error}")],
        },
        artifacts: Artifacts { raw_transcript },
    }
}

pub fn run_block(ctx: &RunContext, finished: &Finished) -> Run {
    Run {
        run_id: ctx.run.run_id.clone(),
        experiment_id: ctx.run.experiment_id.clone(),
        lab: ctx.run.lab.clone(),
        task_id: ctx.run.task_id.clone(),
        condition: ctx.run.condition.clone(),
        repetition: ctx.run.repetition,
        started_at: rfc3339(finished.started),
        ended_at: rfc3339(finished.ended),
    }
}

pub fn rfc3339(time: SystemTime) -> String {
    humantime::format_rfc3339_millis(time).to_string()
}

/// Last commit that changed `paths`, with `-dirty` if they have uncommitted changes.
pub fn git_version(root: &Path, paths: &[&str]) -> Result<String, Box<dyn Error>> {
    let last = Command::new("git")
        .args(["log", "-1", "--format=%h", "--"])
        .args(paths)
        .current_dir(root)
        .output()?;
    let mut version = String::from_utf8(last.stdout)?.trim().to_string();
    let status = Command::new("git")
        .args(["status", "--porcelain", "--"])
        .args(paths)
        .current_dir(root)
        .output()?;
    if !status.stdout.is_empty() {
        version.push_str("-dirty");
    }
    Ok(version)
}

fn reset_dir(dir: &Path) -> Result<(), Box<dyn Error>> {
    if dir.exists() {
        fs::remove_dir_all(dir)?;
    }
    fs::create_dir_all(dir)?;
    Ok(())
}

fn copy_dir(from: &Path, to: &Path) -> Result<(), Box<dyn Error>> {
    for entry in fs::read_dir(from).map_err(|e| format!("{}: {e}", from.display()))? {
        let entry = entry?;
        let target = to.join(entry.file_name());
        if entry.file_type()?.is_dir() {
            fs::create_dir_all(&target)?;
            copy_dir(&entry.path(), &target)?;
        } else {
            fs::copy(entry.path(), &target)?;
        }
    }
    Ok(())
}
