use std::fmt::Write;

use crate::runner::{Outcome, Report};

const LITERAL_KEEP: usize = 4;
const LITERAL_MAX: usize = 12;

pub fn render(report: &Report) -> String {
    let mut tables: Vec<&str> = Vec::new();
    for result in &report.results {
        if !tables.contains(&result.table.as_str()) {
            tables.push(&result.table);
        }
    }

    let mut out = String::new();
    for table in tables {
        let rows: Vec<[String; 5]> = report
            .results
            .iter()
            .filter(|r| r.table == table)
            .map(|r| {
                let (mark, detail) = match &r.outcome {
                    Outcome::Pass(detail) => ("✓", detail.as_str()),
                    Outcome::Fail(detail) => ("✗", detail.as_str()),
                    Outcome::Inconclusive(detail) => ("?", detail.as_str()),
                    Outcome::Unsupported => ("?", "not supported yet"),
                };
                [
                    mark.to_owned(),
                    r.identity.clone(),
                    r.op.as_str().to_owned(),
                    shorten_literals(&r.description),
                    detail.to_owned(),
                ]
            })
            .collect();
        let width = |i: usize| rows.iter().map(|r| r[i].chars().count()).max().unwrap_or(0);
        let (identity_width, op_width, description_width) = (width(1), width(2), width(3));
        let _ = writeln!(out, "{table}");
        for [mark, identity, op, description, detail] in &rows {
            let _ = writeln!(
                out,
                "  {mark} {identity:<identity_width$}  {op:<op_width$}  {description:<description_width$}  {detail}"
            );
        }
    }

    let totals = report.totals();
    let _ = writeln!(
        out,
        "{} failed · {} passed · {} inconclusive",
        totals.failed, totals.passed, totals.inconclusive
    );
    out
}

/// Shortens long quoted literals (typically UUIDs) to their last characters: `'…000a'`.
fn shorten_literals(sql: &str) -> String {
    let mut out = String::with_capacity(sql.len());
    let mut chars = sql.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\'' {
            out.push(c);
            continue;
        }
        let mut literal = String::new();
        while let Some(c) = chars.next() {
            if c == '\'' {
                if chars.peek() == Some(&'\'') {
                    chars.next();
                    literal.push_str("''");
                    continue;
                }
                break;
            }
            literal.push(c);
        }
        let count = literal.chars().count();
        if count > LITERAL_MAX {
            let tail: String = literal.chars().skip(count - LITERAL_KEEP).collect();
            let _ = write!(out, "'…{tail}'");
        } else {
            let _ = write!(out, "'{literal}'");
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn long_literals_are_shortened() {
        assert_eq!(
            shorten_literals("tenant_id = '0191e6a2-0000-7000-8000-00000000000a' and x = 'ok'"),
            "tenant_id = '…000a' and x = 'ok'"
        );
        assert_eq!(shorten_literals("name = 'o''brien'"), "name = 'o''brien'");
    }
}
