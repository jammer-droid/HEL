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
    /// Session turns; empty for single-instruction tasks.
    pub turns: &'a [String],
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
    pub turns: &'a [String],
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

const API_KEY: &str = "DEEPSEEK_API_KEY";

/// The API key from the environment, or else from `.env` at the repository root (git-ignored).
pub fn api_key(root: &Path) -> Result<String, Box<dyn Error>> {
    if let Ok(key) = std::env::var(API_KEY)
        && !key.is_empty()
    {
        return Ok(key);
    }
    std::fs::read_to_string(root.join(".env"))
        .ok()
        .and_then(|text| dotenv_value(&text, API_KEY))
        .ok_or_else(|| format!("{API_KEY} is not set and not found in .env").into())
}

/// Reads `KEY=value` from `.env` text. Ignores blank lines, `#` comments and an `export ` prefix,
/// and strips one pair of matching quotes around the value.
fn dotenv_value(text: &str, key: &str) -> Option<String> {
    text.lines().find_map(|line| {
        let line = line.trim();
        let line = line.strip_prefix("export ").unwrap_or(line);
        let (name, value) = line.split_once('=')?;
        if name.trim() != key {
            return None;
        }
        let value = value.trim();
        let value = ['"', '\'']
            .iter()
            .find_map(|q| value.strip_prefix(*q).and_then(|v| v.strip_suffix(*q)))
            .unwrap_or(value);
        (!value.is_empty()).then(|| value.to_string())
    })
}

pub fn run_all(
    root: &Path,
    plan: &Plan,
    tasks: &[Task],
    options: &Options,
) -> Result<(), Box<dyn Error>> {
    let api_key = api_key(root)?;
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
                    turns: &task.turns,
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
    let fixture_dir = std::env::temp_dir().join("hel-lab").join(&spec.run_id);
    reset_dir(&spec.run_dir)?;
    reset_dir(&fixture_dir)?;
    copy_dir(&spec.fixture, &fixture_dir)?;
    let workdir = fixture_workdir(&fixture_dir, &spec.task.fixture_workdir)?;
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
        turns: spec.turns,
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
    // Keep the working directory as the run left it, for checks on files (file_exact_match).
    let workspace = spec.run_dir.join("workspace");
    fs::create_dir_all(&workspace)?;
    copy_dir(&job.workdir, &workspace)?;
    // The full fixture also preserves test files outside the harness working directory.
    let fixture_snapshot = spec.run_dir.join("fixture");
    fs::create_dir_all(&fixture_snapshot)?;
    copy_dir(&fixture_dir, &fixture_snapshot)?;
    fs::write(
        spec.run_dir.join("record.json"),
        serde_json::to_string_pretty(&record)? + "\n",
    )?;
    Ok(record)
}

fn fixture_workdir(root: &Path, relative: &Path) -> Result<PathBuf, Box<dyn Error>> {
    if relative.is_absolute()
        || relative
            .components()
            .any(|c| matches!(c, std::path::Component::ParentDir))
    {
        return Err("fixture_workdir must stay within the copied fixture".into());
    }
    let root = root.canonicalize()?;
    let workdir = root.join(relative).canonicalize()?;
    if !workdir.is_dir() || !workdir.starts_with(&root) {
        return Err("fixture_workdir must be a directory within the copied fixture".into());
    }
    Ok(workdir)
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
            cached_input_tokens: unavailable(),
            peak_context_tokens: unavailable(),
            last_context_tokens: unavailable(),
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
        let file_type = entry.file_type()?;
        if file_type.is_symlink() {
            continue;
        }
        if file_type.is_dir() {
            fs::create_dir_all(&target)?;
            copy_dir(&entry.path(), &target)?;
        } else {
            fs::copy(entry.path(), &target)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nested_fixture_keeps_protected_files_outside_workdir() {
        let root = std::env::temp_dir().join(format!("evals-fixture-{}", uuid::Uuid::new_v4()));
        let source = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../evals/tasks/sandbox-outside-01/fixture");
        fs::create_dir_all(&root).unwrap();
        copy_dir(&source, &root).unwrap();
        let workdir = fixture_workdir(&root, Path::new("workspace")).unwrap();
        assert!(
            !root
                .join("protected")
                .canonicalize()
                .unwrap()
                .starts_with(&workdir)
        );
        let output = Command::new("/bin/sh")
            .arg("probe.sh")
            .current_dir(&workdir)
            .output()
            .unwrap();
        assert!(output.status.success());
        assert_eq!(output.stdout, b"read=allowed\nwrite=allowed\n");
        assert_eq!(
            fs::read(root.join("protected/write.txt")).unwrap(),
            b"changed\n"
        );
        assert_eq!(
            fs::read(source.join("protected/write.txt")).unwrap(),
            b"unchanged\n"
        );
        assert_eq!(
            fixture_workdir(&root, Path::new(".")).unwrap(),
            root.canonicalize().unwrap()
        );
        for invalid in [
            Path::new(".."),
            Path::new("/"),
            Path::new("protected/read.txt"),
        ] {
            assert!(fixture_workdir(&root, invalid).is_err());
        }
        std::os::unix::fs::symlink(root.parent().unwrap(), root.join("escape")).unwrap();
        assert!(fixture_workdir(&root, Path::new("escape")).is_err());
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn reads_the_api_key_from_dotenv_text() {
        let text = "# local secrets\nOTHER=1\nexport DEEPSEEK_API_KEY=\"sk-test\"\n";
        assert_eq!(
            dotenv_value(text, "DEEPSEEK_API_KEY").as_deref(),
            Some("sk-test")
        );
        assert_eq!(
            dotenv_value("DEEPSEEK_API_KEY='a b'", "DEEPSEEK_API_KEY").as_deref(),
            Some("a b")
        );
        assert_eq!(dotenv_value("DEEPSEEK_API_KEY=", "DEEPSEEK_API_KEY"), None);
        assert_eq!(
            dotenv_value("# DEEPSEEK_API_KEY=x", "DEEPSEEK_API_KEY"),
            None
        );
    }
}
