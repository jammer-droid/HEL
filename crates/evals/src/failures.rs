//! H13: what kind of failure a failed tool call is. `hel` records the facts of each call
//! (`exit_code`, `error`, eval-v14); the kind is judged here, so the rule can change and stored
//! runs be judged again without running the model. Records written before H13 have no facts;
//! they are read from the tool messages in raw/requests.jsonl instead.

use std::collections::{BTreeMap, HashMap};
use std::error::Error;
use std::fs;
use std::path::Path;

use record::{Record, ToolEvent};
use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Kind {
    /// The exit code was not 0 but the command did its job (`diff` found a difference).
    IntendedExit,
    /// An unknown command or option, a syntax error, a step or tool use that failed.
    Command,
    /// A path that does not exist, is not a regular file, or is not a relative path.
    Path,
    /// The harness refused the call: permission, sandbox boundary, hook, delegation depth.
    Policy,
    /// A program the harness depends on is missing or could not start.
    Environment,
}

impl Kind {
    pub const ALL: [Kind; 5] = [
        Kind::IntendedExit,
        Kind::Command,
        Kind::Path,
        Kind::Policy,
        Kind::Environment,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Kind::IntendedExit => "intended-exit",
            Kind::Command => "command",
            Kind::Path => "path",
            Kind::Policy => "policy",
            Kind::Environment => "environment",
        }
    }

    pub fn parse(name: &str) -> Option<Kind> {
        Kind::ALL.into_iter().find(|k| k.name() == name)
    }
}

const DELEGATE_TASK: &str = "delegate_task";

/// Programs whose exit code 1 is an answer, not an error: a difference, no match, a false test.
const ANSWERS_WITH_EXIT_1: [&str; 9] = [
    "diff", "cmp", "grep", "egrep", "fgrep", "rg", "test", "[", "[[",
];

/// The kind of a failed call from what it returned (`error`) and, for `bash`, its exit code and
/// the commands of its last statement (rule v2, H13 revision 2).
pub fn kind(name: &str, args: &Value, exit_code: Option<i32>, error: &str) -> Kind {
    let has = |text: &str| error.contains(text);
    if error.starts_with("permission denied")
        || has("hook blocked")
        || has("outside the working directory")
        || has("Delegation depth limit")
    {
        return Kind::Policy;
    }
    if has("program not found") || has("could not start bash") {
        return Kind::Environment;
    }
    if name == "bash" {
        let command = args.get("command").and_then(Value::as_str).unwrap_or("");
        let answers = deciding_programs(command)
            .iter()
            .any(|program| ANSWERS_WITH_EXIT_1.contains(&program.as_str()));
        return if exit_code == Some(1) && answers {
            Kind::IntendedExit
        } else {
            Kind::Command
        };
    }
    if has("No such file or directory")
        || has("not a regular file")
        || has("Is a directory")
        || has("expected a relative")
    {
        return Kind::Path;
    }
    Kind::Command
}

/// The programs that can have set the exit code of `command`: in its last statement (after the
/// last `;` or newline outside quotes), the last program of each pipeline joined by `&&` or `||`
/// (a pipeline's exit code is its last program's; `A && B` ends with A's code when A fails).
/// A last statement `exit $var` is replaced by the statement before `var=$?`.
fn deciding_programs(command: &str) -> Vec<String> {
    let statements = statements(command);
    let Some(mut last) = statements.len().checked_sub(1) else {
        return Vec::new();
    };
    let words = |s: &Statement| s.iter().flatten().flatten().cloned().collect::<Vec<_>>();
    if let [exit, var] = words(&statements[last]).as_slice()
        && exit == "exit"
    {
        let var = var.trim_matches('"').trim_start_matches('$');
        let saved = format!("{var}=$?");
        if let Some(i) = statements
            .iter()
            .position(|s| words(s).first() == Some(&saved))
            && i > 0
        {
            last = i - 1;
        }
    }
    statements[last]
        .iter()
        .filter_map(|pipeline| pipeline.last())
        .filter_map(|command| {
            command
                .iter()
                .find(|word| *word != "!" && !word.contains('='))
                .cloned()
        })
        .collect()
}

