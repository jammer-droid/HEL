//! H6: context management after DeepSeek Harness (`dsh-compaction-basic`, `dsh-spill-policy`,
//! `dsh-compaction-tool-result-pruner`, MIT). On by default with DeepSeek Harness's thresholds
//! (`Policy::deepseek_default`); `--compact-at` / `--keep-recent` change them and
//! `--no-compaction` turns it off.
//!
//! Before each model request the context is estimated from the last reported `prompt_tokens`
//! plus a characters/4 estimate of what was appended since. Over the threshold, oversized tool
//! results are pruned first; if that is not enough, the oldest span after the system message is
//! sent to the model for a summary and replaced by it, keeping a recent tail verbatim.

use serde_json::{Value, json};

use crate::api::{ApiError, Exchange};
use crate::output::RunLog;

/// Tool results above this estimate are stored in a file and shown as head and tail.
pub const SPILL_TOKENS: u64 = 12_500;
/// Under pressure, earlier tool results longer than this (in characters) are cut in the middle.
const PRUNE_CHARS: usize = 8_192;
const HEAD_CHARS: usize = 4_096;
const TAIL_CHARS: usize = 1_024;
const PRUNE_MARKER: &str = "\n\n[... tool result middle pruned ...]\n\n";

/// Shown to the conversation model above the summary.
const CHECKPOINT_PREAMBLE: &str = "This is an automatically generated checkpoint condensing an \
earlier span of the conversation to free up context. Treat the captured context as established \
background and build on it without restating it. Continue the task directly from the messages \
that follow, without acknowledging this checkpoint.";

/// Appended as the last user message of the summary request. Adapted from DeepSeek Harness.
const SUMMARY_INSTRUCTION: &str = "You are now acting as a compaction engine for this AI coding \
assistant. Condense the conversation ABOVE into a structured checkpoint that lets another model \
resume the work with no loss of essential context.

Output EXACTLY the Markdown structure below: keep every section, in order. Use terse bullets, not \
prose paragraphs. Write \"(none)\" for an empty section.

## Primary Request and Intent
## Files and Code
## Errors and Fixes
## Pending Jobs
## Current Work
## Next Step
## Critical Context

Rules:
- Preserve exact file paths, commands, identifiers, numeric values and every value the user was \
told or may ask about again.
- Capture user instructions faithfully.
- Do NOT mention this summarization request or that the context was compacted.
- Output only the checkpoint text: do not call any tool.
- If the conversation already contains a <compacted-summary> block, it is a PRIOR checkpoint. \
Merge it with newer information into one consolidated summary under the same structure.";

/// Context window of `deepseek-flash` (and DeepSeek Harness's DeepSeek adapter default).
pub const DEFAULT_WINDOW: u64 = 1_000_000;
/// Room DeepSeek Harness keeps free beyond the output reservation.
const HEADROOM: u64 = 65_536;

/// `--compact-at` and `--keep-recent`, in estimated tokens.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Policy {
    pub at: u64,
    pub keep_recent: u64,
}

impl Policy {
    /// DeepSeek Harness's defaults for a window `W` and output reservation `O`: summarize at
    /// `min(W × 0.8, W − O − 65,536)` and keep the newest `(W − O) × 0.16` verbatim.
    pub fn deepseek_default(window: u64, max_output: u64) -> Self {
        let usable = window.saturating_sub(max_output);
        Self {
            at: (window * 8 / 10).min(usable.saturating_sub(HEADROOM)),
            keep_recent: usable * 16 / 100,
        }
    }
}

/// Remembers the last measured request so later estimates only guess the appended part.
#[derive(Debug, Default, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Meter {
    /// (number of messages sent, prompt_tokens reported for them)
    sent: Option<(usize, u64)>,
}

impl Meter {
    pub fn observed(&mut self, messages_sent: usize, prompt_tokens: u64) {
        self.sent = Some((messages_sent, prompt_tokens));
    }

    /// The history was rewritten; the next estimate starts from scratch.
    pub fn reset(&mut self) {
        self.sent = None;
    }

    pub fn estimate(&self, messages: &[Value]) -> u64 {
        match self.sent {
            Some((sent, tokens)) if sent <= messages.len() => tokens + estimate(&messages[sent..]),
            _ => estimate(messages),
        }
    }
}

/// Characters of the serialized messages divided by 4, rounded up.
pub fn estimate(messages: &[Value]) -> u64 {
    let chars: usize = messages
        .iter()
        .map(|m| serde_json::to_string(m).map_or(0, |s| s.chars().count()))
        .sum();
    (chars as u64).div_ceil(4)
}

