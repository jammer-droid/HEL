//! Driver and collector for our own harness, hel (evals/harnesses/hel/PROFILE.md).
//! hel writes record.json itself; the collector only reads it back.

use std::error::Error;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, SystemTime};

use record::{Record, Termination};
use serde_json::Value;

use crate::runner::{self, RunJob};

/// Source paths whose changes make a hel binary out of date.
const SOURCES: [&str; 2] = ["crates/hel", "crates/record"];

/// The hel executable a run uses.
pub struct HelBinary {
    pub path: PathBuf,
    /// Recorded as `harness.version`: last commit of the sources, `-dirty` if modified,
    /// `-stale` if an installed binary is older than the sources.
    pub version: String,
    pub installed: bool,
}

/// Uses `hel` from PATH when installed (unless `force_build`), otherwise builds the working tree.
pub fn resolve(root: &Path, force_build: bool) -> Result<HelBinary, Box<dyn Error>> {
    if !force_build && let Some(path) = find_on_path("hel") {
        let mut version = runner::git_version(root, &SOURCES)?;
        if is_older_than_sources(&path, root) {
            eprintln!(
                "warning: installed hel ({}) is older than crates/hel or crates/record.\n         \
                 Run `cargo install --path crates/hel`, or use --build to run the working tree.",
                path.display()
            );
            version.push_str("-stale");
        }
        return Ok(HelBinary {
            path,
            version,
            installed: true,
        });
    }
    let version = build(root)?;
    Ok(HelBinary {
        path: root.join("target/debug/hel"),
        version,
        installed: false,
    })
}

fn find_on_path(name: &str) -> Option<PathBuf> {
    let paths = std::env::var_os("PATH")?;
    std::env::split_paths(&paths)
        .map(|dir| dir.join(name))
        .find(|candidate| {
            fs::metadata(candidate)
                .map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
                .unwrap_or(false)
        })
}

fn is_older_than_sources(binary: &Path, root: &Path) -> bool {
    let Ok(built) = fs::metadata(binary).and_then(|m| m.modified()) else {
        return true;
    };
    SOURCES
        .iter()
        .filter_map(|dir| newest_mtime(&root.join(dir)))
        .any(|changed| changed > built)
}

fn newest_mtime(path: &Path) -> Option<SystemTime> {
    let meta = fs::metadata(path).ok()?;
    if meta.is_file() {
        return meta.modified().ok();
    }
    fs::read_dir(path)
        .ok()?
        .filter_map(|entry| entry.ok())
        .filter(|entry| entry.file_name() != "target")
        .filter_map(|entry| newest_mtime(&entry.path()))
        .max()
}

/// Builds hel from the working tree and returns its version (commit, `-dirty` if modified).
fn build(root: &Path) -> Result<String, Box<dyn Error>> {
    let status = Command::new("cargo")
        .args(["build", "--quiet", "-p", "hel"])
        .current_dir(root)
        .status()?;
    if !status.success() {
        return Err("cargo build -p hel failed".into());
    }
    runner::git_version(root, &SOURCES)
}

/// hel flags from the condition settings. No setting, no flag, so the Lab's starting code runs
/// with its own defaults: `tools` (a list of tool names) becomes `--tools a,b`; `env: true` /
/// `false` becomes `--env` / `--no-env`; `context_file: <name>` becomes `--context-file <name>`
/// and `context_file: false` becomes `--no-context-file`. (From H3 on, hel sends the environment
/// and `HEL.md` by default.)
fn hel_args(settings: &Value) -> Result<Vec<String>, Box<dyn Error>> {
    let mut args = tool_args(settings)?;
    match settings.get("env") {
        None => {}
        Some(Value::Bool(true)) => args.push("--env".to_string()),
        Some(Value::Bool(false)) => args.push("--no-env".to_string()),
        Some(other) => {
            return Err(format!("settings.env must be true or false, got {other}").into());
        }
    }
    match settings.get("context_file") {
        None => {}
        Some(Value::String(name)) if !name.is_empty() => {
            args.extend(["--context-file".to_string(), name.clone()]);
        }
        Some(Value::Bool(false)) => args.push("--no-context-file".to_string()),
        Some(other) => {
            return Err(
                format!("settings.context_file must be a file name or false, got {other}").into(),
            );
        }
    }
    Ok(args)
}

