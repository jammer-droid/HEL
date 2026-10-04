//! Lab files, experiment manifests and task specs (evals/SPEC.md §4, §8).
//!
//! Both a public Lab file (`evals/labs/<lab>.yaml`) and an internal experiment manifest
//! (`*.yaml` path with `experiment` and `artifacts.results`) are read into one [`Plan`].

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
    /// Experiment ID written into records. For a Lab file this is the Lab ID.
    pub experiment: String,
    pub lab: String,
    pub model: ManifestModel,
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
    model: ManifestModel,
    budget: Budget,
    conditions: Vec<Condition>,
    tasks: Vec<String>,
    repetitions: u32,
    #[serde(default)]
    rubric: BTreeMap<String, String>,
}

#[derive(Debug, Deserialize)]
struct Manifest {
    experiment: String,
    lab: String,
    model: ManifestModel,
    conditions: Vec<Condition>,
    tasks: Vec<String>,
    repetitions: u32,
    budget: Budget,
    artifacts: ManifestArtifacts,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ManifestModel {
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
struct ManifestArtifacts {
    results: PathBuf,
}

#[derive(Debug, Deserialize)]
pub struct Task {
    pub id: String,
    pub instruction: String,
    pub fixture: PathBuf,
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
    ToolCalls {
        category: ToolCategory,
        count: usize,
        path: Option<String>,
    },
}

fn empty_object() -> Value {
    Value::Object(Default::default())
}

/// Resolves `target` to a plan: a `.yaml` path, a Lab ID, or (when `None`) the latest Lab.
pub fn load_plan(root: &Path, target: Option<&str>) -> Result<Plan, Box<dyn Error>> {
    match target {
        Some(path) if path.ends_with(".yaml") || path.ends_with(".yml") => {
            load_manifest(Path::new(path))
        }
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
    Ok(Plan {
        experiment: file.lab.clone(),
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

fn load_manifest(path: &Path) -> Result<Plan, Box<dyn Error>> {
    let text = fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let file: Manifest =
        serde_yaml_ng::from_str(&text).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(Plan {
        experiment: file.experiment,
        lab: file.lab,
        model: file.model,
        conditions: file.conditions,
        tasks: file.tasks,
        repetitions: file.repetitions,
        budget: file.budget,
        results: file.artifacts.results,
        rubric: BTreeMap::new(),
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
    task.dir = dir;
    Ok(task)
}
