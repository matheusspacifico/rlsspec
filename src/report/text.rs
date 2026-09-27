use std::fmt::Write;

use anstyle::Style;

use crate::runner::{Outcome, Report};
use crate::style;

const LITERAL_KEEP: usize = 4;
const LITERAL_MAX: usize = 12;

/// Renders with terminal styles; print through `anstream`, which strips them when colour is off.
pub fn render(report: &Report) -> String {
    let mut tables: Vec<&str> = Vec::new();
    for result in &report.results {
        if !tables.contains(&result.table.as_str()) {
            tables.push(&result.table);
        }
    }

    let mut out = String::new();
    for table in tables {
        let rows: Vec<(Style, [String; 5])> = report
            .results
            .iter()
            .filter(|r| r.table == table)
            .map(|r| {
                let (style, mark, detail) = match &r.outcome {
                    Outcome::Pass(detail) => (style::PASS, "✓", detail.as_str()),
                    Outcome::Fail(detail) => (style::FAIL, "✗", detail.as_str()),
                    Outcome::Inconclusive(detail) => (style::INCONCLUSIVE, "?", detail.as_str()),
                };
                let cells = [
                    mark.to_owned(),
                    r.identity.clone(),
                    r.op.as_str().to_owned(),
                    shorten_literals(&r.description),
                    detail.to_owned(),
                ];
                (style, cells)
            })
            .collect();
        let width = |i: usize| {
            rows.iter()
                .map(|(_, r)| r[i].chars().count())
                .max()
                .unwrap_or(0)
        };
        let (identity_width, op_width, description_width) = (width(1), width(2), width(3));
        let (header, muted) = (style::EMPHASIS, style::MUTED);
        let _ = writeln!(out, "{header}{table}{header:#}");
        for (s, [mark, identity, op, description, detail]) in &rows {
            let detail_style = if *s == style::PASS { Style::new() } else { *s };
            let _ = writeln!(
                out,
                "  {s}{mark}{s:#} {identity:<identity_width$}  {muted}{op:<op_width$}{muted:#}  {description:<description_width$}  {detail_style}{detail}{detail_style:#}"
            );
        }
    }

    let totals = report.totals();
    let count = |n: usize, label: &str, when_nonzero: Style| {
        let s = if n > 0 { when_nonzero } else { Style::new() };
        format!("{s}{n} {label}{s:#}")
    };
    let _ = writeln!(
        out,
        "{} · {} · {}",
        count(totals.failed, "failed", style::FAIL),
        count(totals.passed, "passed", style::PASS),
        count(totals.inconclusive, "inconclusive", style::INCONCLUSIVE)
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
    use crate::runner::{CaseResult, Op, Origin};

    fn report() -> Report {
        let case = |identity: &str, outcome| CaseResult {
            table: "notes".into(),
            identity: identity.into(),
            op: Op::Select,
            description: "deny".into(),
            outcome,
            origin: Origin::Expect,
        };
        Report {
            results: vec![
                case("alice", Outcome::Pass("0 visible".into())),
                case("guest", Outcome::Fail("leaked 1 of 2 rows: id=1".into())),
                case("bob", Outcome::Inconclusive("vacuous".into())),
            ],
        }
    }

    #[test]
    fn styles_do_not_change_the_plain_layout() {
        let styled = render(&report());
        assert!(styled.contains('\x1b'), "{styled:?}");
        assert_eq!(
            anstream::adapter::strip_str(&styled).to_string(),
            "notes
  ✓ alice  select  deny  0 visible
  ✗ guest  select  deny  leaked 1 of 2 rows: id=1
  ? bob    select  deny  vacuous
1 failed · 1 passed · 1 inconclusive
"
        );
    }

    #[test]
    fn long_literals_are_shortened() {
        assert_eq!(
            shorten_literals("tenant_id = '0191e6a2-0000-7000-8000-00000000000a' and x = 'ok'"),
            "tenant_id = '…000a' and x = 'ok'"
        );
        assert_eq!(shorten_literals("name = 'o''brien'"), "name = 'o''brien'");
    }
}