fn tool_args(settings: &Value) -> Result<Vec<String>, Box<dyn Error>> {
    let Some(tools) = settings.get("tools") else {
        return Ok(Vec::new());
    };
    let names: Option<Vec<&str>> = tools
        .as_array()
        .map(|list| list.iter().map(Value::as_str).collect())
        .unwrap_or(None);
    match names {
        Some(names) if !names.is_empty() => Ok(vec!["--tools".to_string(), names.join(",")]),
        _ => Err(
            format!("settings.tools must be a non-empty list of tool names, got {tools}").into(),
        ),
    }
}

pub fn run(job: &RunJob) -> Result<Record, Box<dyn Error>> {
    let context_path = job.run_dir.join("context.json");
    fs::write(
        &context_path,
        serde_json::to_string_pretty(&job.ctx)? + "\n",
    )?;
    let record_path = job.run_dir.join("record.json");

    let hel = job.hel.ok_or("no hel binary resolved")?;
    let mut command = Command::new(&hel.path);
    command
        .args(["--instruction", job.instruction])
        .args(hel_args(&job.condition.settings)?)
        .arg("--context")
        .arg(&context_path)
        .arg("--record")
        .arg(&record_path)
        .current_dir(&job.workdir)
        .env_clear()
        // Same as the Claude Code driver: the system default PATH, including sbin (macOS keeps
        // md5 and md5sum in /sbin).
        .env("PATH", "/usr/bin:/bin:/usr/sbin:/sbin")
        .env("DEEPSEEK_API_KEY", job.api_key)
        .stdin(Stdio::null())
        .stdout(fs::File::create(job.run_dir.join("raw/stdout.txt"))?)
        .stderr(fs::File::create(job.run_dir.join("raw/stderr.txt"))?);

    let budget = Duration::from_secs(job.ctx.budget.timeout_seconds);
    let finished = runner::run_with_timeout(command, budget)?;

    if finished.timed_out {
        return Ok(runner::failure_record(
            &job.ctx,
            &finished,
            Termination::Timeout,
            "killed by runner after timeout".to_string(),
            None,
        ));
    }
    match fs::read_to_string(&record_path) {
        Ok(text) => Ok(serde_json::from_str(&text)?),
        Err(_) => {
            let stderr = fs::read_to_string(job.run_dir.join("raw/stderr.txt")).unwrap_or_default();
            Ok(runner::failure_record(
                &job.ctx,
                &finished,
                Termination::Error,
                format!(
                    "hel exited with {:?} and wrote no record: {}",
                    finished.status,
                    stderr.trim()
                ),
                Some("raw/stderr.txt".to_string()),
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn no_tools_setting_adds_no_flag() {
        assert!(tool_args(&json!({})).unwrap().is_empty());
        assert!(tool_args(&Value::Null).unwrap().is_empty());
    }

    #[test]
    fn tools_setting_becomes_comma_separated_flag() {
        let args = tool_args(&json!({ "tools": ["bash", "read_file"] })).unwrap();
        assert_eq!(args, ["--tools", "bash,read_file"]);
    }

    #[test]
    fn env_and_context_file_settings_become_flags() {
        assert!(hel_args(&json!({})).unwrap().is_empty());
        let off = hel_args(&json!({ "env": false, "context_file": false })).unwrap();
        assert_eq!(off, ["--no-env", "--no-context-file"]);
        let args = hel_args(&json!({ "env": true, "context_file": "HEL.md" })).unwrap();
        assert_eq!(args, ["--env", "--context-file", "HEL.md"]);
        assert!(hel_args(&json!({ "env": "yes" })).is_err());
        assert!(hel_args(&json!({ "context_file": "" })).is_err());
    }

    #[test]
    fn rejects_malformed_tools_setting() {
        assert!(tool_args(&json!({ "tools": [] })).is_err());
        assert!(tool_args(&json!({ "tools": "bash" })).is_err());
        assert!(tool_args(&json!({ "tools": [1] })).is_err());
    }
}