/// A statement: pipelines joined by `&&` / `||`; a pipeline: commands joined by `|`; a command:
/// its words.
type Statement = Vec<Vec<Vec<String>>>;

/// Splits a shell command into statements at `;` and newlines outside quotes. Quotes are kept
/// inside words; this is enough to find program names, not a shell parser.
fn statements(command: &str) -> Vec<Statement> {
    let mut statements: Vec<Statement> = Vec::new();
    let mut statement: Statement = vec![vec![Vec::new()]];
    let mut word = String::new();
    let mut quote: Option<char> = None;
    let mut chars = command.chars().peekable();
    let end_word = |word: &mut String, statement: &mut Statement| {
        if !word.is_empty() {
            statement
                .last_mut()
                .unwrap()
                .last_mut()
                .unwrap()
                .push(std::mem::take(word));
        }
    };
    while let Some(c) = chars.next() {
        if let Some(q) = quote {
            word.push(c);
            if c == q {
                quote = None;
            }
            continue;
        }
        match c {
            '\'' | '"' => {
                quote = Some(c);
                word.push(c);
            }
            ' ' | '\t' => end_word(&mut word, &mut statement),
            ';' | '\n' => {
                end_word(&mut word, &mut statement);
                statements.push(std::mem::replace(&mut statement, vec![vec![Vec::new()]]));
            }
            '&' | '|' if chars.peek() == Some(&c) => {
                chars.next();
                end_word(&mut word, &mut statement);
                statement.push(vec![Vec::new()]);
            }
            '|' => {
                end_word(&mut word, &mut statement);
                statement.last_mut().unwrap().push(Vec::new());
            }
            _ => word.push(c),
        }
    }
    end_word(&mut word, &mut statement);
    statements.push(statement);
    statements.retain(|s| s.iter().flatten().any(|c| !c.is_empty()));
    for s in &mut statements {
        for pipeline in s.iter_mut() {
            pipeline.retain(|c| !c.is_empty());
        }
        s.retain(|p| !p.is_empty());
    }
    statements
}

/// Whose call it was: the run's own agent or a delegated child.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Agent {
    Parent,
    Child,
}

impl Agent {
    pub fn name(self) -> &'static str {
        match self {
            Agent::Parent => "parent",
            Agent::Child => "child",
        }
    }
}

/// One failed tool call of a run.
#[derive(Debug, Clone)]
pub struct Failed {
    pub agent: Agent,
    /// Position among the agent's calls; children of one run are numbered in delegation order.
    pub seq: u32,
    pub name: String,
    pub args: Value,
    pub exit_code: Option<i32>,
    /// `None` when neither the record nor the raw log has what the call returned.
    pub error: Option<String>,
    /// `true` when the facts came from raw/requests.jsonl (a record written before H13).
    pub from_raw: bool,
    /// For a failed `delegate_task`: the kind of the child's last failed call, when the child
    /// made one that can be judged. The child's failure explains the parent's (rule v2).
    pub child_kind: Option<Kind>,
}

impl Failed {
    /// `None` when there is nothing to judge (no `error`).
    pub fn kind(&self) -> Option<Kind> {
        self.child_kind.or_else(|| {
            self.error
                .as_deref()
                .map(|error| kind(&self.name, &self.args, self.exit_code, error))
        })
    }
}

