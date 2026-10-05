//! Tools hel offers to the model. H0: `read_file`. H1: `bash`, and the set of tools given to the
//! model is chosen per run (`--tools`). H2: `write_file`, `search_replace`. H4: `glob`, `grep`.

#[cfg(test)]
use std::fs;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::Path;
#[cfg(test)]
use std::path::PathBuf;

use crate::runtime::Runtime;

use crate::permissions::{self, Access, Action, Approvable, Approval, Execution, Tool};
use crate::search::{self, GLOB, GREP};
use record::ToolCategory;
use serde_json::{Value, json};

pub const READ_FILE: &str = "read_file";
pub const BASH: &str = "bash";
pub const WRITE_FILE: &str = "write_file";
pub const SEARCH_REPLACE: &str = "search_replace";

const KNOWN: &[&str] = &[BASH, READ_FILE, WRITE_FILE, SEARCH_REPLACE, GLOB, GREP];

/// Tools given when `--tools` is not passed. H1 follows the paper's advice to start from bash.
pub const DEFAULT: &[&str] = &[BASH];

/// The tools offered in one run: their definitions for the request, and the only names
/// `execute` will run.
pub struct Toolset {
    tools: Vec<Box<dyn Tool>>,
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
                return Err(format!(
                    "unknown tool: {name} (known: {})",
                    KNOWN.join(", ")
                ));
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
            tools: kept.iter().map(|name| builtin(name)).collect(),
            definitions,
        })
    }

    /// Tool definitions sent with every request (OpenAI-compatible function tools).
    pub fn definitions(&self) -> &Value {
        &self.definitions
    }

    /// Every production invocation passes through the common permission gate.
    pub fn call(
        &self,
        runtime: &Runtime,
        name: &str,
        args: &Value,
        access: Access,
        approval: &mut dyn Approval,
    ) -> Execution {
        let tool = self
            .tools
            .iter()
            .find(|tool| tool.name() == name)
            .map(|tool| tool.as_ref());
        permissions::execute(tool, name, args, runtime, access, approval)
    }

    #[cfg(test)]
    pub fn execute(&self, workdir: &Path, name: &str, args: &Value) -> Result<String, String> {
        self.call(
            &Runtime::new(workdir).map_err(|e| e.to_string())?,
            name,
            args,
            Access::Auto,
            &mut permissions::Input::Unavailable,
        )
        .result
    }
}

macro_rules! tool {
    ($type:ident, $name:expr, $action:expr, $run:expr) => {
        struct $type;
        impl Approvable for $type {
            fn action(&self, _args: &Value) -> Action {
                $action
            }
        }
        impl Tool for $type {
            fn name(&self) -> &'static str {
                $name
            }
            fn run(&self, runtime: &Runtime, args: &Value) -> Result<String, String> {
                ($run)(runtime, args)
            }
        }
    };
}

tool!(
    ReadFile,
    READ_FILE,
    Action::Read,
    |runtime: &Runtime, args| runtime.reader.read(runtime, args)
);
tool!(WriteFile, WRITE_FILE, Action::Write, write_file);
tool!(SearchReplace, SEARCH_REPLACE, Action::Write, search_replace);
tool!(Bash, BASH, Action::Execute, bash);
tool!(Glob, GLOB, Action::Read, |dir, args| search::execute(
    dir, GLOB, args
));
tool!(Grep, GREP, Action::Read, |dir, args| search::execute(
    dir, GREP, args
));

fn builtin(name: &str) -> Box<dyn Tool> {
    match name {
        READ_FILE => Box::new(ReadFile),
        WRITE_FILE => Box::new(WriteFile),
        SEARCH_REPLACE => Box::new(SearchReplace),
        BASH => Box::new(Bash),
        GLOB => Box::new(Glob),
        GREP => Box::new(Grep),
        _ => unreachable!("Toolset validated the registered tool name"),
    }
}

/// Category recorded for each tool call (evals/harnesses/hel/PROFILE.md). `bash` is `exec`
/// whatever the command does.
pub fn category(name: &str) -> ToolCategory {
    match name {
        READ_FILE => ToolCategory::Read,
        BASH => ToolCategory::Exec,
        WRITE_FILE | SEARCH_REPLACE => ToolCategory::Edit,
        GLOB | GREP => ToolCategory::Search,
        _ => ToolCategory::Other,
    }
}

