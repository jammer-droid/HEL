//! Summarizes judged runs in the terminal and in results/<lab>/report.md (SPEC §8.1).

use std::collections::BTreeMap;
use std::error::Error;
use std::fmt::Write as _;
use std::fs;
use std::path::Path;

use record::{Metric, MetricStatus, Record, ToolEvent};

use crate::check::{Checked, Judged};
use crate::spec::Plan;

struct Row {
    condition: String,
    task: String,
    runs: usize,
    valid: usize,
    pass: usize,
    checks: BTreeMap<String, (usize, usize)>,
    tokens: String,
    /// Mean cached input tokens and mean peak context, e.g. `4800 · 4591`.
    context: String,
    calls: String,
    wall: String,
    /// Mean tool calls per run by tool name, e.g. `bash 2.0 · read_file 1.0`.
    tools: String,
}

pub fn write(root: &Path, plan: &Plan, checked: &Checked) -> Result<(), Box<dyn Error>> {
    let judged = &checked.judged;
    if judged.is_empty() {
        println!(
            "{} · no runs in {}",
            plan.experiment,
            plan.results.display()
        );
        if !checked.other_experiments.is_empty() {
            println!(
                "      other experiments there: {} (not included)",
                checked.other_experiments.join(", ")
            );
        }
        return Ok(());
    }

    let rows = rows(judged);
    let check_ids: Vec<String> = rows
        .iter()
        .flat_map(|r| r.checks.keys().cloned())
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect();
    let invalid = judged
        .iter()
        .filter(|r| r.verdict.overall == "invalid")
        .count();

    print_terminal(plan, judged.len(), invalid, &rows, &check_ids);
    let markdown = markdown(&root.join(&plan.results), plan, judged, &rows, &check_ids)?;
    let path = root.join(&plan.results).join("report.md");
    fs::write(&path, markdown)?;
    println!("\nreport    {}", plan.results.join("report.md").display());
    Ok(())
}

fn rows(judged: &[Judged]) -> Vec<Row> {
    let mut groups: BTreeMap<(String, String), Vec<&Judged>> = BTreeMap::new();
    for run in judged {
        groups
            .entry((run.condition.clone(), run.task_id.clone()))
            .or_default()
            .push(run);
    }
    groups
        .into_iter()
        .map(|((condition, task), runs)| {
            let valid: Vec<&Judged> = runs
                .iter()
                .copied()
                .filter(|r| r.verdict.overall != "invalid")
                .collect();
            let mut checks: BTreeMap<String, (usize, usize)> = BTreeMap::new();
            for run in &valid {
                for check in &run.verdict.checks {
                    let entry = checks.entry(check.id.clone()).or_default();
                    if check.result != "n/a" {
                        entry.1 += 1;
                        if check.result == "pass" {
                            entry.0 += 1;
                        }
                    }
                }
            }
            let records: Vec<_> = valid.iter().filter_map(|r| r.record.as_ref()).collect();
            let mean_of =
                |pick: fn(&record::Usage) -> &Metric| mean(records.iter().map(|r| pick(&r.usage)));
            Row {
                runs: runs.len(),
                pass: valid.iter().filter(|r| r.verdict.overall == "pass").count(),
                valid: valid.len(),
                checks,
                tokens: format!(
                    "{} / {}",
                    mean_of(|u| &u.input_tokens),
                    mean_of(|u| &u.output_tokens)
                ),
                context: format!(
                    "{} · {}",
                    mean_of(|u| &u.cached_input_tokens),
                    mean_of(|u| &u.peak_context_tokens)
                ),
                calls: mean_of(|u| &u.model_calls),
                wall: mean_of(|u| &u.wall_time_ms),
                tools: tool_means(&records),
                condition,
                task,
            }
        })
        .collect()
}

fn check_cell(row: &Row, id: &str) -> String {
    match row.checks.get(id) {
        Some((_, 0)) | None => "n/a".to_string(),
        Some((pass, applicable)) => format!("{pass}/{applicable}"),
    }
}

