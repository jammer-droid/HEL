//! H4: read-only, on-demand repository search through ripgrep. Limits apply to the
//! text returned to the model; rg still completes the search before formatting.

use std::fs;
use std::path::Path;
use std::process::{Command, Stdio};

use serde_json::{Value, json};

pub const GLOB: &str = "glob";
pub const GREP: &str = "grep";
const MAX_MATCHES: usize = 100;
const MAX_BYTES: usize = 10_000;
const TRUNCATED: &str = "\n[Results truncated. Narrow the path or pattern.]\n";

pub fn definition(name: &str) -> Option<Value> {
    let description = match name {
        GLOB => {
            "Find UTF-8 file paths matching a glob pattern (for example **/*.rs). \
                 path is a directory inside the working directory and defaults to '.'. \
                 Uses rg --files --glob; explicit glob matches override ignore rules. \
                 Does not follow symlinks. \
                 Returns sorted relative paths, up to 100 matches and 10000 UTF-8 bytes, \
                 with a notice if truncated. No matches is a normal result."
        }
        GREP => {
            "Search UTF-8 file contents with a ripgrep regular expression. path is \
                 a file or directory inside the working directory and defaults to '.'. \
                 include optionally filters file paths with a glob (for example *.rs). \
                 Honors ignore rules unless overridden by include; does not follow symlinks. \
                 Returns relative path:line:text, sorted by path, up to 100 matching \
                 lines and 10000 UTF-8 bytes, with a notice if truncated. No matches is \
                 normal; invalid patterns and execution failures are errors."
        }
        _ => return None,
    };
    let mut properties = json!({
        "pattern": { "type": "string", "description": if name == GLOB { "File-name glob pattern." } else { "Regular expression to match within each line." } },
        "path": { "type": "string", "description": "Search path inside the working directory. Omit to use '.'." }
    });
    if name == GREP {
        properties["include"] = json!({
            "type": "string", "description": "Optional file-name glob filter."
        });
    }
    Some(json!({
        "type": "function",
        "function": {
            "name": name,
            "description": description,
            "parameters": {
                "type": "object", "properties": properties,
                "required": ["pattern"], "additionalProperties": false
            }
        }
    }))
}

pub fn execute(workdir: &Path, name: &str, args: &Value) -> Result<String, String> {
    // The loop prefixes errors with "error: ". Bound path/argument errors as well as rg errors.
    execute_search(workdir, name, args)
        .map_err(|error| bounded_text(error, MAX_BYTES - "error: ".len()))
}

fn execute_search(workdir: &Path, name: &str, args: &Value) -> Result<String, String> {
    if !matches!(name, GLOB | GREP) {
        return Err(format!("unknown search tool: {name}"));
    }
    let pattern = optional_string(args, "pattern")?.ok_or("missing string argument: pattern")?;
    let path = optional_string(args, "path")?.unwrap_or(".");
    let include = optional_string(args, "include")?;
    let root = fs::canonicalize(workdir).map_err(|e| format!("working directory: {e}"))?;
    let target = fs::canonicalize(root.join(path)).map_err(|e| format!("{path}: {e}"))?;
    if !target.starts_with(&root) {
        return Err(format!("{path}: outside the working directory"));
    }
    if name == GLOB && !target.is_dir() {
        return Err(format!("{path}: glob path must be a directory"));
    }
    if !target.is_dir() && !target.is_file() {
        return Err(format!("{path}: not a regular file or directory"));
    }
    // Relative search roots keep machine-specific absolute prefixes out of every match.
    let relative = target
        .strip_prefix(&root)
        .expect("target checked inside root");
    let relative = if relative.as_os_str().is_empty() {
        Path::new(".")
    } else {
        relative
    };
    let mut command = Command::new("rg");
    command.args([
        "--no-config",
        "--no-follow",
        "--sort",
        "path",
        "--color",
        "never",
    ]);
    if name == GLOB {
        command.args(["--files", "--null", "--glob", pattern]);
    } else {
        command.args(["--json", "--regexp", pattern]);
        if let Some(include) = include {
            command.args(["--glob", include]);
        }
    }
    let output = command
        .arg("--")
        .arg(relative)
        .current_dir(&root)
        .stdin(Stdio::null())
        .output()
        .map_err(|e| format!("could not run ripgrep: {e}"))?;
    match output.status.code() {
        Some(0) => {}
        Some(1) => return Ok("No matches found.".to_string()),
        code => {
            let error = format!(
                "ripgrep failed ({code:?}): {}",
                String::from_utf8_lossy(&output.stderr).trim()
            );
            return Err(error);
        }
    }
    let mut result = SearchOutput::default();
    if name == GLOB {
        for bytes in output.stdout.split(|b| *b == 0).filter(|s| !s.is_empty()) {
            let path =
                std::str::from_utf8(bytes).map_err(|_| "search returned a non-UTF-8 path")?;
            if !result.push(&display_path(path)) {
                break;
            }
        }
    } else {
        for bytes in output
            .stdout
            .split(|b| *b == b'\n')
            .filter(|s| !s.is_empty())
        {
            let event: Value =
                serde_json::from_slice(bytes).map_err(|e| format!("invalid ripgrep JSON: {e}"))?;
            if event["type"] != "match" {
                continue;
            }
            let data = &event["data"];
            let path = data["path"]["text"]
                .as_str()
                .ok_or("search returned a non-UTF-8 path")?;
            let text = data["lines"]["text"]
                .as_str()
                .ok_or("search returned non-UTF-8 content")?;
            let line = data["line_number"]
                .as_u64()
                .ok_or("search result has no line number")?;
            let text = text.strip_suffix('\n').unwrap_or(text);
            let text = text.strip_suffix('\r').unwrap_or(text);
            if !result.push(&format!("{}:{line}:{text}", display_path(path))) {
                break;
            }
        }
    }
    Ok(result.finish())
}