/// The failed calls of the run stored in `run_dir`: the agent's own calls from the record, and
/// delegated children's calls from raw/delegations.jsonl. Records written before H13 get their
/// facts from raw/requests.jsonl: the parent's from its requests, each child's from its
/// `"agent": "child"` requests when the children ran one after another.
pub fn of_run(run_dir: &Path, record: &Record) -> Vec<Failed> {
    let raw: Vec<Value> = fs::read_to_string(run_dir.join("raw/requests.jsonl"))
        .unwrap_or_default()
        .lines()
        .filter_map(|line| serde_json::from_str(line).ok())
        .collect();
    let parent_raw: Vec<&Value> = raw
        .iter()
        .filter(|e| e.get("agent").is_none() && e.get("purpose").is_none())
        .collect();
    let mut failed = failed_calls(Agent::Parent, 0, &record.events, &parent_raw);

    let traces: Vec<Value> = fs::read_to_string(run_dir.join("raw/delegations.jsonl"))
        .unwrap_or_default()
        .lines()
        .filter_map(|line| serde_json::from_str(line).ok())
        .collect();
    let child_raw: Vec<&Value> = raw
        .iter()
        .filter(|e| e.get("agent").and_then(Value::as_str) == Some("child"))
        .collect();
    let sequential = !overlapping(&traces);
    let mut child_kinds = Vec::new();
    let (mut child_seq, mut raw_offset) = (0, 0);
    for trace in &traces {
        let events: Vec<ToolEvent> = trace
            .get("child_events")
            .cloned()
            .and_then(|v| serde_json::from_value(v).ok())
            .unwrap_or_default();
        let requests = trace
            .get("child_requests")
            .and_then(Value::as_array)
            .map_or(0, Vec::len);
        let own_raw: Vec<&Value> = if sequential {
            child_raw
                .iter()
                .skip(raw_offset)
                .take(requests)
                .copied()
                .collect()
        } else {
            Vec::new()
        };
        raw_offset += requests;
        let child = failed_calls(Agent::Child, child_seq, &events, &own_raw);
        child_seq += events.len() as u32;
        child_kinds.push(child.iter().rev().find_map(Failed::kind));
        failed.extend(child);
    }
    // A run's delegate_task calls and its delegation traces are in the same order.
    let delegate_seqs: Vec<u32> = record
        .events
        .iter()
        .filter(|e| e.name == DELEGATE_TASK)
        .map(|e| e.seq)
        .collect();
    for f in failed.iter_mut().filter(|f| f.agent == Agent::Parent) {
        if let Some(i) = delegate_seqs.iter().position(|seq| *seq == f.seq) {
            f.child_kind = child_kinds.get(i).copied().flatten();
        }
    }
    failed
}

/// The failed calls among `events`, numbered from `first_seq + 1` for children. Calls without
/// recorded facts take them from `raw` (requests of the same agent).
fn failed_calls(agent: Agent, first_seq: u32, events: &[ToolEvent], raw: &[&Value]) -> Vec<Failed> {
    let legacy = events
        .iter()
        .any(|e| e.ok == Some(false) && e.error.is_none());
    let from_raw = if legacy {
        facts_from_raw(raw, events)
    } else {
        Vec::new()
    };
    events
        .iter()
        .enumerate()
        .filter(|(_, e)| e.ok == Some(false))
        .map(|(i, e)| {
            let raw_facts = from_raw.get(i).cloned().flatten();
            let use_raw = e.error.is_none() && raw_facts.is_some();
            let (exit_code, error) = match raw_facts {
                Some(facts) if use_raw => facts,
                _ => (e.exit_code, e.error.clone()),
            };
            Failed {
                agent,
                seq: match agent {
                    Agent::Parent => e.seq,
                    Agent::Child => first_seq + i as u32 + 1,
                },
                name: e.name.clone(),
                args: e.args.clone(),
                exit_code,
                error,
                from_raw: use_raw,
                child_kind: None,
            }
        })
        .collect()
}

/// Whether any two children ran at the same time (H12 `--parallel`), from their start and end
/// times. Traces without times come from sequential delegation.
fn overlapping(traces: &[Value]) -> bool {
    let spans: Vec<(u64, u64)> = traces
        .iter()
        .filter_map(|t| Some((t["started_at_ms"].as_u64()?, t["ended_at_ms"].as_u64()?)))
        .collect();
    spans
        .iter()
        .enumerate()
        .any(|(i, a)| spans[i + 1..].iter().any(|b| a.0 < b.1 && b.0 < a.1))
}

