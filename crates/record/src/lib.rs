use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum MetricStatus {
    Measured,
    Unavailable,
    Derived,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ToolCategory {
    Read,
    Search,
    Edit,
    Exec,
    Other,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Termination {
    Completed,
    MaxTurns,
    MaxOutputTokens,
    Timeout,
    Error,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ComparisonClass {
    Subject,
    SameModel,
    Reference,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Metric {
    pub value: Option<f64>,
    pub status: MetricStatus,
}

impl Metric {
    /// A metric the harness could not report. Also the default for fields added after
    /// record-v0, so older records still load.
    pub fn unavailable() -> Self {
        Self {
            value: None,
            status: MetricStatus::Unavailable,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ToolEvent {
    pub seq: u32,
    pub category: ToolCategory,
    pub name: String,
    pub args: serde_json::Value,
    pub ok: Option<bool>,
    /// H13: the exit code of a `bash` call. `None` for other tools, a killed process, a call the
    /// harness refused before running it, and records written before H13.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    /// H13: what a failed call returned, cut by [`error_excerpt`]. `None` when the call
    /// succeeded, the harness does not report it, or the record was written before H13.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Characters of a failed call's result kept in [`ToolEvent::error`].
pub const ERROR_CHARS: usize = 300;

/// The part of a failed call's result kept in the record: the whole text up to [`ERROR_CHARS`],
/// otherwise its end for `bash` (stderr follows stdout there) and its start for other tools.
/// A leading `error: ` added for the model is not part of the result and is dropped.
pub fn error_excerpt(tool: &str, text: &str) -> String {
    let text = text.strip_prefix("error: ").unwrap_or(text);
    let count = text.chars().count();
    if count <= ERROR_CHARS {
        return text.to_string();
    }
    if tool == "bash" {
        let tail: String = text.chars().skip(count - ERROR_CHARS).collect();
        format!("…{tail}")
    } else {
        let head: String = text.chars().take(ERROR_CHARS).collect();
        format!("{head}…")
    }
}

/// The exit code on the first line (`exit=<code>`) of a `bash` result, if there is one.
pub fn bash_exit_code(text: &str) -> Option<i32> {
    let text = text.strip_prefix("error: ").unwrap_or(text);
    text.lines().next()?.strip_prefix("exit=")?.parse().ok()
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Run {
    pub run_id: String,
    pub experiment_id: String,
    pub lab: String,
    pub task_id: String,
    pub condition: String,
    pub repetition: u32,
    pub started_at: String,
    pub ended_at: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Harness {
    pub name: String,
    pub version: String,
    pub comparison_class: ComparisonClass,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Model {
    pub provider: String,
    pub requested: String,
    pub actual: Option<String>,
    pub params: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Outcome {
    pub final_output: Option<String>,
    pub termination: Termination,
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Usage {
    pub input_tokens: Metric,
    pub output_tokens: Metric,
    pub model_calls: Metric,
    pub wall_time_ms: Metric,
    /// Input tokens served from the provider's prompt cache, summed over the run (H5).
    #[serde(default = "Metric::unavailable")]
    pub cached_input_tokens: Metric,
    /// Largest single-request input, i.e. the most context the run used at once (H5).
    #[serde(default = "Metric::unavailable")]
    pub peak_context_tokens: Metric,
    /// Input of the last successful request: the context size when the run ended (H5).
    #[serde(default = "Metric::unavailable")]
    pub last_context_tokens: Metric,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Validity {
    pub valid: bool,
    pub reasons: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Artifacts {
    pub raw_transcript: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Record {
    pub schema_version: String,
    pub run: Run,
    pub harness: Harness,
    pub model: Model,
    pub outcome: Outcome,
    pub usage: Usage,
    pub events: Vec<ToolEvent>,
    pub validity: Validity,
    pub artifacts: Artifacts,
}

/// Run information the runner passes to a harness before a run starts.
/// The harness fills in timestamps and results to produce a [`Record`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RunContext {
    pub run: RunInfo,
    pub harness: Harness,
    pub model: ModelRequest,
    pub budget: Budget,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RunInfo {
    pub run_id: String,
    pub experiment_id: String,
    pub lab: String,
    pub task_id: String,
    pub condition: String,
    pub repetition: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ModelRequest {
    pub provider: String,
    pub requested: String,
    pub params: serde_json::Value,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Budget {
    pub max_turns: u32,
    pub timeout_seconds: u64,
    pub max_output_tokens: u32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tool_event_written_before_h13_loads_without_exit_code_or_error() {
        let json = r#"{"seq": 1, "category": "exec", "name": "bash",
            "args": {"command": "diff a b"}, "ok": false}"#;
        let event: ToolEvent = serde_json::from_str(json).unwrap();
        assert_eq!((event.exit_code, event.error.as_deref()), (None, None));
        let written = serde_json::to_value(&event).unwrap();
        assert!(written.get("exit_code").is_none() && written.get("error").is_none());
    }

    #[test]
    fn error_excerpt_keeps_the_end_of_bash_output_and_the_start_of_other_errors() {
        assert_eq!(
            error_excerpt("read_file", "error: a.txt: missing"),
            "a.txt: missing"
        );
        let long = format!("exit=1\n{}\ncat: illegal option -- A", "x".repeat(400));
        let bash = error_excerpt("bash", &long);
        assert!(bash.starts_with('…') && bash.ends_with("illegal option -- A"));
        assert_eq!(bash.chars().count(), ERROR_CHARS + 1);
        let other = error_excerpt("grep", &"y".repeat(400));
        assert!(other.ends_with('…') && other.starts_with("yyy"));
    }

    #[test]
    fn bash_exit_code_reads_the_first_line() {
        assert_eq!(bash_exit_code("exit=0\nok\n"), Some(0));
        assert_eq!(
            bash_exit_code("error: exit=127\nbash: md5sum: command not found"),
            Some(127)
        );
        assert_eq!(bash_exit_code("exit=killed\n"), None);
        assert_eq!(bash_exit_code("permission denied: bash"), None);
    }

    #[test]
    fn usage_written_before_h5_loads_with_unavailable_context_fields() {
        let measured = r#"{"value": 1.0, "status": "measured"}"#;
        let json = format!(
            r#"{{"input_tokens": {measured}, "output_tokens": {measured},
                "model_calls": {measured}, "wall_time_ms": {measured}}}"#
        );
        let usage: Usage = serde_json::from_str(&json).unwrap();
        assert_eq!(usage.cached_input_tokens, Metric::unavailable());
        assert_eq!(usage.peak_context_tokens, Metric::unavailable());
        assert_eq!(usage.last_context_tokens, Metric::unavailable());
    }
}