fn definition(name: &str) -> Option<Value> {
    if name == READ_FILE {
        return Some(crate::read_file::definition());
    }
    if matches!(name, GLOB | GREP) {
        return search::definition(name);
    }
    const PATH: (&str, &str) = (
        "path",
        "Path of the file, relative to the working directory.",
    );
    let (description, params): (&str, &[(&str, &str)]) = match name {
        BASH => (
            "Run a bash command in the working directory. Returns `exit=<code>` on the first \
             line, then stdout, then stderr. Each call runs in a new shell, so `cd` and \
             environment variables do not carry over.",
            &[("command", "The bash command to run.")],
        ),
        WRITE_FILE => (
            "Write a UTF-8 text file in the working directory, creating it or replacing its \
             whole contents. The parent directory must exist.",
            &[PATH, ("content", "The complete new contents of the file.")],
        ),
        SEARCH_REPLACE => (
            "Replace text in a file in the working directory. `search` must appear in the \
             file exactly once, character for character; otherwise nothing is changed and \
             the number of occurrences is returned as an error.",
            &[
                PATH,
                (
                    "search",
                    "The exact text to replace. Must occur exactly once in the file.",
                ),
                ("replace", "The text to put in its place."),
            ],
        ),
        _ => return None,
    };
    let properties: serde_json::Map<String, Value> = params
        .iter()
        .map(|(param, about)| {
            (
                param.to_string(),
                json!({ "type": "string", "description": about }),
            )
        })
        .collect();
    let required: Vec<&str> = params.iter().map(|(param, _)| *param).collect();
    Some(json!({
        "type": "function",
        "function": {
            "name": name,
            "description": description,
            "parameters": {
                "type": "object",
                "properties": properties,
                "required": required
            }
        }
    }))
}

/// Reads a string argument, or explains which one is missing.
fn string_arg<'a>(args: &'a Value, name: &str) -> Result<&'a str, String> {
    args.get(name)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("missing string argument: {name}"))
}

/// Writes `content` to a file inside `workdir`, creating or overwriting it.
fn write_file(runtime: &Runtime, args: &Value) -> Result<String, String> {
    let path = string_arg(args, "path")?;
    let content = string_arg(args, "content")?;
    let mut file = runtime
        .edit_open(Path::new(path), true)
        .map_err(|e| format!("{path}: {e}"))?;
    file.set_len(0)
        .and_then(|()| file.write_all(content.as_bytes()))
        .map_err(|e| format!("{path}: {e}"))?;
    Ok(format!("wrote {} bytes to {path}", content.len()))
}

/// Replaces `search` with `replace` in a file inside `workdir` only when `search` occurs
/// exactly once (paper §16.10, Listing 3). Otherwise the file is left as it was.
fn search_replace(runtime: &Runtime, args: &Value) -> Result<String, String> {
    let path = string_arg(args, "path")?;
    let search = string_arg(args, "search")?;
    let replace = string_arg(args, "replace")?;
    if search.is_empty() {
        return Err("search must not be empty".to_string());
    }
    let mut file = runtime
        .edit_open(Path::new(path), false)
        .map_err(|e| format!("{path}: {e}"))?;
    let mut text = String::new();
    file.read_to_string(&mut text)
        .map_err(|e| format!("{path}: {e}"))?;
    let count = text.matches(search).count();
    if count != 1 {
        return Err(format!(
            "search string occurs {count}x in {path}; must be unique"
        ));
    }
    let changed = text.replacen(search, replace, 1);
    file.seek(SeekFrom::Start(0))
        .and_then(|_| file.write_all(changed.as_bytes()))
        .and_then(|()| file.set_len(changed.len() as u64))
        .map_err(|e| format!("{path}: {e}"))?;
    Ok(format!("replaced 1 occurrence in {path}"))
}

