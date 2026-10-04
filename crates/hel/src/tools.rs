//! Tools hel offers to the model. H0: a single read-only tool.

use std::fs;
use std::path::Path;

use serde_json::{Value, json};

pub const READ_FILE: &str = "read_file";

/// Tool definitions sent with every request (OpenAI-compatible function tools).
pub fn definitions() -> Value {
    json!([{
        "type": "function",
        "function": {
            "name": READ_FILE,
            "description": "Read a UTF-8 text file in the working directory and return its full contents.",
            "parameters": {
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Path of the file, relative to the working directory."
                    }
                },
                "required": ["path"]
            }
        }
    }])
}

/// Runs a tool call. Errors are returned as text so the model can see what went wrong.
pub fn execute(workdir: &Path, name: &str, args: &Value) -> Result<String, String> {
    match name {
        READ_FILE => read_file(workdir, args),
        other => Err(format!("unknown tool: {other}")),
    }
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

    #[test]
    fn reads_file_inside_workdir() {
        let dir = workdir("inside");
        let out = execute(&dir, READ_FILE, &json!({ "path": "hello.txt" }));
        assert_eq!(out, Ok("Hello, harness!\n".to_string()));
    }

    #[test]
    fn refuses_path_outside_workdir() {
        let dir = workdir("outside");
        let sibling = dir.parent().unwrap().join("outside-sibling");
        fs::create_dir_all(&sibling).unwrap();
        fs::write(sibling.join("secret.txt"), "secret").unwrap();
        let out = execute(
            &dir,
            READ_FILE,
            &json!({ "path": "../outside-sibling/secret.txt" }),
        );
        assert!(out.unwrap_err().contains("outside the working directory"));
    }

    #[test]
    fn reports_missing_argument() {
        let dir = workdir("missing");
        assert!(execute(&dir, READ_FILE, &json!({})).is_err());
    }
}