fn print_terminal(plan: &Plan, runs: usize, invalid: usize, rows: &[Row], check_ids: &[String]) {
    let validity = if invalid == 0 {
        "all valid".to_string()
    } else {
        format!("{invalid} invalid")
    };
    println!("\n{} · {runs} runs · {validity}\n", plan.experiment);

    let mut header = vec![
        "condition".to_string(),
        "task".to_string(),
        "pass".to_string(),
    ];
    header.extend(check_ids.iter().cloned());
    header.push("in/out tokens".to_string());
    header.push("cached · peak ctx".to_string());
    header.push("calls".to_string());
    header.push("tools".to_string());

    let mut table = vec![header];
    for row in rows {
        let mut line = vec![
            row.condition.clone(),
            row.task.clone(),
            format!("{}/{}", row.pass, row.valid),
        ];
        line.extend(check_ids.iter().map(|id| check_cell(row, id)));
        line.push(row.tokens.clone());
        line.push(row.context.clone());
        line.push(row.calls.clone());
        line.push(row.tools.clone());
        table.push(line);
    }
    let widths: Vec<usize> = (0..table[0].len())
        .map(|col| {
            table
                .iter()
                .map(|line| line[col].chars().count())
                .max()
                .unwrap_or(0)
        })
        .collect();
    for line in &table {
        let cells: Vec<String> = line
            .iter()
            .zip(&widths)
            .map(|(cell, width)| format!("{cell:<width$}"))
            .collect();
        println!("{}", cells.join("  ").trim_end());
    }
}

fn markdown(
    results: &Path,
    plan: &Plan,
    judged: &[Judged],
    rows: &[Row],
    check_ids: &[String],
) -> Result<String, Box<dyn Error>> {
    let mut out = String::new();
    writeln!(out, "# Report — {}\n", plan.experiment)?;
    writeln!(
        out,
        "lab: `{}` · model: `{}` · repetitions: {} · runs: {}\n",
        plan.lab,
        plan.model.id,
        plan.repetitions,
        judged.len()
    )?;

    if !plan.rubric.is_empty() {
        writeln!(out, "## Rubric\n")?;
        for (id, text) in &plan.rubric {
            writeln!(out, "- `{id}`: {text}")?;
        }
        writeln!(out)?;
    }

    writeln!(out, "## Summary\n")?;
    write!(out, "| condition | task | runs | valid | pass |")?;
    for id in check_ids {
        write!(out, " {id} |")?;
    }
    writeln!(
        out,
        " in/out tokens | cached in · peak context | model calls | wall time (ms) | tool calls |"
    )?;
    writeln!(out, "|{}", " --- |".repeat(10 + check_ids.len()))?;
    for row in rows {
        write!(
            out,
            "| {} | {} | {} | {} | {} |",
            row.condition, row.task, row.runs, row.valid, row.pass
        )?;
        for id in check_ids {
            write!(out, " {} |", check_cell(row, id))?;
        }
        writeln!(
            out,
            " {} | {} | {} | {} | {} |",
            row.tokens, row.context, row.calls, row.wall, row.tools
        )?;
    }
    writeln!(
        out,
        "\nNumbers are means over valid runs. `*` marks derived values (computed, not measured).\n"
    )?;

    let charts = cache_charts(results, judged);
    if !charts.is_empty() {
        writeln!(out, "## Cache hit by request\n")?;
        writeln!(
            out,
            "Mean over valid runs that reached each request. Auxiliary requests (e.g. compaction) are excluded from the series and listed below each chart.\n"
        )?;
        out.push_str(&charts);
    }

    writeln!(out, "## Runs\n")?;
    for run in judged {
        let termination = run
            .record
            .as_ref()
            .map(|r| format!("{:?}", r.outcome.termination))
            .unwrap_or_default();
        let changed = if run.overridden {
            " · input overridden"
        } else {
            ""
        };
        writeln!(
            out,
            "- **{}** — {} · {termination}{changed}",
            run.run_id, run.verdict.overall
        )?;
        if let Some(record) = &run.record {
            if let Some(line) = record
                .artifacts
                .raw_transcript
                .as_ref()
                .and_then(|raw| fs::read_to_string(results.join(&run.run_id).join(raw)).ok())
                .and_then(|raw| context_per_call(&raw))
            {
                writeln!(out, "  - {line}")?;
            }
            for event in &record.events {
                writeln!(out, "  - tool: {}", describe_event(event))?;
            }
        }
        for check in &run.verdict.checks {
            if check.result == "fail" || run.verdict.overall == "invalid" {
                writeln!(out, "  - {} ({})", check.id, check.result)?;
                writeln!(out, "    ```text")?;
                for line in check.detail.lines() {
                    writeln!(out, "    {line}")?;
                }
                writeln!(out, "    ```")?;
            }
        }
    }
    Ok(out)
}