fn optional_string<'a>(args: &'a Value, key: &str) -> Result<Option<&'a str>, String> {
    match args.get(key) {
        None => Ok(None),
        Some(Value::String(value)) if !value.is_empty() => Ok(Some(value)),
        _ => Err(format!("{key} must be a non-empty string")),
    }
}

fn display_path(path: &str) -> String {
    path.strip_prefix("./")
        .unwrap_or(path)
        .escape_debug()
        .to_string()
}

#[derive(Default)]
struct SearchOutput {
    text: String,
    matches: usize,
    truncated: bool,
}

impl SearchOutput {
    fn push(&mut self, line: &str) -> bool {
        if self.matches == MAX_MATCHES {
            self.truncated = true;
            return false;
        }
        self.matches += 1;
        let remaining = MAX_BYTES - self.text.len();
        if line.len() + 1 > remaining {
            self.text.push_str(utf8_prefix(line, remaining));
            self.truncated = true;
            return false;
        }
        self.text.push_str(line);
        self.text.push('\n');
        true
    }

    fn finish(self) -> String {
        if self.truncated {
            let mut text = utf8_prefix(&self.text, MAX_BYTES - TRUNCATED.len()).to_string();
            text.push_str(TRUNCATED);
            text
        } else if self.matches == 0 {
            "No matches found.".to_string()
        } else {
            self.text
        }
    }
}

fn utf8_prefix(text: &str, limit: usize) -> &str {
    let mut end = limit.min(text.len());
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    &text[..end]
}