fn role(message: &Value) -> &str {
    message.get("role").and_then(Value::as_str).unwrap_or("")
}

/// Keeps the first `HEAD_CHARS` and last `TAIL_CHARS` characters around `marker`.
fn head_and_tail(text: &str, marker: &str) -> String {
    let head: String = text.chars().take(HEAD_CHARS).collect();
    let count = text.chars().count();
    let tail: String = text
        .chars()
        .skip(count.saturating_sub(TAIL_CHARS))
        .collect();
    format!("{head}{marker}{tail}")
}

/// Cuts earlier tool results longer than `PRUNE_CHARS` to head and tail. Returns how many changed.
pub fn prune_tool_results(messages: &mut [Value]) -> usize {
    let mut pruned = 0;
    for message in messages.iter_mut().filter(|m| role(m) == "tool") {
        let Some(content) = message.get("content").and_then(Value::as_str) else {
            continue;
        };
        if content.chars().count() > PRUNE_CHARS {
            message["content"] = json!(head_and_tail(content, PRUNE_MARKER));
            pruned += 1;
        }
    }
    pruned
}

/// Where the verbatim tail starts: the earliest index after `first` whose suffix fits in
/// `keep_recent` and does not begin with a tool result (so a tool call stays with its results).
/// If even the last such boundary is over budget, the shortest valid tail is kept.
/// `None` when nothing before the tail can be summarized.
pub fn tail_start(messages: &[Value], first: usize, keep_recent: u64) -> Option<usize> {
    let mut size = 0;
    let mut within = None;
    let mut shortest = None;
    for i in (first + 1..messages.len()).rev() {
        size += estimate(&messages[i..=i]);
        if role(&messages[i]) == "tool" {
            continue;
        }
        shortest.get_or_insert(i);
        if size <= keep_recent {
            within = Some(i);
        }
    }
    within.or(shortest)
}

/// The summary request: everything before the tail, unchanged, plus the instruction.
pub fn summary_request(messages: &[Value], tail: usize) -> Vec<Value> {
    let mut request = messages[..tail].to_vec();
    request.push(json!({ "role": "user", "content": SUMMARY_INSTRUCTION }));
    request
}

/// The summary text, or why the response cannot be used as one.
pub fn summary_text(exchange: &Exchange) -> Result<String, String> {
    let choice = exchange
        .response
        .choices
        .first()
        .ok_or("summary response has no choices")?;
    let has_calls = choice
        .message
        .get("tool_calls")
        .and_then(Value::as_array)
        .is_some_and(|calls| !calls.is_empty());
    if has_calls {
        return Err("summary response called a tool".into());
    }
    if choice.finish_reason.as_deref() == Some("length") {
        return Err("summary response hit the output limit".into());
    }
    let text = choice
        .message
        .get("content")
        .and_then(Value::as_str)
        .unwrap_or("")
        .trim();
    if text.is_empty() {
        return Err("summary response is empty".into());
    }
    Ok(text.to_string())
}

fn checkpoint(summary: &str) -> Value {
    json!({
        "role": "user",
        "content": format!(
            "{CHECKPOINT_PREAMBLE}\n\n<compacted-summary>\n{summary}\n</compacted-summary>"
        ),
    })
}

/// Index of the first message that may be summarized: after a leading system message.
fn first_compactable(messages: &[Value]) -> usize {
    usize::from(messages.first().is_some_and(|m| role(m) == "system"))
}

/// Runs before a model request. Prunes, then summarizes, when the estimate is over `policy.at`.
/// `summarize` sends one request (the real client in hel, a stub in tests).
pub fn before_request(
    messages: &mut Vec<Value>,
    policy: &Policy,
    meter: &mut Meter,
    log: &mut RunLog,
    summarize: &mut dyn FnMut(&[Value]) -> Result<Exchange, ApiError>,
) {
    let before = meter.estimate(messages);
    if before <= policy.at {
        return;
    }

    if prune_tool_results(messages) > 0 {
        meter.reset();
        let pruned = meter.estimate(messages);
        eprintln!("[compaction] pruned tool results: ~{before} → ~{pruned} tokens");
        if pruned <= policy.at {
            return;
        }
    }

    let first = first_compactable(messages);
    let Some(tail) = tail_start(messages, first, policy.keep_recent) else {
        return;
    };
    let request = summary_request(messages, tail);
    let exchange = match summarize(&request) {
        Ok(exchange) => exchange,
        Err(err) => {
            log.auxiliary_failed("compaction", &err.to_string());
            eprintln!("[compaction] summary request failed: {err}");
            return;
        }
    };
    log.auxiliary(&exchange, "compaction");
    let summary = match summary_text(&exchange) {
        Ok(summary) => summary,
        Err(reason) => {
            log.compaction_note(&reason);
            eprintln!("[compaction] skipped: {reason}");
            return;
        }
    };
    let replacement = checkpoint(&summary);
    let region = &messages[first..tail];
    if estimate(std::slice::from_ref(&replacement)) >= estimate(region) {
        log.compaction_note("summary is not smaller than the span it replaces");
        eprintln!("[compaction] skipped: summary is not smaller than the span");
        return;
    }
    let replaced = tail - first;
    messages.splice(first..tail, [replacement]);
    meter.reset();
    eprintln!(
        "[compaction] {replaced} messages → summary: ~{before} → ~{} tokens",
        meter.estimate(messages)
    );
}