/// Model requests read from a raw log whose lines carry an OpenAI-style `response.usage`.
/// Entries with a `purpose` (e.g. `"compaction"`, H6) are the harness's own auxiliary requests,
/// and entries with `"agent": "child"` are a subagent's requests (H11); both are kept apart from
/// the parent conversation's requests.
struct Requests {
    /// Conversation requests in order: (prompt tokens, cache-hit tokens if reported).
    main: Vec<(u64, Option<u64>)>,
    /// Auxiliary requests: (number of conversation requests before it, purpose, prompt, hit).
    aux: Vec<(usize, String, u64, Option<u64>)>,
}

fn parse_requests(raw: &str) -> Option<Requests> {
    let mut requests = Requests {
        main: Vec::new(),
        aux: Vec::new(),
    };
    for line in raw.lines() {
        let entry: serde_json::Value = serde_json::from_str(line).ok()?;
        let Some(usage) = entry.pointer("/response/usage") else {
            continue;
        };
        let prompt = usage.get("prompt_tokens")?.as_u64()?;
        let hit = usage
            .get("prompt_cache_hit_tokens")
            .and_then(serde_json::Value::as_u64);
        let purpose = entry
            .get("purpose")
            .and_then(serde_json::Value::as_str)
            .or_else(|| (entry.get("agent")? == "child").then_some("child"));
        match purpose {
            Some(purpose) => {
                requests
                    .aux
                    .push((requests.main.len(), purpose.to_string(), prompt, hit))
            }
            None => requests.main.push((prompt, hit)),
        }
    }
    (!requests.main.is_empty()).then_some(requests)
}

/// One line per run, e.g. `context per call: 462 → 4409 → 4591 · cache hit: 0 → 640 → 4352`,
/// followed by any auxiliary requests. `None` for raw logs in other formats.
fn context_per_call(raw: &str) -> Option<String> {
    let requests = parse_requests(raw)?;
    let contexts: Vec<String> = requests.main.iter().map(|(p, _)| p.to_string()).collect();
    let hits: Vec<String> = requests
        .main
        .iter()
        .map(|(_, h)| h.map_or("?".to_string(), |h| h.to_string()))
        .collect();
    let mut line = format!(
        "context per call: {} · cache hit: {}",
        contexts.join(" → "),
        hits.join(" → ")
    );
    for (after, purpose, prompt, hit) in &requests.aux {
        let hit = hit.map_or("?".to_string(), |h| h.to_string());
        line.push_str(&format!(
            " · {purpose} after call {after}: {prompt} (cache hit {hit})"
        ));
    }
    Some(line)
}

/// Mermaid charts of the mean cache-hit share and context size by request position, one pair per
/// task and condition, averaged over the valid runs that reached that position.
/// Upper end of the token axis: the largest value rounded up to a thousand, so bars start at 0.
fn context_axis_top(values: &[f64]) -> u64 {
    let max = values.iter().copied().fold(0.0, f64::max);
    ((max / 1000.0).ceil() as u64).max(1) * 1000
}

/// Draws chart marks in a strong color; Mermaid's default xychart palette is very light.
const CHART_INIT: &str =
    r##"%%{init: {"themeVariables": {"xyChart": {"plotColorPalette": "#e8590c"}}}}%%"##;