fn bounded_text(text: String, limit: usize) -> String {
    if text.len() <= limit {
        return text;
    }
    format!(
        "{}{}",
        utf8_prefix(&text, limit - TRUNCATED.len()),
        TRUNCATED
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::tools::{Toolset, category};
    use record::ToolCategory;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct Workspace(PathBuf);
    impl Workspace {
        fn new() -> Self {
            static NEXT: AtomicUsize = AtomicUsize::new(0);
            let path = std::env::temp_dir().join(format!(
                "hel search {} {}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir_all(&path).unwrap();
            Self(path)
        }
        fn write(&self, path: &str, text: &str) {
            let path = self.0.join(path);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, text).unwrap();
        }
        fn call(&self, name: &str, args: Value) -> Result<String, String> {
            Toolset::new(&[GLOB, GREP])
                .unwrap()
                .execute(&self.0, name, &args)
        }
    }
    impl Drop for Workspace {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn search_tools_find_paths_and_scoped_content_with_optional_arguments() {
        let dir = Workspace::new();
        dir.write("src/a file.rs", "first\nneedle = 7\n");
        dir.write("src/b.rs", "needle = 9\n");
        dir.write("src/b.txt", "needle = 11\n");
        dir.write("docs/note.rs", "needle = 13\n");
        assert_eq!(
            dir.call(GLOB, json!({"pattern":"**/*.rs", "path":"src"}))
                .unwrap(),
            "src/a file.rs\nsrc/b.rs\n"
        );
        assert_eq!(
            dir.call(
                GREP,
                json!({"pattern":"needle", "path":"src", "include":"*.rs"})
            )
            .unwrap(),
            "src/a file.rs:2:needle = 7\nsrc/b.rs:1:needle = 9\n"
        );
        assert_eq!(
            dir.call(GREP, json!({"pattern":"needle", "path":"src/a file.rs"}))
                .unwrap(),
            "src/a file.rs:2:needle = 7\n"
        );
        assert!(
            dir.call(GLOB, json!({"pattern":"**/*.rs"}))
                .unwrap()
                .contains("docs/note.rs")
        );
        assert_eq!(category(GREP), ToolCategory::Search);
        assert_eq!(category(GLOB), ToolCategory::Search);
        let tools = Toolset::new(&[GLOB, GREP]).unwrap();
        let defs = tools.definitions();
        assert_eq!(
            defs[0]["function"]["parameters"]["required"],
            json!(["pattern"])
        );
    }

    #[test]
    fn no_matches_is_success_but_invalid_patterns_and_arguments_are_errors() {
        let dir = Workspace::new();
        dir.write("a.txt", "needle\n");
        assert_eq!(
            dir.call(GREP, json!({"pattern":"absent"})).unwrap(),
            "No matches found."
        );
        assert_eq!(
            dir.call(GLOB, json!({"pattern":"*.missing"})).unwrap(),
            "No matches found."
        );
        assert!(
            dir.call(GREP, json!({"pattern":"["}))
                .unwrap_err()
                .contains("ripgrep failed")
        );
        assert!(dir.call(GLOB, json!({"pattern":"["})).is_err());
        for args in [
            json!({}),
            json!({"pattern":""}),
            json!({"pattern":"a", "path":null}),
            json!({"pattern":"a", "include":9}),
        ] {
            assert!(dir.call(GREP, args).is_err());
        }
        assert!(
            dir.call(GLOB, json!({"pattern":"*", "path":"a.txt"}))
                .is_err()
        );
        assert!(
            dir.call(GREP, json!({"pattern":"a", "path":"missing"}))
                .is_err()
        );
        let long_path = dir
            .call(GREP, json!({"pattern":"a", "path":"한".repeat(6000)}))
            .unwrap_err();
        assert!(long_path.len() + "error: ".len() <= MAX_BYTES);
        assert!(long_path.ends_with(TRUNCATED));
        dir.call(GREP, json!({"pattern":"; touch injected"}))
            .unwrap();
        assert!(!dir.0.join("injected").exists());
    }

    #[test]
    fn search_uses_ripgrep_ignore_precedence_and_refuses_outside_paths_and_symlinks() {
        use std::os::unix::fs::symlink;
        let dir = Workspace::new();
        let outside = Workspace::new();
        outside.write("secret.txt", "needle secret\n");
        fs::create_dir(dir.0.join(".git")).unwrap();
        dir.write(".gitignore", "ignored.txt\n");
        dir.write("ignored.txt", "needle ignored\n");
        dir.write("visible.txt", "needle visible\n");
        symlink(outside.0.join("secret.txt"), dir.0.join("linked.txt")).unwrap();
        assert_eq!(
            dir.call(GREP, json!({"pattern":"needle"})).unwrap(),
            "visible.txt:1:needle visible\n"
        );
        assert_eq!(
            dir.call(GLOB, json!({"pattern":"*.txt"})).unwrap(),
            "ignored.txt\nvisible.txt\n"
        );
        assert_eq!(
            dir.call(GREP, json!({"pattern":"needle", "include":"*.txt"}))
                .unwrap(),
            "ignored.txt:1:needle ignored\nvisible.txt:1:needle visible\n"
        );
        for path in [
            outside.0.to_string_lossy().into_owned(),
            "linked.txt".to_string(),
            "..".to_string(),
        ] {
            assert!(
                dir.call(GREP, json!({"pattern":"needle", "path":path}))
                    .unwrap_err()
                    .contains("outside the working directory")
            );
        }
    }

    #[test]
    fn caps_results_and_marks_truncation_without_breaking_utf8() {
        let dir = Workspace::new();
        dir.write("matches.txt", &"needle\n".repeat(100));
        let exact = dir.call(GREP, json!({"pattern":"needle"})).unwrap();
        assert_eq!(exact.lines().count(), 100);
        assert!(!exact.contains("truncated"));
        dir.write("matches.txt", &"needle\n".repeat(101));
        let limited = dir.call(GREP, json!({"pattern":"needle"})).unwrap();
        assert!(limited.contains("matches.txt:100:needle"));
        assert!(!limited.contains("matches.txt:101:needle"));
        assert!(limited.ends_with(TRUNCATED));
        dir.write("matches.txt", &format!("needle {}\n", "한글".repeat(6000)));
        let long = dir.call(GREP, json!({"pattern":"needle"})).unwrap();
        assert!(long.len() <= MAX_BYTES);
        assert!(long.starts_with("matches.txt:1:needle 한글"));
        assert!(long.ends_with(TRUNCATED));
        for i in 0..101 {
            dir.write(&format!("paths/{i:03}.txt"), "");
        }
        let paths = dir
            .call(GLOB, json!({"pattern":"*.txt", "path":"paths"}))
            .unwrap();
        assert!(paths.contains("paths/099.txt"));
        assert!(!paths.contains("paths/100.txt"));
        assert!(paths.ends_with(TRUNCATED));
    }
}
