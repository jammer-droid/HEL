//! evals — try, run and report Lab experiments (evals/SPEC.md §8).
//!
//!   evals try <task> [--lab <lab>] [--harness <name> | --condition <name>] [--instruction <text>] [--fixture <dir>]
//!   evals run [lab] [--conditions a,b] [--force]
//!   evals report [lab]
//!
//! Works from any directory inside the HarnessEngineeringLab repository. A Lab is given by its
//! ID (`h00` → evals/labs/h00.yaml) and defaults to the latest Lab; a task by its ID
//! (`read-echo-01` → evals/tasks/read-echo-01/).

mod check;
mod claude_code;
mod hel;
mod report;
mod runner;
mod session;
mod spec;
mod try_run;

use std::error::Error;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

const USAGE: &str = "usage:
  evals try <task> [--lab <lab>] [--harness hel|claude-code | --condition <name>] [--instruction <text>] [--fixture <dir>] [--build]
  evals run [lab] [--conditions a,b] [--force] [--build]
  evals report [lab]

hel: uses the installed `hel` on PATH if present, otherwise builds crates/hel.
--build always builds and runs the working tree.
--condition runs try with that condition of the Lab definition (harness and settings).";

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(err) => {
            eprintln!("evals: {err}");
            ExitCode::from(2)
        }
    }
}

fn run() -> Result<(), Box<dyn Error>> {
    let mut args = std::env::args().skip(1).peekable();
    let command = args.next().ok_or(USAGE)?;
    if command == "-h" || command == "--help" {
        println!("{USAGE}");
        return Ok(());
    }
    let root = find_root()?;
    let root = root.as_path();

    // First positional argument (task ID or Lab ID), then flags.
    let target = match args.peek() {
        Some(arg) if !arg.starts_with("--") => args.next(),
        _ => None,
    };
    let mut flags = Flags::default();
    while let Some(flag) = args.next() {
        let mut value = || args.next().ok_or(format!("{flag} needs a value"));
        match flag.as_str() {
            "--conditions" => {
                flags.conditions = Some(value()?.split(',').map(str::to_string).collect())
            }
            "--force" => flags.force = true,
            "--build" => flags.build = true,
            "--lab" => flags.lab = Some(value()?),
            "--harness" => flags.harness = Some(value()?),
            "--condition" => flags.condition = Some(value()?),
            "--instruction" => flags.instruction = Some(value()?),
            "--fixture" => flags.fixture = Some(PathBuf::from(value()?)),
            other => return Err(format!("unknown argument: {other}\n{USAGE}").into()),
        }
    }

    match command.as_str() {
        "try" => try_run::run(
            root,
            try_run::TryOptions {
                task: target.ok_or(format!("try needs a task ID\n{USAGE}"))?,
                lab: flags.lab,
                harness: flags.harness.unwrap_or_else(|| "hel".to_string()),
                condition: flags.condition,
                instruction: flags.instruction,
                fixture: flags.fixture,
                build: flags.build,
            },
        ),
        "run" | "report" | "check" => {
            let plan = spec::load_plan(root, target.as_deref())?;
            let tasks = plan
                .tasks
                .iter()
                .map(|id| spec::load_task(root, id))
                .collect::<Result<Vec<_>, _>>()?;
            if command == "run" {
                let options = runner::Options {
                    conditions: flags.conditions,
                    force: flags.force,
                    build: flags.build,
                };
                runner::run_all(root, &plan, &tasks, &options)?;
            }
            let checked = check::check_all(root, &plan, &tasks)?;
            report::write(root, &plan, &checked)
        }
        other => Err(format!("unknown command: {other}\n{USAGE}").into()),
    }
}

#[derive(Default)]
struct Flags {
    conditions: Option<Vec<String>>,
    force: bool,
    build: bool,
    lab: Option<String>,
    harness: Option<String>,
    condition: Option<String>,
    instruction: Option<String>,
    fixture: Option<PathBuf>,
}

/// Walks up from the current directory to the repository root (the directory with
/// `evals/SPEC.md`). Absolute, because harness processes run with a temp workdir as cwd.
fn find_root() -> Result<PathBuf, Box<dyn Error>> {
    let cwd = std::env::current_dir()?;
    let mut dir: Option<&Path> = Some(&cwd);
    while let Some(current) = dir {
        if current.join("evals/SPEC.md").exists() && current.join("Cargo.toml").exists() {
            return Ok(current.to_path_buf());
        }
        dir = current.parent();
    }
    Err("run evals inside the HarnessEngineeringLab repository".into())
}