fn cache_charts(results: &Path, judged: &[Judged]) -> String {
    let mut groups: BTreeMap<(String, String), Vec<Requests>> = BTreeMap::new();
    for run in judged.iter().filter(|r| r.verdict.overall != "invalid") {
        let Some(raw) = run
            .record
            .as_ref()
            .and_then(|r| r.artifacts.raw_transcript.as_ref())
        else {
            continue;
        };
        let Some(requests) = fs::read_to_string(results.join(&run.run_id).join(raw))
            .ok()
            .and_then(|text| parse_requests(&text))
        else {
            continue;
        };
        groups
            .entry((run.task_id.clone(), run.condition.clone()))
            .or_default()
            .push(requests);
    }
    let mut out = String::new();
    for ((task, condition), runs) in &groups {
        let longest = runs.iter().map(|r| r.main.len()).max().unwrap_or(0);
        let mut hit_share = Vec::new();
        let mut context = Vec::new();
        for i in 0..longest {
            let at: Vec<(u64, Option<u64>)> =
                runs.iter().filter_map(|r| r.main.get(i).copied()).collect();
            let shares: Vec<f64> = at
                .iter()
                .filter_map(|(p, h)| {
                    h.map(|h| {
                        if *p == 0 {
                            0.0
                        } else {
                            h as f64 * 100.0 / *p as f64
                        }
                    })
                })
                .collect();
            hit_share.push(if shares.is_empty() {
                0.0
            } else {
                shares.iter().sum::<f64>() / shares.len() as f64
            });
            context.push(at.iter().map(|(p, _)| *p as f64).sum::<f64>() / at.len() as f64);
        }
        let axis: Vec<String> = (1..=longest).map(|i| i.to_string()).collect();
        let join = |values: &[f64]| {
            values
                .iter()
                .map(|v| format!("{v:.0}"))
                .collect::<Vec<_>>()
                .join(", ")
        };
        let _ = writeln!(out, "### {task} · {condition} ({} runs)\n", runs.len());
        let _ = writeln!(
            out,
            "```mermaid\n{CHART_INIT}\nxychart-beta\n    title \"cache hit % by request\"\n    x-axis \"request\" [{}]\n    y-axis \"cache hit %\" 0 --> 100\n    line [{}]\n```\n",
            axis.join(", "),
            join(&hit_share)
        );
        let _ = writeln!(
            out,
            "```mermaid\n{CHART_INIT}\nxychart-beta\n    title \"context tokens by request\"\n    x-axis \"request\" [{}]\n    y-axis \"tokens\" 0 --> {}\n    bar [{}]\n```\n",
            axis.join(", "),
            context_axis_top(&context),
            join(&context)
        );
        let aux: Vec<String> = runs
            .iter()
            .enumerate()
            .flat_map(|(n, r)| {
                r.aux.iter().map(move |(after, purpose, _, _)| {
                    format!("run {} {purpose} after request {after}", n + 1)
                })
            })
            .collect();
        if !aux.is_empty() {
            let _ = writeln!(out, "{}\n", aux.join(" · "));
        }
    }
    out
}

fn mean<'a>(metrics: impl Iterator<Item = &'a Metric>) -> String {
    let mut values = Vec::new();
    let mut derived = false;
    for metric in metrics {
        if let Some(value) = metric.value {
            values.push(value);
            derived |= metric.status == MetricStatus::Derived;
        }
    }
    if values.is_empty() {
        return "—".to_string();
    }
    let mean = values.iter().sum::<f64>() / values.len() as f64;
    format!("{mean:.0}{}", if derived { "*" } else { "" })
}

fn tool_means(records: &[&Record]) -> String {
    if records.is_empty() {
        return "—".to_string();
    }
    let mut counts: BTreeMap<&str, usize> = BTreeMap::new();
    for record in records {
        for event in &record.events {
            *counts.entry(event.name.as_str()).or_default() += 1;
        }
    }
    if counts.is_empty() {
        return "none".to_string();
    }
    counts
        .iter()
        .map(|(name, count)| format!("{name} {:.1}", *count as f64 / records.len() as f64))
        .collect::<Vec<_>>()
        .join(" · ")
}

