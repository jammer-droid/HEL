//! Summarizes judged runs in the terminal and in results/<lab>/report.md (SPEC §8.1).

use std::collections::BTreeMap;
use std::error::Error;
use std::fmt::Write as _;
use std::fs;
use std::path::Path;

use record::{Metric, MetricStatus};

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
    calls: String,
    wall: String,
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
    let markdown = markdown(plan, judged, &rows, &check_ids)?;
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
                calls: mean_of(|u| &u.model_calls),
                wall: mean_of(|u| &u.wall_time_ms),
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
    header.push("calls".to_string());

    let mut table = vec![header];
    for row in rows {
        let mut line = vec![
            row.condition.clone(),
            row.task.clone(),
            format!("{}/{}", row.pass, row.valid),
        ];
        line.extend(check_ids.iter().map(|id| check_cell(row, id)));
        line.push(row.tokens.clone());
        line.push(row.calls.clone());
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
    writeln!(out, " in/out tokens | model calls | wall time (ms) |")?;
    writeln!(out, "|{}", " --- |".repeat(8 + check_ids.len()))?;
    for row in rows {
        write!(
            out,
            "| {} | {} | {} | {} | {} |",
            row.condition, row.task, row.runs, row.valid, row.pass
        )?;
        for id in check_ids {
            write!(out, " {} |", check_cell(row, id))?;
        }
        writeln!(out, " {} | {} | {} |", row.tokens, row.calls, row.wall)?;
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
