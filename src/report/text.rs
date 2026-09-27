use std::fmt::Write;

use anstyle::Style;

use crate::config::Unspecified;
use crate::runner::coverage::Coverage;
use crate::runner::{CaseResult, Op, Origin, Outcome, Report};
use crate::style;

const LITERAL_KEEP: usize = 4;
const LITERAL_MAX: usize = 12;

struct Line {
    style: Style,
    mark: &'static str,
    identity: String,
    op: String,
    description: String,
    detail: String,
}

fn marked(outcome: &Outcome) -> (Style, &'static str, &str) {
    match outcome {
        Outcome::Pass(detail) => (style::PASS, "✓", detail),
        Outcome::Fail(detail) => (style::FAIL, "✗", detail),
        Outcome::Inconclusive(detail) => (style::INCONCLUSIVE, "?", detail),
    }
}

fn line(result: &CaseResult) -> Line {
    let (style, mark, detail) = marked(&result.outcome);
    Line {
        style,
        mark,
        identity: result.identity.clone(),
        op: result.op.as_str().to_owned(),
        description: result.description.clone(),
        detail: detail.to_owned(),
    }
}

/// One line for all the cases an identity got from `defaults` on a table.
fn aggregate(results: &[&CaseResult]) -> Line {
    let worst = results.iter().map(|r| &r.outcome).max_by_key(|o| match o {
        Outcome::Pass(_) => 0,
        Outcome::Inconclusive(_) => 1,
        Outcome::Fail(_) => 2,
    });
    let (style, mark, _) = worst.map_or((style::PASS, "✓", ""), marked);

    let mut descriptions: Vec<&str> = results.iter().map(|r| r.description.as_str()).collect();
    descriptions.dedup();
    let description = match descriptions.as_slice() {
        [one] if results.len() > 1 => (*one).to_owned(),
        _ => results
            .iter()
            .map(|r| format!("{} {}", r.op.as_str(), r.description))
            .collect::<Vec<_>>()
            .join(", "),
    };

    let mut ops: Vec<Op> = results.iter().map(|r| r.op).collect();
    ops.dedup();
    let unit = if ops.len() == results.len() {
        "ops"
    } else {
        "cases"
    };
    let passed = results
        .iter()
        .filter(|r| matches!(r.outcome, Outcome::Pass(_)))
        .count();
    let mut detail = format!("{passed}/{} {unit}", results.len());
    for result in results {
        if let Outcome::Fail(text) | Outcome::Inconclusive(text) = &result.outcome {
            let _ = write!(detail, " · {}: {text}", result.op.as_str());
        }
    }
    Line {
        style,
        mark,
        identity: results
            .first()
            .map_or(String::new(), |r| r.identity.clone()),
        op: "*".to_owned(),
        description,
        detail,
    }
}

fn lines(results: &[&CaseResult]) -> Vec<Line> {
    enum Slot<'a> {
        Case(&'a CaseResult),
        Defaults(Vec<&'a CaseResult>),
    }
    let mut slots: Vec<Slot> = Vec::new();
    for &result in results {
        if result.origin == Origin::Expect {
            slots.push(Slot::Case(result));
            continue;
        }
        let group = slots.iter_mut().find_map(|slot| match slot {
            Slot::Defaults(group) if group[0].identity == result.identity => Some(group),
            _ => None,
        });
        match group {
            Some(group) => group.push(result),
            None => slots.push(Slot::Defaults(vec![result])),
        }
    }
    slots
        .iter()
        .map(|slot| match slot {
            Slot::Case(result) => line(result),
            Slot::Defaults(group) => aggregate(group),
        })
        .collect()
}

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
        let results: Vec<&CaseResult> =
            report.results.iter().filter(|r| r.table == table).collect();
        let mut rows = lines(&results);
        for row in &mut rows {
            row.description = shorten_literals(&row.description);
        }
        let width = |cell: fn(&Line) -> &str| {
            rows.iter()
                .map(|r| cell(r).chars().count())
                .max()
                .unwrap_or(0)
        };
        let identity_width = width(|r| &r.identity);
        let op_width = width(|r| &r.op);
        let description_width = width(|r| &r.description);
        let (header, muted) = (style::EMPHASIS, style::MUTED);
        let _ = writeln!(out, "{header}{table}{header:#}");
        for Line {
            style: s,
            mark,
            identity,
            op,
            description,
            detail,
        } in &rows
        {
            let detail_style = if *s == style::PASS { Style::new() } else { *s };
            let _ = writeln!(
                out,
                "  {s}{mark}{s:#} {identity:<identity_width$}  {muted}{op:<op_width$}{muted:#}  {description:<description_width$}  {detail_style}{detail}{detail_style:#}"
            );
        }
    }

    out.push_str(&coverage_summary(&report.coverage));
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

/// `coverage 86/96 cells (89.6%) · 10 unspecified (warn)`, then the gaps unless `ignore`.
fn coverage_summary(coverage: &Coverage) -> String {
    let mut out = coverage_line(coverage);
    if coverage.policy == Unspecified::Ignore {
        return out;
    }
    let mark = gap_style(coverage.policy);
    let header = style::EMPHASIS;
    for (table, gaps) in coverage.gaps() {
        let _ = writeln!(out, "{header}{table}{header:#}");
        let width = gaps.iter().map(|g| g.identity.chars().count()).max();
        let width = width.unwrap_or(0);
        for gap in gaps {
            let ops: Vec<&str> = gap.ops.iter().map(|op| op.as_str()).collect();
            let _ = writeln!(
                out,
                "  {mark}○{mark:#} {:<width$}  {}",
                gap.identity,
                ops.join(", ")
            );
        }
    }
    out
}

