//! `evals try <task>`: run one task once and show what happened in the terminal.
//! Results go to results/try/, outside the official experiment records.

use std::error::Error;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use record::{Metric, Record, Termination};
use serde_json::json;

use crate::check::{self, Judged};
use crate::hel;
use crate::runner::{self, RunSpec};
use crate::spec::{self, Condition, Plan, Task};

const MAX_OUTPUT_LINES: usize = 40;

pub struct TryOptions {
    pub task: String,
    pub lab: Option<String>,
    pub harness: String,
    pub instruction: Option<String>,
    pub fixture: Option<PathBuf>,
    pub build: bool,
}

pub fn run(root: &Path, options: TryOptions) -> Result<(), Box<dyn Error>> {
    let lab = match &options.lab {
        Some(lab) => lab.clone(),
        None => spec::latest_lab(root)?,
    };
    let plan = spec::load_lab(root, &lab)?;
    let task = spec::load_task(root, &options.task)?;
    let condition = condition_for(&plan, &options.harness);
    if !runner::harness_available(&condition) {
        return Err(format!("{} is not installed", condition.harness).into());
    }

    let overridden = options.instruction.is_some() || options.fixture.is_some();
    let instruction = options
        .instruction
        .clone()
        .unwrap_or_else(|| task.instruction.clone());
    let fixture = match &options.fixture {
        Some(dir) => std::env::current_dir()?.join(dir),
        None => task.dir.join(&task.fixture),
    };

    let hel = match condition.harness.as_str() {
        "hel" => Some(hel::resolve(root, options.build)?),
        _ => None,
    };
    let stamp = timestamp();
    let run_dir = root
        .join("results/try")
        .join(format!("{stamp}-{}", task.id));
    let spec = RunSpec {
        run_id: format!("try-{stamp}-{}", task.id),
        experiment: "try".to_string(),
        repetition: 1,
        task: &task,
        condition: &condition,
        instruction: &instruction,
        fixture,
        run_dir: run_dir.clone(),
        overridden,
    };
    let api_key = runner::api_key()?;
    let record = runner::execute(&api_key, &plan, &spec, hel.as_ref())?;
    let judged = check::check_run(root, &run_dir, &task)?;

    let saved = run_dir.strip_prefix(root).unwrap_or(&run_dir);
    let shown = Shown {
        instruction: &instruction,
        overridden,
        saved,
        source: hel.as_ref().map(runner::describe),
    };
    show(&plan, &task, &record, &judged, &shown);
    Ok(())
}

/// The condition `try` runs as. Names follow the record schema (`variant`, `external-<harness>`).
fn condition_for(plan: &Plan, harness: &str) -> Condition {
    if harness == "hel" {
        return Condition {
            name: "variant".to_string(),
            harness: "hel".to_string(),
            optional: false,
            settings: json!({}),
        };
    }
    plan.conditions
        .iter()
        .find(|c| c.harness == harness)
        .cloned()
        .unwrap_or_else(|| Condition {
            name: format!("external-{harness}"),
            harness: harness.to_string(),
            optional: false,
            settings: json!({}),
        })
}

/// Run facts shown in the terminal besides the record itself.
struct Shown<'a> {
    instruction: &'a str,
    overridden: bool,
    saved: &'a Path,
    source: Option<String>,
}

fn show(plan: &Plan, task: &Task, record: &Record, judged: &Judged, shown: &Shown) {
    let Shown {
        instruction,
        overridden,
        saved,
        source,
    } = shown;
    let (instruction, overridden) = (*instruction, *overridden);
    println!();
    println!(
        "task      {} · {} {} · {} (lab {})",
        task.id, record.harness.name, record.harness.version, plan.model.id, plan.lab
    );
    if let Some(source) = source {
        println!("binary    {source}");
    }
    let marker = if instruction != task.instruction {
        "   (differs from the task instruction)"
    } else {
        ""
    };
    println!("input     {}{marker}", instruction.trim());

    println!("\ntool calls");
    if record.events.is_empty() {
        println!("  (none)");
    }
    for event in &record.events {
        let status = match event.ok {
            Some(true) => "ok",
            Some(false) => "error",
            None => "?",
        };
        println!("  {}  {}  {}  {status}", event.seq, event.name, event.args);
    }

    match &record.outcome.final_output {
        Some(output) => {
            let lines: Vec<&str> = output.lines().collect();
            println!("\noutput ({} lines)", lines.len());
            for line in lines.iter().take(MAX_OUTPUT_LINES) {
                println!("  {line}");
            }
            if lines.len() > MAX_OUTPUT_LINES {
                println!("  … ({} lines more)", lines.len() - MAX_OUTPUT_LINES);
            }
        }
        None => println!("\noutput    (none)"),
    }
    if record.outcome.termination != Termination::Completed {
        println!(
            "\nended     {:?}{}",
            record.outcome.termination,
            record
                .outcome
                .error
                .as_deref()
                .map(|e| format!(": {e}"))
                .unwrap_or_default()
        );
    }

    let note = if overridden {
        "    (input overridden: for reference only)"
    } else {
        ""
    };
    println!("\nchecks{note}");
    for check in &judged.verdict.checks {
        let mark = match check.result.as_str() {
            "pass" => "✓",
            "fail" => "✗",
            _ => "–",
        };
        let mut lines = check.detail.lines();
        println!(
            "  {mark} {:<14} {}",
            check.id,
            lines.next().unwrap_or_default()
        );
        for line in lines {
            println!("      {line}");
        }
    }
    println!("result    {}", judged.verdict.overall);

    let value = |m: &Metric| {
        m.value
            .map(|v| format!("{v:.0}"))
            .unwrap_or_else(|| "?".to_string())
    };
    let seconds = record
        .usage
        .wall_time_ms
        .value
        .map(|ms| format!("{:.1}s", ms / 1000.0))
        .unwrap_or_else(|| "?".to_string());
    println!(
        "\nusage     input {} · output {} · model calls {} · {seconds}",
        value(&record.usage.input_tokens),
        value(&record.usage.output_tokens),
        value(&record.usage.model_calls)
    );
    println!("saved     {}", saved.display());
}

/// UTC timestamp like `20261003T153001Z`, used in try run names.
fn timestamp() -> String {
    humantime::format_rfc3339_seconds(SystemTime::now())
        .to_string()
        .replace(['-', ':'], "")
}