/// Tool results above SPILL_TOKENS are stored by the current run's trusted owner.
pub fn spill(runtime: &crate::runtime::Runtime, content: String) -> String {
    if estimate(&[json!(content)]) <= SPILL_TOKENS {
        return content;
    }
    let Ok(path) = runtime.save_spill(content.as_bytes()) else {
        return content;
    };
    let omitted = content.len();
    let notice = format!(
        "\n\n[... Omitted {omitted} bytes. Full result stored at: {}. Use read_file with start_line/max_lines or its continuation cursor to read parts of it. ...]\n\n",
        path.display()
    );
    head_and_tail(&content, &notice)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::ChatResponse;

    fn msg(role: &str, content: &str) -> Value {
        json!({ "role": role, "content": content })
    }

    fn reply(message: Value, finish: &str) -> Exchange {
        let response_json = json!({
            "model": "deepseek-flash",
            "choices": [{ "message": message, "finish_reason": finish }],
            "usage": { "prompt_tokens": 900, "completion_tokens": 50, "prompt_cache_hit_tokens": 768 },
        });
        Exchange {
            request: json!({}),
            response: serde_json::from_value::<ChatResponse>(response_json.clone()).unwrap(),
            response_json,
        }
    }

    /// system, then three read turns: user, assistant tool call, tool result, assistant answer.
    fn session() -> Vec<Value> {
        let mut messages = vec![msg("system", "env")];
        for (file, port) in [("alpha", "7310"), ("bravo", "7420"), ("charlie", "7530")] {
            messages.push(msg("user", &format!("Read {file}.toml")));
            messages.push(json!({ "role": "assistant", "content": "", "tool_calls": [
                { "id": file, "type": "function", "function": { "name": "bash", "arguments": "{}" } }
            ]}));
            messages.push(json!({ "role": "tool", "tool_call_id": file,
                "content": format!("port = {port}\n{}", "x".repeat(2_000)) }));
            messages.push(msg("assistant", port));
        }
        messages
    }

    #[test]
    fn default_policy_follows_deepseek_harness() {
        // deepseek-flash with hel's 8,192-token output limit.
        assert_eq!(
            Policy::deepseek_default(DEFAULT_WINDOW, 8_192),
            Policy {
                at: 800_000,
                keep_recent: 158_689
            }
        );
        // A small window is bounded by the output reservation and headroom instead.
        assert_eq!(Policy::deepseek_default(128_000, 8_000).at, 54_464);
    }

    #[test]
    fn meter_adds_an_estimate_of_what_was_appended() {
        let messages = vec![msg("user", "hi"), msg("assistant", &"a".repeat(400))];
        let mut meter = Meter::default();
        assert_eq!(meter.estimate(&messages), estimate(&messages));
        meter.observed(1, 1000);
        assert_eq!(meter.estimate(&messages), 1000 + estimate(&messages[1..]));
        meter.reset();
        assert_eq!(meter.estimate(&messages), estimate(&messages));
    }

    #[test]
    fn prunes_only_long_tool_results() {
        let mut messages = vec![
            msg("tool", &"t".repeat(PRUNE_CHARS + 1)),
            msg("tool", "short"),
            msg("user", &"u".repeat(PRUNE_CHARS + 1)),
        ];
        assert_eq!(prune_tool_results(&mut messages), 1);
        let pruned = messages[0]["content"].as_str().unwrap();
        assert_eq!(
            pruned.chars().count(),
            HEAD_CHARS + PRUNE_MARKER.chars().count() + TAIL_CHARS
        );
        assert!(pruned.contains("[... tool result middle pruned ...]"));
        assert_eq!(messages[1]["content"], "short");
        assert_eq!(prune_tool_results(&mut messages), 0, "pruning converges");
    }

    #[test]
    fn tail_never_starts_with_a_tool_result() {
        let messages = session();
        // A budget for about one tool result keeps the last answer only… or the last call group.
        let tail = tail_start(&messages, 1, 700).unwrap();
        assert_ne!(role(&messages[tail]), "tool");
        assert!(tail > 1);
        // Too small for anything: the shortest valid tail (the last assistant answer) is kept.
        assert_eq!(tail_start(&messages, 1, 1), Some(messages.len() - 1));
        // A huge budget leaves nothing to summarize except the first user message.
        assert_eq!(tail_start(&messages, 1, 1_000_000), Some(2));
    }

    #[test]
    fn summary_request_keeps_the_prefix_unchanged() {
        let messages = session();
        let request = summary_request(&messages, 9);
        assert_eq!(&request[..9], &messages[..9]);
        assert!(
            request[9]["content"]
                .as_str()
                .unwrap()
                .contains("compaction engine")
        );
    }

    #[test]
    fn rejects_summaries_that_call_tools_or_are_cut() {
        let tool_call =
            json!({ "role": "assistant", "content": "", "tool_calls": [{ "id": "x" }] });
        assert!(summary_text(&reply(tool_call, "tool_calls")).is_err());
        assert!(summary_text(&reply(msg("assistant", "half"), "length")).is_err());
        assert!(summary_text(&reply(msg("assistant", "  "), "stop")).is_err());
        assert_eq!(
            summary_text(&reply(
                msg("assistant", " ## Current Work\n- port 7530 "),
                "stop"
            )),
            Ok("## Current Work\n- port 7530".to_string())
        );
    }

    #[test]
    fn replaces_the_oldest_span_and_keeps_system_and_tail() {
        let mut messages = session();
        let last = messages.last().unwrap().clone();
        let mut meter = Meter::default();
        let mut log = RunLog::new();
        let mut seen = Vec::new();
        before_request(
            &mut messages,
            &Policy {
                at: 100,
                keep_recent: 700,
            },
            &mut meter,
            &mut log,
            &mut |request| {
                seen.push(request.to_vec());
                Ok(reply(
                    msg("assistant", "## Current Work\n- ports 7310 7420 7530"),
                    "stop",
                ))
            },
        );
        assert_eq!(seen.len(), 1, "one summary request");
        assert_eq!(messages[0], msg("system", "env"));
        let checkpoint = messages[1]["content"].as_str().unwrap();
        assert!(checkpoint.contains("<compacted-summary>\n## Current Work"));
        assert_eq!(messages.last(), Some(&last));
        assert!(messages.len() < session().len());
    }

    #[test]
    fn leaves_history_alone_under_the_threshold_or_when_the_summary_fails() {
        let original = session();
        let mut meter = Meter::default();
        let mut log = RunLog::new();

        let mut messages = original.clone();
        let mut calls = 0;
        before_request(
            &mut messages,
            &Policy {
                at: 1_000_000,
                keep_recent: 700,
            },
            &mut meter,
            &mut log,
            &mut |_| {
                calls += 1;
                Err(ApiError::Http("unused".into()))
            },
        );
        assert_eq!((calls, &messages), (0, &original));

        before_request(
            &mut messages,
            &Policy {
                at: 100,
                keep_recent: 700,
            },
            &mut meter,
            &mut log,
            &mut |_| {
                Ok(reply(
                    json!({ "role": "assistant", "content": "", "tool_calls": [{ "id": "x" }] }),
                    "tool_calls",
                ))
            },
        );
        assert_eq!(messages, original, "a tool-calling summary is not used");
    }

    #[test]
    fn spills_large_results_to_the_current_runs_private_file() {
        use std::fs;
        use std::os::unix::fs::PermissionsExt;
        let temp = std::env::temp_dir().join(format!("hel-spill-test-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&temp).unwrap();
        let runtime = crate::runtime::Runtime::new_in(&temp, &temp.join("runs")).unwrap();
        assert_eq!(spill(&runtime, "small".into()), "small");
        let big = format!("start{}end", "z".repeat(60_000));
        let shown = spill(&runtime, big.clone());
        assert!(shown.starts_with("start") && shown.ends_with("end"));
        let file = fs::read_dir(&runtime.spill)
            .unwrap()
            .next()
            .unwrap()
            .unwrap()
            .path();
        assert_eq!(fs::read_to_string(&file).unwrap(), big);
        assert_eq!(
            fs::metadata(&file).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert_eq!(
            fs::metadata(&runtime.spill).unwrap().permissions().mode() & 0o777,
            0o700
        );
        assert!(shown.contains(file.to_str().unwrap()));
        drop(runtime);
        fs::remove_dir_all(temp).unwrap();
    }
}