fn gap_style(policy: Unspecified) -> Style {
    match policy {
        Unspecified::Fail => style::FAIL,
        Unspecified::Warn | Unspecified::Ignore => style::INCONCLUSIVE,
    }
}

/// The coverage matrix: one row per table, one column per identity, `SIUD` with `·` for gaps.
pub fn render_cover(coverage: &Coverage) -> String {
    const LETTERS: [char; 4] = ['S', 'I', 'U', 'D'];
    let table_width = coverage
        .tables
        .iter()
        .map(|t| t.table.chars().count())
        .max();
    let table_width = table_width.unwrap_or(0);
    let widths: Vec<usize> = coverage
        .identities
        .iter()
        .map(|i| i.chars().count().max(LETTERS.len()))
        .collect();
    let gap = match coverage.policy {
        Unspecified::Ignore => style::MUTED,
        policy => gap_style(policy),
    };
    let (header, muted) = (style::EMPHASIS, style::MUTED);

    let pad = |i: usize, used: usize| {
        let last = i + 1 == widths.len();
        if last { 0 } else { widths[i] - used }
    };

    let mut out = " ".repeat(table_width);
    for (i, identity) in coverage.identities.iter().enumerate() {
        let used = identity.chars().count();
        let _ = write!(
            out,
            "  {muted}{identity}{muted:#}{}",
            " ".repeat(pad(i, used))
        );
    }
    out.push('\n');
    for table in &coverage.tables {
        let _ = write!(out, "{header}{:<table_width$}{header:#}", table.table);
        for (i, cells) in table.cells.iter().enumerate() {
            out.push_str("  ");
            for (letter, given) in LETTERS.iter().zip(cells) {
                if *given {
                    out.push(*letter);
                } else {
                    let _ = write!(out, "{gap}·{gap:#}");
                }
            }
            out.push_str(&" ".repeat(pad(i, LETTERS.len())));
        }
        out.push('\n');
    }
    out.push_str(&coverage_line(coverage));
    out
}

pub fn coverage_line(coverage: &Coverage) -> String {
    let (specified, total) = (coverage.specified(), coverage.total());
    let unspecified = coverage.unspecified();
    // Rounded down, so a single gap never shows as 100.0%.
    let percent = match (specified * 1000).checked_div(total) {
        Some(tenths) => format!(" ({}.{}%)", tenths / 10, tenths % 10),
        None => String::new(),
    };
    let s = if unspecified > 0 && coverage.policy != Unspecified::Ignore {
        gap_style(coverage.policy)
    } else {
        Style::new()
    };
    format!(
        "coverage {specified}/{total} cells{percent} · {s}{unspecified} unspecified ({}){s:#}\n",
        coverage.policy.as_str()
    )
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
    use crate::runner::coverage::TableCoverage;

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
            ..Report::default()
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
coverage 0/0 cells · 0 unspecified (warn)
1 failed · 1 passed · 1 inconclusive
"
        );
    }

    fn coverage(policy: Unspecified, cells: [[bool; 4]; 2]) -> Coverage {
        Coverage {
            policy,
            identities: vec!["alice".into(), "bob".into()],
            tables: vec![TableCoverage {
                table: "notes".into(),
                cells: cells.to_vec(),
            }],
        }
    }

    fn plain(text: &str) -> String {
        anstream::adapter::strip_str(text).to_string()
    }

    #[test]
    fn coverage_lists_gaps_unless_ignored() {
        let cells = [[true; 4], [true, false, false, true]];
        assert_eq!(
            plain(&coverage_summary(&coverage(Unspecified::Warn, cells))),
            "coverage 6/8 cells (75.0%) · 2 unspecified (warn)\nnotes\n  ○ bob  insert, update\n"
        );
        assert_eq!(
            plain(&coverage_summary(&coverage(Unspecified::Ignore, cells))),
            "coverage 6/8 cells (75.0%) · 2 unspecified (ignore)\n"
        );
    }

    #[test]
    fn cover_matrix_marks_gaps() {
        let mut coverage = coverage(Unspecified::Fail, [[true; 4], [true, false, false, true]]);
        coverage.identities[0] = "anonymous".into();
        coverage.tables.push(TableCoverage {
            table: "organizations".into(),
            cells: vec![[false; 4], [true; 4]],
        });
        let styled = render_cover(&coverage);
        assert!(styled.contains('\x1b'), "{styled:?}");
        assert_eq!(
            plain(&styled),
            "               anonymous  bob
notes          SIUD       S··D
organizations  ····       SIUD
coverage 10/16 cells (62.5%) · 6 unspecified (fail)
"
        );
    }

    #[test]
    fn coverage_percentage_rounds_down() {
        let mut coverage = coverage(Unspecified::Fail, [[true; 4]; 2]);
        coverage.tables[0].cells = vec![[true; 4]; 250];
        coverage.identities = vec!["x".into(); 250];
        coverage.tables[0].cells[0][0] = false;
        assert_eq!(
            plain(&coverage_line(&coverage)),
            "coverage 999/1000 cells (99.9%) · 1 unspecified (fail)\n"
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
