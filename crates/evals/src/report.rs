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

/// Per-call context from a raw log whose lines carry an OpenAI-style `response.usage`, e.g.
/// `context per call: 462 → 4409 → 4591 · cache hit: 0 → 640 → 4352`. `None` for other formats.
fn context_per_call(raw: &str) -> Option<String> {
    let mut contexts = Vec::new();
    let mut hits = Vec::new();
    for line in raw.lines() {
        let entry: serde_json::Value = serde_json::from_str(line).ok()?;
        let Some(usage) = entry.pointer("/response/usage") else {
            continue;
        };
        contexts.push(usage.get("prompt_tokens")?.as_u64()?.to_string());
        hits.push(
            usage
                .get("prompt_cache_hit_tokens")
                .and_then(serde_json::Value::as_u64)
                .map_or("?".to_string(), |hit| hit.to_string()),
        );
    }
    if contexts.is_empty() {
        return None;
    }
    Some(format!(
        "context per call: {} · cache hit: {}",
        contexts.join(" → "),
        hits.join(" → ")
    ))
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
}