/// One line per tool call: the name and its main argument (`command` or `path`).
fn describe_event(event: &ToolEvent) -> String {
    const MAX_CHARS: usize = 80;
    let arg = ["command", "path", "file_path"]
        .iter()
        .find_map(|key| event.args.get(key).and_then(|v| v.as_str()))
        .map(str::to_string)
        .unwrap_or_else(|| event.args.to_string());
    let arg: String = arg.replace('\n', " ⏎ ");
    let shown = if arg.chars().count() > MAX_CHARS {
        format!("{}…", arg.chars().take(MAX_CHARS).collect::<String>())
    } else {
        arg
    };
    let failed = if event.ok == Some(false) {
        " (error)"
    } else {
        ""
    };
    format!("`{}` `{}`{failed}", event.name, shown.replace('`', "'"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use record::ToolCategory;
    use serde_json::json;

    fn event(name: &str, args: serde_json::Value, ok: Option<bool>) -> ToolEvent {
        ToolEvent {
            seq: 1,
            category: ToolCategory::Other,
            name: name.to_string(),
            args,
            ok,
        }
    }

    #[test]
    fn describes_command_and_path_arguments() {
        let bash = event(
            "bash",
            json!({ "command": "find . -name a.txt" }),
            Some(true),
        );
        assert_eq!(describe_event(&bash), "`bash` `find . -name a.txt`");
        let read = event("read_file", json!({ "path": "a.txt" }), Some(false));
        assert_eq!(describe_event(&read), "`read_file` `a.txt` (error)");
    }

    #[test]
    fn shortens_long_multiline_commands() {
        let long = "x".repeat(100);
        let bash = event("bash", json!({ "command": format!("cat a\n{long}") }), None);
        let shown = describe_event(&bash);
        assert!(shown.contains("cat a ⏎ x"));
        assert!(shown.ends_with("…`"));
    }

    #[test]
    fn lists_context_and_cache_hits_per_call() {
        let raw = [
            r#"{"request":{},"response":{"usage":{"prompt_tokens":462,"prompt_cache_hit_tokens":0}}}"#,
            r#"{"request":{},"response":{"usage":{"prompt_tokens":4409,"prompt_cache_hit_tokens":640}}}"#,
            r#"{"error":"timeout"}"#,
            r#"{"request":{},"response":{"usage":{"prompt_tokens":4591}}}"#,
        ]
        .join("\n");
        assert_eq!(
            context_per_call(&raw).as_deref(),
            Some("context per call: 462 → 4409 → 4591 · cache hit: 0 → 640 → ?")
        );
    }

    #[test]
    fn skips_raw_logs_without_usage() {
        assert_eq!(context_per_call(r#"{"type":"assistant"}"#), None);
        assert_eq!(context_per_call("not json"), None);
    }

    #[test]
    fn keeps_compaction_requests_apart_from_the_conversation() {
        let raw = [
            r#"{"turn":1,"response":{"usage":{"prompt_tokens":500,"prompt_cache_hit_tokens":0}}}"#,
            r#"{"turn":1,"response":{"usage":{"prompt_tokens":2000,"prompt_cache_hit_tokens":384}}}"#,
            r#"{"turn":2,"purpose":"compaction","response":{"usage":{"prompt_tokens":2100,"prompt_cache_hit_tokens":1920}}}"#,
            r#"{"turn":2,"response":{"usage":{"prompt_tokens":900,"prompt_cache_hit_tokens":384}}}"#,
        ]
        .join("\n");
        let requests = parse_requests(&raw).unwrap();
        assert_eq!(
            requests.main,
            [(500, Some(0)), (2000, Some(384)), (900, Some(384))]
        );
        assert_eq!(
            requests.aux,
            [(2, "compaction".to_string(), 2100, Some(1920))]
        );
        assert_eq!(
            context_per_call(&raw).as_deref(),
            Some(
                "context per call: 500 → 2000 → 900 · cache hit: 0 → 384 → 384 \
                 · compaction after call 2: 2100 (cache hit 1920)"
            )
        );
    }

    #[test]
    fn keeps_child_requests_apart_from_the_parent() {
        let raw = [
            r#"{"response":{"usage":{"prompt_tokens":1800,"prompt_cache_hit_tokens":0}}}"#,
            r#"{"agent":"child","response":{"usage":{"prompt_tokens":1900,"prompt_cache_hit_tokens":1792}}}"#,
            r#"{"response":{"usage":{"prompt_tokens":1950,"prompt_cache_hit_tokens":1920}}}"#,
        ]
        .join("\n");
        let requests = parse_requests(&raw).unwrap();
        assert_eq!(requests.main, [(1800, Some(0)), (1950, Some(1920))]);
        assert_eq!(requests.aux, [(1, "child".to_string(), 1900, Some(1792))]);
    }

    #[test]
    fn token_axis_starts_at_zero_and_rounds_up() {
        assert_eq!(context_axis_top(&[445.0, 5351.0]), 6000);
        assert_eq!(context_axis_top(&[3000.0]), 3000);
        assert_eq!(context_axis_top(&[]), 1000);
    }
}