/// `evals failures [lab] [--labels <tsv>]`: one tab-separated line per failed tool call of the
/// stored runs (all of results/ without a Lab), with the kind this rule assigns. With `labels`,
/// compares the rule with a hand classification (`run_id`, `agent`, `seq`, `kind` per line);
/// a `?` kind marks a call the hand classification could not judge and is left out.
pub fn list(root: &Path, lab: Option<&str>, labels: Option<&Path>) -> Result<(), Box<dyn Error>> {
    let results = root.join("results");
    let labs: Vec<_> = match lab {
        Some(lab) => vec![results.join(lab)],
        None => sorted_dirs(&results)?,
    };
    let mut rows = Vec::new();
    for lab_dir in labs {
        for run_dir in sorted_dirs(&lab_dir)? {
            let Ok(text) = fs::read_to_string(run_dir.join("record.json")) else {
                continue;
            };
            let Ok(record) = serde_json::from_str::<Record>(&text) else {
                continue;
            };
            for failed in of_run(&run_dir, &record) {
                rows.push((record.run.run_id.clone(), failed));
            }
        }
    }
    let Some(labels) = labels else {
        println!("run_id\tagent\tseq\ttool\texit\tkind\tsource\ttarget\terror");
        for (run_id, f) in &rows {
            println!(
                "{run_id}\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}",
                f.agent.name(),
                f.seq,
                f.name,
                f.exit_code.map_or("-".to_string(), |c| c.to_string()),
                f.kind().map_or("?", Kind::name),
                if f.error.is_none() {
                    "-"
                } else if f.from_raw {
                    "raw"
                } else {
                    "record"
                },
                one_line(target(&f.args), 80),
                one_line(f.error.as_deref().unwrap_or(""), 120),
            );
        }
        return Ok(());
    };
    let by_key: HashMap<(String, String, u32), &Failed> = rows
        .iter()
        .map(|(run_id, f)| ((run_id.clone(), f.agent.name().to_string(), f.seq), f))
        .collect();
    let mut agree = 0;
    let mut judged = 0;
    let mut excluded = 0;
    let mut missing = Vec::new();
    let mut disagreements = Vec::new();
    let mut per_label: BTreeMap<&'static str, (usize, usize)> = BTreeMap::new();
    let (mut intended_as_failure, mut failure_as_intended) = (0, 0);
    let text = fs::read_to_string(labels)?;
    for line in text.lines() {
        let cells: Vec<&str> = line.split('\t').collect();
        if line.starts_with('#') || cells.first() == Some(&"run_id") || cells.len() < 4 {
            continue;
        }
        if cells[3].trim() == "?" {
            excluded += 1;
            continue;
        }
        let label = Kind::parse(cells[3].trim())
            .ok_or(format!("unknown kind {:?} in {line:?}", cells[3]))?;
        let seq: u32 = cells[2].trim().parse()?;
        let key = (cells[0].to_string(), cells[1].to_string(), seq);
        let Some(failed) = by_key.get(&key) else {
            missing.push(line.to_string());
            continue;
        };
        let entry = per_label.entry(label.name()).or_default();
        entry.1 += 1;
        let Some(rule) = failed.kind() else {
            disagreements.push(format!(
                "{}\t{}\t{}\t{} → ?",
                key.0,
                key.1,
                key.2,
                label.name()
            ));
            continue;
        };
        judged += 1;
        if rule == label {
            agree += 1;
            entry.0 += 1;
        } else {
            if label == Kind::IntendedExit {
                intended_as_failure += 1;
            }
            if rule == Kind::IntendedExit {
                failure_as_intended += 1;
            }
            disagreements.push(format!(
                "{}\t{}\t{}\t{} → {}",
                key.0,
                key.1,
                key.2,
                label.name(),
                rule.name()
            ));
        }
    }
    let labeled: usize = per_label.values().map(|(_, total)| total).sum();
    let percent = |n: usize, d: usize| {
        if d == 0 {
            0.0
        } else {
            100.0 * n as f64 / d as f64
        }
    };
    println!(
        "labeled {labeled} · judged by the rule {judged} · agree {agree} ({:.1}% of labeled) · \
         excluded as unjudgeable by hand {excluded}",
        percent(agree, labeled)
    );
    println!("intended exit counted as a failure: {intended_as_failure}");
    println!("failure counted as an intended exit: {failure_as_intended}");
    for (label, (ok, total)) in &per_label {
        println!("  {label}: {ok}/{total}");
    }
    for line in &disagreements {
        println!("  differs: {line}");
    }
    for line in &missing {
        println!("  not found in results: {line}");
    }
    Ok(())
}

