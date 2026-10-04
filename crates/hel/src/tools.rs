//! Tools hel offers to the model. H0: `read_file`. H1: `bash`, and the set of tools given to the
//! model is chosen per run (`--tools`).

use std::fs;
use std::path::Path;
use std::process::{Command, Stdio};

use record::ToolCategory;
use serde_json::{Value, json};

pub const READ_FILE: &str = "read_file";
pub const BASH: &str = "bash";

/// Tools given when `--tools` is not passed. H1 follows the paper's advice to start from bash.
pub const DEFAULT: &[&str] = &[BASH];

/// The tools offered in one run: their definitions for the request, and the only names
/// `execute` will run.
pub struct Toolset {
    names: Vec<String>,
    definitions: Value,
}

impl Toolset {
    /// Builds a toolset from names such as `["bash", "read_file"]`. Unknown names and an
    /// empty list are refused; duplicates are kept once.
    pub fn new<S: AsRef<str>>(names: &[S]) -> Result<Self, String> {
        let mut kept: Vec<String> = Vec::new();
        for name in names {
            let name = name.as_ref().trim();
            if definition(name).is_none() {
                return Err(format!("unknown tool: {name} (known: {BASH}, {READ_FILE})"));
            }
            if !kept.iter().any(|k| k == name) {
                kept.push(name.to_string());
            }
        }
        if kept.is_empty() {
            return Err("no tools given".to_string());
        }
        let definitions = Value::Array(kept.iter().filter_map(|n| definition(n)).collect());
        Ok(Toolset {
            names: kept,
            definitions,
        })
    }

    /// Tool definitions sent with every request (OpenAI-compatible function tools).
    pub fn definitions(&self) -> &Value {
        &self.definitions
    }

    /// Runs a tool call. Errors are returned as text so the model can see what went wrong.
    /// A tool that exists but was not given in this run is refused like an unknown one.
    pub fn execute(&self, workdir: &Path, name: &str, args: &Value) -> Result<String, String> {
        if !self.names.iter().any(|n| n == name) {
            return Err(format!("unknown tool: {name}"));
        }
        match name {
            READ_FILE => read_file(workdir, args),
            BASH => bash(workdir, args),
            other => Err(format!("unknown tool: {other}")),
        }
    }
}

/// Category recorded for each tool call (evals/harnesses/hel/PROFILE.md). `bash` is `exec`
/// whatever the command does.
pub fn category(name: &str) -> ToolCategory {
    match name {
        READ_FILE => ToolCategory::Read,
        BASH => ToolCategory::Exec,
        _ => ToolCategory::Other,
    }
}

fn definition(name: &str) -> Option<Value> {
    let (description, param, param_description) = match name {
        READ_FILE => (
            "Read a UTF-8 text file in the working directory and return its full contents.",
            "path",
            "Path of the file, relative to the working directory.",
        ),
        BASH => (
            "Run a bash command in the working directory. Returns `exit=<code>` on the first \
             line, then stdout, then stderr. Each call runs in a new shell, so `cd` and \
             environment variables do not carry over.",
            "command",
            "The bash command to run.",
        ),
        _ => return None,
    };
    Some(json!({
        "type": "function",
        "function": {
            "name": name,
            "description": description,
            "parameters": {
                "type": "object",
                "properties": {
                    param: { "type": "string", "description": param_description }
                },
                "required": [param]
            }
        }
    }))
}

/// Reads a file inside `workdir`. Paths that resolve outside it are refused, so the model
/// cannot send files from elsewhere on the machine to the API.
fn read_file(workdir: &Path, args: &Value) -> Result<String, String> {
    let path = args
        .get("path")
        .and_then(Value::as_str)
        .ok_or("missing string argument: path")?;
    let root = fs::canonicalize(workdir).map_err(|e| format!("working directory: {e}"))?;
    let target = fs::canonicalize(root.join(path)).map_err(|e| format!("{path}: {e}"))?;
    if !target.starts_with(&root) {
        return Err(format!("{path}: outside the working directory"));
    }
    fs::read_to_string(&target).map_err(|e| format!("{path}: {e}"))
}

