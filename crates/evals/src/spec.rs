//! Lab files and task specs (evals/SPEC.md §4, §8).
//!
//! A Lab file (`evals/labs/<lab>.yaml`) is the only measurement definition; it is read into a
//! [`Plan`].

use std::collections::BTreeMap;
use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};

use record::{Budget, ToolCategory};
use serde::Deserialize;
use serde_json::Value;

/// What `evals run` / `evals report` work on.
#[derive(Debug)]
pub struct Plan {
    /// Experiment ID written into records: the Lab ID, or `<lab>-r<N>` from revision 2 on.
    pub experiment: String,
    pub lab: String,
    pub model: LabModel,
    pub conditions: Vec<Condition>,
    pub tasks: Vec<String>,
    pub repetitions: u32,
    pub budget: Budget,
    pub results: PathBuf,
    pub rubric: BTreeMap<String, String>,
}

#[derive(Debug, Deserialize)]
struct LabFile {
    lab: String,
    /// Bumped when a Lab is measured again with changed conditions; keeps earlier runs apart.
    #[serde(default = "first_revision")]
    revision: u32,
    model: LabModel,
    budget: Budget,
    conditions: Vec<Condition>,
    tasks: Vec<String>,
    repetitions: u32,
    #[serde(default)]
    rubric: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct LabModel {
    pub provider: String,
    pub id: String,
    #[serde(default = "empty_object")]
    pub params: Value,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Condition {
    pub name: String,
    pub harness: String,
    /// Skip this condition when its harness is not installed.
    #[serde(default)]
    pub optional: bool,
    #[serde(default)]
    pub settings: Value,
}

#[derive(Debug, Deserialize)]
pub struct Task {
    pub id: String,
    /// A single instruction. Exactly one of `instruction` and `turns` is set.
    #[serde(default)]
    pub instruction: String,
    /// Instructions sent one after another in one session (H6). The last turn's output is judged.
    #[serde(default)]
    pub turns: Vec<String>,
    pub fixture: PathBuf,
    /// Working directory within the copied fixture; siblings can be protected test data.
    #[serde(default = "fixture_root")]
    pub fixture_workdir: PathBuf,
    pub checks: Vec<Check>,
    /// Directory of task.yaml; relative paths in the spec resolve against it.
    #[serde(skip)]
    pub dir: PathBuf,
}

#[derive(Debug, Deserialize)]
pub struct Check {
    pub id: String,
    #[serde(default)]
    pub not_applicable: Vec<String>,
    #[serde(flatten)]
    pub kind: CheckKind,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum CheckKind {
    OutputExactMatch {
        expected_file: PathBuf,
    },
    FileExactMatch {
        /// File in the working directory after the run (relative to it).
        path: PathBuf,
        expected_file: PathBuf,
        #[serde(default)]
        scope: FileScope,
    },
    ToolCalls {
        category: ToolCategory,
        count: usize,
        path: Option<String>,
    },
    /// A key in an INI section of a file in the working directory after the run has `value`.
    IniValue {
        path: PathBuf,
        section: String,
        key: String,
        value: String,
    },
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FileScope {
    #[default]
    Workspace,
    Fixture,
}

fn fixture_root() -> PathBuf {
    PathBuf::from(".")
}

fn empty_object() -> Value {
    Value::Object(Default::default())
}

fn first_revision() -> u32 {
    1
}

/// Resolves `target` to a plan: a Lab ID, or (when `None`) the latest Lab.
pub fn load_plan(root: &Path, target: Option<&str>) -> Result<Plan, Box<dyn Error>> {
    match target {
        Some(lab) => load_lab(root, lab),
        None => load_lab(root, &latest_lab(root)?),
    }
}

/// The Lab file with the highest ID in `evals/labs/` (h00 < h01 < ...).
pub fn latest_lab(root: &Path) -> Result<String, Box<dyn Error>> {
    let dir = root.join("evals/labs");
    let mut labs: Vec<String> = fs::read_dir(&dir)
        .map_err(|e| format!("{}: {e}", dir.display()))?
        .filter_map(|entry| entry.ok())
        .filter_map(|entry| {
            let name = entry.file_name().into_string().ok()?;
            name.strip_suffix(".yaml").map(str::to_string)
        })
        .collect();
    labs.sort();
    labs.pop()
        .ok_or_else(|| format!("no Lab files in {}", dir.display()).into())
}

pub fn load_lab(root: &Path, lab: &str) -> Result<Plan, Box<dyn Error>> {
    let path = root.join("evals/labs").join(format!("{lab}.yaml"));
    let text =
        fs::read_to_string(&path).map_err(|e| format!("Lab {lab}: {}: {e}", path.display()))?;
    let file: LabFile =
        serde_yaml_ng::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))?;
    if file.lab != lab {
        return Err(format!(
            "{}: lab is {:?}, expected {lab:?}",
            path.display(),
            file.lab
        )
        .into());
    }
    if file.revision == 0 {
        return Err(format!("{}: revision starts at 1", path.display()).into());
    }
    let experiment = match file.revision {
        1 => file.lab.clone(),
        n => format!("{}-r{n}", file.lab),
    };
    Ok(Plan {
        experiment,
        results: PathBuf::from("results").join(&file.lab),
        lab: file.lab,
        model: file.model,
        conditions: file.conditions,
        tasks: file.tasks,
        repetitions: file.repetitions,
        budget: file.budget,
        rubric: file.rubric,
    })
}

pub fn load_task(root: &Path, id: &str) -> Result<Task, Box<dyn Error>> {
    let dir = root.join("evals/tasks").join(id);
    let path = dir.join("task.yaml");
    let text =
        fs::read_to_string(&path).map_err(|e| format!("task {id}: {}: {e}", path.display()))?;
    let mut task: Task =
        serde_yaml_ng::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))?;
    if task.id != id {
        return Err(format!("{}: id is {:?}, expected {id:?}", path.display(), task.id).into());
    }
    let has_instruction = !task.instruction.trim().is_empty();
    let has_turns = !task.turns.is_empty();
    if has_instruction == has_turns {
        return Err(format!(
            "{}: set exactly one of instruction and turns",
            path.display()
        )
        .into());
    }
    if task.turns.iter().any(|turn| turn.trim().is_empty()) {
        return Err(format!("{}: turns must not be empty strings", path.display()).into());
    }
    task.dir = dir;
    Ok(task)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every Lab definition and the tasks it names parse, so a typo shows up before any run.
    #[test]
    fn repository_labs_and_tasks_parse() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let labs = fs::read_dir(root.join("evals/labs")).unwrap();
        for entry in labs {
            let path = entry.unwrap().path();
            let Some(lab) = path.file_stem().and_then(|s| s.to_str()) else {
                continue;
            };
            let plan = load_plan(&root, Some(lab)).unwrap_or_else(|e| panic!("{lab}: {e}"));
            for id in &plan.tasks {
                load_task(&root, id).unwrap_or_else(|e| panic!("{lab}/{id}: {e}"));
            }
        }
    }

    #[test]
    fn session_task_has_turns_and_no_instruction() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let task = load_task(&root, "session-recall-01").unwrap();
        assert!(task.instruction.is_empty());
        assert_eq!(task.turns.len(), 6);
    }
}