fn sorted_dirs(dir: &Path) -> Result<Vec<std::path::PathBuf>, Box<dyn Error>> {
    let mut dirs: Vec<_> = fs::read_dir(dir)?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.is_dir())
        .collect();
    dirs.sort();
    Ok(dirs)
}

/// The main argument of a call: its command or path, else all arguments.
fn target(args: &Value) -> String {
    ["command", "path", "pattern", "task"]
        .iter()
        .find_map(|key| args.get(key).and_then(Value::as_str))
        .map(str::to_string)
        .unwrap_or_else(|| args.to_string())
}

fn one_line(text: impl AsRef<str>, max: usize) -> String {
    let line = text.as_ref().replace(['\n', '\t'], " ⏎ ");
    if line.chars().count() > max {
        format!("{}…", line.chars().take(max).collect::<String>())
    } else {
        line
    }
}

/// Facts of each event read from one agent's requests in raw/requests.jsonl, for records written
/// before H13. The agent's tool calls are collected in order from its assistant messages and
/// matched to the events by position; a call is used only when its name and arguments equal the
/// event's. The caller passes only that agent's requests.
fn facts_from_raw(
    raw: &[&Value],
    events: &[ToolEvent],
) -> Vec<Option<(Option<i32>, Option<String>)>> {
    let mut calls: Vec<(String, String, Value)> = Vec::new();
    let mut results: HashMap<String, String> = HashMap::new();
    for entry in raw {
        let messages = entry
            .pointer("/request/messages")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        for message in messages {
            match message.get("role").and_then(Value::as_str) {
                Some("assistant") => {
                    for call in message
                        .get("tool_calls")
                        .and_then(Value::as_array)
                        .into_iter()
                        .flatten()
                    {
                        let id = call["id"].as_str().unwrap_or_default().to_string();
                        if calls.iter().any(|(seen, _, _)| *seen == id) {
                            continue;
                        }
                        let name = call["function"]["name"].as_str().unwrap_or_default();
                        let args = call["function"]["arguments"]
                            .as_str()
                            .and_then(|a| serde_json::from_str(a).ok())
                            .unwrap_or(Value::Null);
                        calls.push((id, name.to_string(), args));
                    }
                }
                Some("tool") => {
                    if let (Some(id), Some(content)) = (
                        message.get("tool_call_id").and_then(Value::as_str),
                        message.get("content").and_then(Value::as_str),
                    ) {
                        results.insert(id.to_string(), content.to_string());
                    }
                }
                _ => {}
            }
        }
    }
    events
        .iter()
        .enumerate()
        .map(|(i, event)| {
            let (id, name, args) = calls.get(i)?;
            if *name != event.name || *args != event.args {
                return None;
            }
            let content = results.get(id)?;
            let exit_code = (name == "bash")
                .then(|| record::bash_exit_code(content))
                .flatten();
            let error = (event.ok == Some(false)).then(|| record::error_excerpt(name, content));
            Some((exit_code, error))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn bash(command: &str, exit: i32, error: &str) -> Kind {
        kind("bash", &json!({ "command": command }), Some(exit), error)
    }

    #[test]
    fn exit_1_of_a_comparing_or_searching_program_is_intended() {
        assert_eq!(
            bash("diff a.txt b.txt", 1, "exit=1\n< one"),
            Kind::IntendedExit
        );
        assert_eq!(
            bash("cp a b && diff -u a b", 1, "exit=1\n"),
            Kind::IntendedExit
        );
        assert_eq!(
            bash("grep -n TODO src/*.rs", 1, "exit=1\n"),
            Kind::IntendedExit
        );
        assert_eq!(bash("md5 -q x || md5sum x", 1, "exit=1\n"), Kind::Command);
        assert_eq!(bash("diff a b", 2, "exit=2\nNo such file"), Kind::Command);
    }

    #[test]
    fn rule_v2_reads_the_and_or_chain_and_exit_status_of_the_last_statement() {
        let compare = r#"python3 -c "print('ok')"; echo "---"; diff <(grep -v X a) <(grep -v X b) && echo "same""#;
        assert_eq!(bash(compare, 1, "exit=1\n35d34"), Kind::IntendedExit);
        let saved = "tmp=$(mktemp); diff -u \"$tmp\" s.py; status=$?; rm -f \"$tmp\"; grep -n X s.py; exit $status";
        assert_eq!(bash(saved, 1, "exit=1\n@@"), Kind::IntendedExit);
        assert_eq!(bash("diff a b | head -5", 1, "exit=1\n"), Kind::Command);
        assert_eq!(bash("echo 'a;diff'", 1, "exit=1\n"), Kind::Command);
        assert_eq!(
            bash(
                "ls -la config && echo '--- x ---' && cat -A config/x",
                1,
                "exit=1\ncat: illegal option -- A"
            ),
            Kind::Command
        );
    }

    #[test]
    fn statements_split_at_semicolons_and_newlines_outside_quotes() {
        let s = statements("awk '\n /x/ { a; b }\n' f > g\necho \"a;b\"; x=1 && y | z");
        let programs: Vec<String> = s.iter().map(|st| st[0][0][0].clone()).collect();
        assert_eq!(programs, ["awk", "echo", "x=1"]);
        assert_eq!(s[2].len(), 2);
        assert_eq!(s[2][1].len(), 2);
    }

    #[test]
    fn other_bash_failures_are_command_failures() {
        assert_eq!(
            bash(
                "md5sum data.txt",
                127,
                "exit=127\nbash: md5sum: command not found"
            ),
            Kind::Command
        );
        assert_eq!(
            bash("cat -A settings.py", 1, "exit=1\ncat: illegal option -- A"),
            Kind::Command
        );
    }

    #[test]
    fn harness_refusals_and_missing_programs_come_before_the_tool() {
        let args = json!({ "command": "printf done > status.txt" });
        assert_eq!(
            kind(
                "bash",
                &args,
                None,
                "permission denied: bash (access=ReadOnly, approval=None)"
            ),
            Kind::Policy
        );
        assert_eq!(
            kind(
                "search_replace",
                &json!({}),
                None,
                "PreToolUse hook blocked search_replace: read first"
            ),
            Kind::Policy
        );
        assert_eq!(
            kind("glob", &json!({}), None, "program not found: rg"),
            Kind::Environment
        );
        assert_eq!(
            kind(
                "delegate_task",
                &json!({}),
                None,
                "the subagent ended without a final answer (program not found: rg)"
            ),
            Kind::Environment
        );
    }

    #[test]
    fn file_tools_on_missing_or_wrong_paths_are_path_failures() {
        let read = |error| kind("read_file", &json!({ "path": "x" }), None, error);
        assert_eq!(read("No such file or directory (os error 2)"), Kind::Path);
        assert_eq!(read("not a regular file"), Kind::Path);
        assert_eq!(read("expected a relative file path"), Kind::Path);
        assert_eq!(read(".: Is a directory (os error 21)"), Kind::Path);
        assert_eq!(
            kind(
                "search_replace",
                &json!({}),
                None,
                "search string occurs 0x"
            ),
            Kind::Command
        );
    }

    #[test]
    fn raw_facts_match_the_parent_calls_by_position_name_and_arguments() {
        let request = |messages: Value| json!({ "request": { "messages": messages } });
        let call = |id: &str, name: &str, args: Value| {
            json!({"role": "assistant", "tool_calls": [{"id": id, "type": "function",
                "function": {"name": name, "arguments": args.to_string()}}]})
        };
        let raw = [
            request(json!([{"role": "user", "content": "go"}])),
            request(json!([
                {"role": "user", "content": "go"},
                call("a", "bash", json!({"command": "diff x y"})),
                {"role": "tool", "tool_call_id": "a", "content": "error: exit=1\n< x"}
            ])),
            json!({"agent": "child", "request": {"messages": [
                call("c", "read_file", json!({"path": "child.txt"})),
                {"role": "tool", "tool_call_id": "c", "content": "error: missing"}
            ]}}),
            request(json!([
                {"role": "user", "content": "go"},
                call("a", "bash", json!({"command": "diff x y"})),
                {"role": "tool", "tool_call_id": "a", "content": "error: exit=1\n< x"},
                call("b", "read_file", json!({"path": "gone.txt"})),
                {"role": "tool", "tool_call_id": "b", "content": "error: No such file or directory (os error 2)"}
            ])),
        ];
        let parent: Vec<&Value> = raw.iter().filter(|e| e.get("agent").is_none()).collect();
        let event = |seq, name: &str, args: Value| ToolEvent {
            seq,
            category: record::ToolCategory::Other,
            name: name.to_string(),
            args,
            ok: Some(false),
            exit_code: None,
            error: None,
        };
        let events = [
            event(1, "bash", json!({"command": "diff x y"})),
            event(2, "read_file", json!({"path": "gone.txt"})),
            event(3, "read_file", json!({"path": "never-sent.txt"})),
        ];
        let facts = facts_from_raw(&parent, &events);
        assert_eq!(facts[0], Some((Some(1), Some("exit=1\n< x".to_string()))));
        assert_eq!(
            facts[1],
            Some((
                None,
                Some("No such file or directory (os error 2)".to_string())
            ))
        );
        assert_eq!(facts[2], None);
    }

    /// A run directory with one failed delegation whose child failed on a missing program.
    fn delegated_run(child_facts: bool) -> (std::path::PathBuf, Record) {
        let dir = std::env::temp_dir().join(format!("evals-failures-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(dir.join("raw")).unwrap();
        let mut record: Record = serde_json::from_str(include_str!(
            "../../../evals/schema/examples/record-v0.json"
        ))
        .unwrap();
        let delegate = |ok| ToolEvent {
            seq: 1,
            category: record::ToolCategory::Other,
            name: "delegate_task".to_string(),
            args: json!({"task": "find it"}),
            ok: Some(ok),
            exit_code: None,
            error: child_facts
                .then(|| "the subagent ended without a final answer (MaxTurns)".to_string()),
        };
        record.events = vec![delegate(false)];
        let child_event = json!({"seq": 1, "category": "search", "name": "glob",
            "args": {"pattern": "**/*"}, "ok": false});
        let mut child_event_with_facts = child_event.clone();
        child_event_with_facts["error"] = json!("program not found: rg");
        let trace = json!({"seq": 1, "child_requests": [{}, {}],
            "child_events": [if child_facts { child_event_with_facts } else { child_event }]});
        fs::write(dir.join("raw/delegations.jsonl"), format!("{trace}\n")).unwrap();
        let call = json!({"role": "assistant", "tool_calls": [{"id": "g", "type": "function",
            "function": {"name": "glob", "arguments": "{\"pattern\":\"**/*\"}"}}]});
        let parent_call = json!({"role": "assistant", "tool_calls": [{"id": "d", "type": "function",
            "function": {"name": "delegate_task", "arguments": "{\"task\":\"find it\"}"}}]});
        let raw = [
            json!({"request": {"messages": [parent_call]}}),
            json!({"agent": "child", "request": {"messages": [{"role": "user", "content": "find it"}]}}),
            json!({"agent": "child", "request": {"messages": [call,
                {"role": "tool", "tool_call_id": "g", "content": "error: program not found: rg"}]}}),
            json!({"request": {"messages": [parent_call,
                {"role": "tool", "tool_call_id": "d", "content": "error: the subagent ended without a final answer (MaxTurns)"}]}}),
        ];
        let lines: Vec<String> = raw.iter().map(Value::to_string).collect();
        fs::write(dir.join("raw/requests.jsonl"), lines.join("\n")).unwrap();
        (dir, record)
    }

    #[test]
    fn a_failed_delegation_takes_the_kind_of_its_childs_last_failure() {
        for child_facts in [true, false] {
            let (dir, record) = delegated_run(child_facts);
            let failed = of_run(&dir, &record);
            assert_eq!(failed.len(), 2, "{child_facts}");
            let child = &failed[1];
            assert_eq!(child.agent, Agent::Child);
            assert_eq!(child.kind(), Some(Kind::Environment));
            assert_eq!(child.from_raw, !child_facts);
            assert_eq!(failed[0].kind(), Some(Kind::Environment), "{child_facts}");
            fs::remove_dir_all(dir).unwrap();
        }
    }
}