/// Runs a shell command with `bash -c` in `workdir` and returns `exit=<code>` followed by
/// stdout and then stderr. stdin is closed, so a command that waits for input ends at once.
/// A non-zero exit is returned as `Err` so the run record marks the call as failed.
fn bash(workdir: &Path, args: &Value) -> Result<String, String> {
    let command = args
        .get("command")
        .and_then(Value::as_str)
        .ok_or("missing string argument: command")?;
    let output = Command::new("bash")
        .args(["-c", command])
        .current_dir(workdir)
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("could not start bash: {e}"))?;
    let code = output
        .status
        .code()
        .map_or("killed".to_string(), |c| c.to_string());
    let text = format!(
        "exit={code}\n{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    if output.status.success() {
        Ok(text)
    } else {
        Err(text)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn workdir(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join("hel-tools-test").join(name);
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("hello.txt"), "Hello, harness!\n").unwrap();
        dir
    }

    fn read_only() -> Toolset {
        Toolset::new(&[READ_FILE]).unwrap()
    }

    fn with_bash() -> Toolset {
        Toolset::new(&[BASH]).unwrap()
    }

    #[test]
    fn reads_file_inside_workdir() {
        let dir = workdir("inside");
        let out = read_only().execute(&dir, READ_FILE, &json!({ "path": "hello.txt" }));
        assert_eq!(out, Ok("Hello, harness!\n".to_string()));
    }

    #[test]
    fn refuses_path_outside_workdir() {
        let dir = workdir("outside");
        let sibling = dir.parent().unwrap().join("outside-sibling");
        fs::create_dir_all(&sibling).unwrap();
        fs::write(sibling.join("secret.txt"), "secret").unwrap();
        let out = read_only().execute(
            &dir,
            READ_FILE,
            &json!({ "path": "../outside-sibling/secret.txt" }),
        );
        assert!(out.unwrap_err().contains("outside the working directory"));
    }

    #[test]
    fn reports_missing_argument() {
        let dir = workdir("missing");
        assert!(read_only().execute(&dir, READ_FILE, &json!({})).is_err());
    }

    #[test]
    fn bash_runs_in_workdir_and_reports_exit_code() {
        let dir = workdir("bash-ok");
        let out = with_bash().execute(&dir, BASH, &json!({ "command": "cat hello.txt" }));
        assert_eq!(out, Ok("exit=0\nHello, harness!\n".to_string()));
    }

    #[test]
    fn bash_failure_is_err_with_exit_code_and_stderr() {
        let dir = workdir("bash-fail");
        let out = with_bash().execute(&dir, BASH, &json!({ "command": "cat missing.txt" }));
        let err = out.unwrap_err();
        assert!(err.starts_with("exit=1\n"), "{err}");
        assert!(err.contains("missing.txt"), "{err}");
    }

    #[test]
    fn bash_does_not_wait_for_input() {
        let dir = workdir("bash-stdin");
        let out = with_bash().execute(&dir, BASH, &json!({ "command": "cat" }));
        assert_eq!(out, Ok("exit=0\n".to_string()));
    }

    #[test]
    fn refuses_tool_not_given_in_this_run() {
        let dir = workdir("not-given");
        let out = with_bash().execute(&dir, READ_FILE, &json!({ "path": "hello.txt" }));
        assert_eq!(out, Err("unknown tool: read_file".to_string()));
    }

    #[test]
    fn toolset_rejects_unknown_and_empty_lists_and_drops_duplicates() {
        assert!(Toolset::new(&["bash", "grep"]).is_err());
        assert!(Toolset::new::<&str>(&[]).is_err());
        let both = Toolset::new(&["bash", "read_file", "bash"]).unwrap();
        let names: Vec<&str> = both
            .definitions()
            .as_array()
            .unwrap()
            .iter()
            .map(|d| d["function"]["name"].as_str().unwrap())
            .collect();
        assert_eq!(names, ["bash", "read_file"]);
    }
}