/// Runs a shell command with `bash -c` in `workdir` and returns `exit=<code>` followed by
/// stdout and then stderr. stdin is closed, so a command that waits for input ends at once.
/// A non-zero exit is returned as `Err` so the run record marks the call as failed.
fn bash(runtime: &Runtime, args: &Value) -> Result<String, String> {
    let command = args
        .get("command")
        .and_then(Value::as_str)
        .ok_or("missing string argument: command")?;
    let output = crate::sandbox::command(runtime, "/bin/bash")?
        .args(["--noprofile", "--norc", "-c", command])
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

    #[test]
    #[cfg(target_os = "macos")]
    fn all_builtins_follow_access_levels_without_changing_files_before_approval() {
        use crate::permissions::{Approval, Input, Response};
        struct CheckFile {
            path: PathBuf,
            response: Response,
            asked: usize,
        }
        impl Approval for CheckFile {
            fn request(&mut self, _name: &str, _args: &Value) -> Response {
                assert_eq!(fs::read_to_string(&self.path).unwrap(), "pending\n");
                self.asked += 1;
                self.response
            }
        }
        let dir = workdir("permissions");
        let path = dir.join("status.txt");
        let tools = Toolset::new(KNOWN).unwrap();
        let requests = [
            (WRITE_FILE, json!({"path":"status.txt", "content":"done\n"})),
            (
                SEARCH_REPLACE,
                json!({"path":"status.txt", "search":"pending", "replace":"done"}),
            ),
            (BASH, json!({"command":"printf 'done\\n' > status.txt"})),
        ];
        for (name, args) in requests {
            for access in [Access::ReadOnly, Access::Confirm, Access::Auto] {
                for response in [Response::Approved, Response::Denied, Response::Unavailable] {
                    fs::write(&path, "pending\n").unwrap();
                    let mut approval = CheckFile {
                        path: path.clone(),
                        response,
                        asked: 0,
                    };
                    let out = tools.call(
                        &Runtime::new(&dir).unwrap(),
                        name,
                        &args,
                        access,
                        &mut approval,
                    );
                    let allowed = access == Access::Auto
                        || (access == Access::Confirm && response == Response::Approved);
                    assert_eq!(
                        out.result.is_ok(),
                        allowed,
                        "{name} {access:?} {response:?}"
                    );
                    assert_eq!(out.trace.executed, allowed);
                    assert_eq!(approval.asked, usize::from(access == Access::Confirm));
                    assert_eq!(
                        fs::read_to_string(&path).unwrap(),
                        if allowed { "done\n" } else { "pending\n" }
                    );
                    let read = tools.call(
                        &Runtime::new(&dir).unwrap(),
                        READ_FILE,
                        &json!({"path":"status.txt"}),
                        access,
                        &mut Input::Unavailable,
                    );
                    assert!(
                        read.result.is_ok(),
                        "reading must still work after a denial"
                    );
                }
            }
        }
        // rg-backed search is described by its controlled operation, not by process spawning.
        for name in [READ_FILE, GLOB, GREP] {
            assert_eq!(builtin(name).action(&json!({})), Action::Read);
        }
        fs::remove_dir_all(dir).unwrap();
    }

    fn read_only() -> Toolset {
        Toolset::new(&[READ_FILE]).unwrap()
    }

    fn with_bash() -> Toolset {
        Toolset::new(&[BASH]).unwrap()
    }

    fn editing() -> Toolset {
        Toolset::new(&[WRITE_FILE, SEARCH_REPLACE]).unwrap()
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
    #[cfg(target_os = "macos")]
    fn bash_runs_in_workdir_and_reports_exit_code() {
        let dir = workdir("bash-ok");
        let out = with_bash().execute(&dir, BASH, &json!({ "command": "cat hello.txt" }));
        assert_eq!(out, Ok("exit=0\nHello, harness!\n".to_string()));
    }

    #[test]
    #[cfg(target_os = "macos")]
    fn bash_failure_is_err_with_exit_code_and_stderr() {
        let dir = workdir("bash-fail");
        let out = with_bash().execute(&dir, BASH, &json!({ "command": "cat missing.txt" }));
        let err = out.unwrap_err();
        assert!(err.starts_with("exit=1\n"), "{err}");
        assert!(err.contains("missing.txt"), "{err}");
    }

    #[test]
    #[cfg(target_os = "macos")]
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
        assert!(Toolset::new(&["bash", "unknown"]).is_err());
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

    #[test]
    fn write_file_creates_and_overwrites_inside_workdir() {
        let dir = workdir("write");
        let tools = editing();
        let out = tools.execute(
            &dir,
            WRITE_FILE,
            &json!({ "path": "new.txt", "content": "a\n" }),
        );
        assert_eq!(out, Ok("wrote 2 bytes to new.txt".to_string()));
        tools
            .execute(
                &dir,
                WRITE_FILE,
                &json!({ "path": "hello.txt", "content": "bye" }),
            )
            .unwrap();
        assert_eq!(fs::read_to_string(dir.join("new.txt")).unwrap(), "a\n");
        assert_eq!(fs::read_to_string(dir.join("hello.txt")).unwrap(), "bye");
    }

    #[test]
    fn write_file_refuses_outside_workdir_and_missing_parent() {
        let dir = workdir("write-outside");
        let tools = editing();
        let outside = tools.execute(
            &dir,
            WRITE_FILE,
            &json!({ "path": "../escaped.txt", "content": "x" }),
        );
        assert!(
            outside
                .unwrap_err()
                .contains("outside the working directory")
        );
        assert!(!dir.parent().unwrap().join("escaped.txt").exists());
        let absolute = tools.execute(
            &dir,
            WRITE_FILE,
            &json!({ "path": "/tmp/hel-escaped.txt", "content": "x" }),
        );
        assert!(
            absolute
                .unwrap_err()
                .contains("outside the working directory")
        );
        let no_parent = tools.execute(
            &dir,
            WRITE_FILE,
            &json!({ "path": "no/such/dir.txt", "content": "x" }),
        );
        assert!(no_parent.is_err());
    }

    #[test]
    fn write_file_refuses_symlink_pointing_outside() {
        let dir = workdir("write-symlink");
        let outside = dir.parent().unwrap().join("write-symlink-target.txt");
        fs::write(&outside, "keep").unwrap();
        std::os::unix::fs::symlink(&outside, dir.join("link.txt")).unwrap();
        let out = editing().execute(
            &dir,
            WRITE_FILE,
            &json!({ "path": "link.txt", "content": "x" }),
        );
        assert!(out.unwrap_err().contains("outside the working directory"));
        assert_eq!(fs::read_to_string(&outside).unwrap(), "keep");
    }

    #[test]
    fn search_replace_changes_a_unique_match_only() {
        let dir = workdir("replace");
        fs::write(
            dir.join("a.ini"),
            "[a]\nretries = 3\n[b]\nretries = 3\ntimeout = 10\n",
        )
        .unwrap();
        let tools = editing();
        let many = tools.execute(
            &dir,
            SEARCH_REPLACE,
            &json!({ "path": "a.ini", "search": "retries = 3", "replace": "retries = 5" }),
        );
        assert_eq!(
            many,
            Err("search string occurs 2x in a.ini; must be unique".to_string())
        );
        let none = tools.execute(
            &dir,
            SEARCH_REPLACE,
            &json!({ "path": "a.ini", "search": "retries = 9", "replace": "x" }),
        );
        assert_eq!(
            none,
            Err("search string occurs 0x in a.ini; must be unique".to_string())
        );
        let one = tools.execute(&dir, SEARCH_REPLACE, &json!({ "path": "a.ini", "search": "[b]\nretries = 3", "replace": "[b]\nretries = 5" }));
        assert_eq!(one, Ok("replaced 1 occurrence in a.ini".to_string()));
        assert_eq!(
            fs::read_to_string(dir.join("a.ini")).unwrap(),
            "[a]\nretries = 3\n[b]\nretries = 5\ntimeout = 10\n"
        );
    }

    #[test]
    fn search_replace_refuses_empty_search_and_outside_paths() {
        let dir = workdir("replace-refuse");
        let tools = editing();
        let empty = tools.execute(
            &dir,
            SEARCH_REPLACE,
            &json!({ "path": "hello.txt", "search": "", "replace": "x" }),
        );
        assert!(empty.is_err());
        let outside = tools.execute(
            &dir,
            SEARCH_REPLACE,
            &json!({ "path": "../outside-sibling/secret.txt", "search": "s", "replace": "x" }),
        );
        assert!(outside.is_err());
        assert_eq!(
            fs::read_to_string(dir.join("hello.txt")).unwrap(),
            "Hello, harness!\n"
        );
    }

    #[test]
    fn edit_tools_are_recorded_as_edit_and_define_all_arguments() {
        assert_eq!(category(WRITE_FILE), ToolCategory::Edit);
        assert_eq!(category(SEARCH_REPLACE), ToolCategory::Edit);
        let tools = editing();
        let defs = tools.definitions().as_array().unwrap();
        assert_eq!(
            defs[1]["function"]["parameters"]["required"],
            json!(["path", "search", "replace"])
        );
    }
}
