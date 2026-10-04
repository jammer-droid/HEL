//! System message (H3): facts about the execution environment and the contents of a context file
//! in the working directory (`HEL.md` by default), sent before the user's instruction. The
//! harness adds no instructions of its own on how to act on them.

use std::fs;
use std::path::Path;
use std::process::Command;

use serde_json::{Value, json};

/// Context file read from the working directory unless `--no-context-file` is given.
pub const DEFAULT_CONTEXT_FILE: &str = "HEL.md";

/// The system message for the options given, or `None` when there is nothing to send.
/// The context file is read from the working directory only (no parent directories); a missing
/// or empty file adds nothing.
pub fn system_message(env: bool, context_file: Option<&str>, workdir: &Path) -> Option<Value> {
    let mut parts = Vec::new();
    if env {
        parts.push(environment(workdir));
    }
    if let Some(name) = context_file
        && let Ok(text) = fs::read_to_string(workdir.join(name))
        && !text.trim().is_empty()
    {
        parts.push(format!(
            "Contents of {name} in the working directory:\n\n{}",
            text.trim_end()
        ));
    }
    (!parts.is_empty()).then(|| json!({ "role": "system", "content": parts.join("\n\n") }))
}

/// OS, shell and working directory, as found on this machine.
pub fn environment(workdir: &Path) -> String {
    let kernel = output("uname", &["-sr"]).unwrap_or_else(|| "unknown".to_string());
    let shell = output("bash", &["-c", "echo $BASH_VERSION"])
        .map(|version| format!("bash {version}"))
        .unwrap_or_else(|| "bash (version unknown)".to_string());
    format!(
        "Environment:\n- OS: {} ({kernel}, {})\n- Shell: {shell}\n- Working directory: {}",
        std::env::consts::OS,
        std::env::consts::ARCH,
        workdir.display()
    )
}

/// Trimmed stdout of a command that succeeded, or `None`.
fn output(program: &str, args: &[&str]) -> Option<String> {
    let out = Command::new(program).args(args).output().ok()?;
    let text = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (out.status.success() && !text.is_empty()).then_some(text)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn workdir(name: &str, files: &[(&str, &str)]) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join("hel-prompt-test").join(name);
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        for (file, text) in files {
            fs::write(dir.join(file), text).unwrap();
        }
        dir
    }

    #[test]
    fn no_system_message_without_options() {
        assert!(system_message(false, None, Path::new("/tmp")).is_none());
    }

    #[test]
    fn context_file_is_appended_after_environment() {
        let dir = workdir("both", &[("HEL.md", "# Notes\n- rule\n")]);
        let message = system_message(true, Some("HEL.md"), &dir).unwrap();
        let text = message["content"].as_str().unwrap();
        let env_at = text.find("Environment:").unwrap();
        let file_at = text
            .find("Contents of HEL.md in the working directory:\n\n# Notes\n- rule")
            .unwrap();
        assert!(env_at < file_at, "{text}");
    }

    #[test]
    fn context_file_alone_and_missing_file() {
        let dir = workdir("alone", &[("HEL.md", "only notes")]);
        let text = system_message(false, Some("HEL.md"), &dir).unwrap()["content"]
            .as_str()
            .unwrap()
            .to_string();
        assert!(!text.contains("Environment:"), "{text}");
        assert!(text.ends_with("only notes"), "{text}");
        let empty = workdir("missing", &[]);
        assert!(system_message(false, Some("HEL.md"), &empty).is_none());
    }

    #[test]
    fn context_file_is_not_read_from_parent_directories() {
        let parent = workdir("parent", &[("HEL.md", "parent notes")]);
        let child = parent.join("child");
        fs::create_dir_all(&child).unwrap();
        assert!(system_message(false, Some("HEL.md"), &child).is_none());
    }

    #[test]
    fn env_message_names_os_shell_and_workdir() {
        let message = system_message(true, None, Path::new("/work/repo")).unwrap();
        assert_eq!(message["role"], "system");
        let text = message["content"].as_str().unwrap();
        assert!(
            text.contains(&format!("- OS: {} (", std::env::consts::OS)),
            "{text}"
        );
        assert!(text.contains("- Shell: bash "), "{text}");
        assert!(text.contains("- Working directory: /work/repo"), "{text}");
    }
}
