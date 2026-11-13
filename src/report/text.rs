use std::fmt::Write;

use anstyle::Style;

use crate::config::Unspecified;
use crate::lint::{LintReport, Severity};
use crate::runner::coverage::Coverage;
use crate::runner::{CaseResult, Op, Origin, Outcome, Report};
use crate::style;

const LITERAL_KEEP: usize = 4;
const LITERAL_MAX: usize = 12;
/// The description column is cut here. JSON and JUnit keep the whole text.
const DESCRIPTION_MAX: usize = 64;
/// A row longer than this puts its detail on the following lines. A fixed number, not the
/// terminal's width: the output is the same in a pipe, in CI and in snapshots.
const LINE_MAX: usize = 120;

struct Line {
    style: Style,
    /// Usually one; a folded line with both a failure and an inconclusive op shows `✗?`.
    marks: Vec<(Style, &'static str)>,
    identity: String,
    op: String,
    description: String,
    detail: String,
    /// For a folded line: what each failing or inconclusive operation said, and what was skipped.
    notes: Vec<String>,
}

/// The detail as printed: a failure's samples name rows by key, and UUID keys are shortened like
/// the literals of a description. Error messages of inconclusive cases are left whole.
fn printed(outcome: &Outcome) -> String {
    match outcome {
        Outcome::Fail(detail) => shorten_uuids(detail),
        Outcome::Pass(detail) | Outcome::Inconclusive(detail) => detail.clone(),
        Outcome::Skipped(_) => String::new(),
    }
}

fn marked(outcome: &Outcome) -> (Style, &'static str, &str) {
    match outcome {
        Outcome::Pass(detail) => (style::PASS, "✓", detail),
        Outcome::Fail(detail) => (style::FAIL, "✗", detail),
        Outcome::Inconclusive(detail) => (style::INCONCLUSIVE, "?", detail),
        Outcome::Skipped(_) => (style::MUTED, "○", ""),
    }
}

fn skipped(outcome: &Outcome) -> Option<String> {
    match outcome {
        Outcome::Skipped(reason) => Some(format!("skipped ({})", reason.text())),
        _ => None,
    }
}

fn line(result: &CaseResult) -> Line {
    let (style, mark, _) = marked(&result.outcome);
    Line {
        style,
        marks: vec![(style, mark)],
        identity: result.identity.clone(),
        op: result.op.as_str().to_owned(),
        description: result.description.clone(),
        detail: skipped(&result.outcome).unwrap_or_else(|| printed(&result.outcome)),
        notes: Vec::new(),
    }
}

/// One line for all the cases an identity got from `defaults` on a table.
fn aggregate(all: &[&CaseResult]) -> Line {
    let identity = all.first().map_or(String::new(), |r| r.identity.clone());
    let results: Vec<&CaseResult> = all
        .iter()
        .copied()
        .filter(|r| !matches!(r.outcome, Outcome::Skipped(_)))
        .collect();
    let skipped_note = all.iter().find_map(|r| skipped(&r.outcome));
    let skipped_count = all.len() - results.len();
    if let (true, Some(note)) = (results.is_empty(), &skipped_note) {
        return Line {
            style: style::MUTED,
            marks: vec![(style::MUTED, "○")],
            identity,
            op: "*".to_owned(),
            description: note.clone(),
            detail: String::new(),
            notes: Vec::new(),
        };
    }
    let failed = results
        .iter()
        .find(|r| matches!(r.outcome, Outcome::Fail(_)));
    let inconclusive = results
        .iter()
        .find(|r| matches!(r.outcome, Outcome::Inconclusive(_)));
    let marks: Vec<(Style, &str)> = [failed, inconclusive]
        .into_iter()
        .flatten()
        .map(|r| {
            let (style, mark, _) = marked(&r.outcome);
            (style, mark)
        })
        .collect();
    let marks = if marks.is_empty() {
        vec![(style::PASS, "✓")]
    } else {
        marks
    };
    let style = marks[0].0;

    // Over every case of the line, skipped ones included: it says what the default asks for.
    let mut descriptions: Vec<&str> = all.iter().map(|r| r.description.as_str()).collect();
    descriptions.dedup();
    let description = match descriptions.as_slice() {
        [one] if all.len() > 1 => (*one).to_owned(),
        _ => all
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
    let detail = format!("{passed}/{} {unit}", results.len());
    let mut notes: Vec<String> = results
        .iter()
        .filter(|r| matches!(r.outcome, Outcome::Fail(_) | Outcome::Inconclusive(_)))
        .map(|r| format!("{}: {}", r.op.as_str(), printed(&r.outcome)))
        .collect();
    if let Some(note) = skipped_note {
        notes.push(format!("{skipped_count} {note}"));
    }
    Line {
        style,
        marks,
        identity,
        op: "*".to_owned(),
        description,
        detail,
        notes,
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
            row.description = cap_description(&shorten_literals(&row.description));
        }
        let width = |cell: fn(&Line) -> &str| {
            rows.iter()
                .map(|r| cell(r).chars().count())
                .max()
                .unwrap_or(0)
        };
        let mark_width = rows.iter().map(|r| r.marks.len()).max().unwrap_or(1);
        let identity_width = width(|r| &r.identity);
        let op_width = width(|r| &r.op);
        // A line without a detail pads nothing, so it doesn't widen the column either.
        let description_width = rows
            .iter()
            .filter(|r| !r.detail.is_empty())
            .map(|r| r.description.chars().count())
            .max()
            .unwrap_or(0);
        // Mark, identity and op, with the spaces around them.
        let prefix_width = 2 + mark_width + 1 + identity_width + 2 + op_width + 2;
        // Continuation lines start two columns into the identity column.
        let indent = " ".repeat(2 + mark_width + 1 + 2);
        let (header, muted) = (style::EMPHASIS, style::MUTED);
        let _ = writeln!(out, "{header}{table}{header:#}");
        for Line {
            style: s,
            marks,
            identity,
            op,
            description,
            detail,
            notes,
        } in &rows
        {
            let mut mark = String::new();
            for (style, m) in marks {
                let _ = write!(mark, "{style}{m}{style:#}");
            }
            mark.push_str(&" ".repeat(mark_width - marks.len()));
            let detail_style = if *s == style::PASS { Style::new() } else { *s };
            let _ = write!(
                out,
                "  {mark} {identity:<identity_width$}  {muted}{op:<op_width$}{muted:#}  "
            );
            if detail.is_empty() {
                let _ = writeln!(out, "{s}{description}{s:#}");
                continue;
            }
            let inline = std::iter::once(detail)
                .chain(notes)
                .map(String::as_str)
                .collect::<Vec<_>>()
                .join(" · ");
            let fits = prefix_width + description_width + 2 + inline.chars().count() <= LINE_MAX;
            let _ = if fits {
                writeln!(
                    out,
                    "{description:<description_width$}  {detail_style}{inline}{detail_style:#}"
                )
            } else if notes.is_empty() {
                // One case: its description alone, then what happened.
                writeln!(
                    out,
                    "{description}\n{indent}{detail_style}{detail}{detail_style:#}"
                )
            } else {
                // A folded default: the count stays, then one operation per line.
                let _ = writeln!(
                    out,
                    "{description:<description_width$}  {detail_style}{detail}{detail_style:#}"
                );
                for note in notes {
                    let _ = writeln!(out, "{indent}{detail_style}{note}{detail_style:#}");
                }
                Ok(())
            };
        }
    }

    out.push_str(&coverage_summary(&report.coverage));
    let totals = report.totals();
    let count = |n: usize, label: &str, when_nonzero: Style| {
        let s = if n > 0 { when_nonzero } else { Style::new() };
        format!("{s}{n} {label}{s:#}")
    };
    let _ = write!(
        out,
        "{} · {} · {}",
        count(totals.failed, "failed", style::FAIL),
        count(totals.passed, "passed", style::PASS),
        count(totals.inconclusive, "inconclusive", style::INCONCLUSIVE)
    );
    if totals.skipped > 0 {
        let _ = write!(out, " · {}", count(totals.skipped, "skipped", style::MUTED));
    }
    out.push('\n');
    // A fresh scaffold: every cell is `todo` or missing, so no case exists to run or to skip.
    let coverage = &report.coverage;
    if report.results.is_empty() && coverage.specified() == 0 && coverage.total() > 0 {
        let _ = writeln!(
            out,
            "{muted}nothing is specified yet: replace a todo with an expectation (rlsspec cover shows the matrix){muted:#}",
            muted = style::MUTED
        );
    }
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

/// One line per finding, grouped by rule, then the rules skipped and the totals.
pub fn render_lint(report: &LintReport) -> String {
    let width = |cell: fn(&crate::lint::Finding) -> usize| {
        report.findings.iter().map(cell).max().unwrap_or(0)
    };
    let object_width = width(|f| f.object.chars().count());
    let role_width = width(|f| f.role.as_ref().map_or(0, |r| r.chars().count()));
    let muted = style::MUTED;
    let mut out = String::new();
    for finding in &report.findings {
        let (s, mark) = severity_mark(finding.severity);
        let _ = write!(
            out,
            "{s}{mark}{s:#} {}  {:<object_width$}",
            finding.rule, finding.object
        );
        if role_width > 0 {
            let role = finding.role.as_deref().unwrap_or("");
            let _ = write!(out, "  {muted}{role:<role_width$}{muted:#}");
        }
        let _ = writeln!(out, "  {}", finding.hint);
    }
    for skipped in &report.skipped {
        let _ = writeln!(
            out,
            "{muted}{} skipped: {}{muted:#}",
            skipped.rule, skipped.reason
        );
    }

    let count = |n: usize, one: &str, many: &str, when_nonzero: Style| {
        let s = if n > 0 { when_nonzero } else { Style::new() };
        let label = if n == 1 { one } else { many };
        format!("{s}{n} {label}{s:#}")
    };
    let errors = report.count(Severity::Error);
    let warnings = report.count(Severity::Warn);
    let _ = writeln!(
        out,
        "{} · {} · {} · {}",
        count(errors, "error", "errors", style::FAIL),
        count(warnings, "warning", "warnings", style::INCONCLUSIVE),
        count(report.count(Severity::Info), "info", "info", Style::new()),
        count(report.ignored, "ignored", "ignored", style::MUTED),
    );
    out
}

fn severity_mark(severity: Severity) -> (Style, &'static str) {
    match severity {
        Severity::Error => (style::FAIL, "✗"),
        Severity::Warn => (style::INCONCLUSIVE, "!"),
        Severity::Info => (style::MUTED, "i"),
    }
}

/// A description cut to `DESCRIPTION_MAX` characters. The expectation a write case ends with
/// (`→ allow`, `→ deny`) is the part a reader looks for, so the cut is made before it.
fn cap_description(text: &str) -> String {
    for ending in [" → allow", " → deny"] {
        if let Some(head) = text.strip_suffix(ending) {
            let room = DESCRIPTION_MAX - ending.chars().count();
            return format!("{}{ending}", cap(head, room));
        }
    }
    cap(text, DESCRIPTION_MAX)
}

/// `text`, cut to `max` characters with a final `…` when it is longer.
fn cap(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_owned();
    }
    let mut out: String = text.chars().take(max.saturating_sub(1)).collect();
    out.push('…');
    out
}

/// Shortens every UUID to its last characters, as `shorten_literals` does: `id=…000a`.
fn shorten_uuids(text: &str) -> String {
    const GROUPS: [usize; 5] = [8, 4, 4, 4, 12];
    const LENGTH: usize = 36;
    let chars: Vec<char> = text.chars().collect();
    let is_uuid = |at: usize| {
        let Some(window) = chars.get(at..at + LENGTH) else {
            return false;
        };
        let mut groups = window.split(|c| *c == '-');
        let shaped = GROUPS.iter().all(|&len| {
            groups
                .next()
                .is_some_and(|g| g.len() == len && g.iter().all(char::is_ascii_hexdigit))
        });
        let bounded = |c: Option<&char>| !c.is_some_and(|c| c.is_alphanumeric() || *c == '-');
        shaped
            && groups.next().is_none()
            && bounded(at.checked_sub(1).and_then(|i| chars.get(i)))
            && bounded(chars.get(at + LENGTH))
    };
    let mut out = String::with_capacity(text.len());
    let mut at = 0;
    while at < chars.len() {
        if is_uuid(at) {
            out.push('…');
            out.extend(&chars[at + LENGTH - LITERAL_KEEP..at + LENGTH]);
            at += LENGTH;
        } else {
            out.push(chars[at]);
            at += 1;
        }
    }
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
    use crate::runner::coverage::TableCoverage;

    fn report() -> Report {
        let case = |identity: &str, outcome| CaseResult {
            table: "notes".into(),
            identity: identity.into(),
            op: Op::Select,
            description: "deny".into(),
            outcome,
            origin: Origin::Expect,
            span: crate::config::Span::default(),
        };
        Report {
            results: vec![
                case("alice", Outcome::Pass("0 visible".into())),
                case(
                    "guest",
                    Outcome::Fail("leaked 1 of the 2 rows it should not see: id=1".into()),
                ),
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
  ✗ guest  select  deny  leaked 1 of the 2 rows it should not see: id=1
  ? bob    select  deny  vacuous
coverage 0/0 cells · 0 unspecified (warn)
1 failed · 1 passed · 1 inconclusive
"
        );
    }

    #[test]
    fn folded_defaults_mark_inconclusive_and_both() {
        let case = |identity: &str, op, outcome| CaseResult {
            table: "notes".into(),
            identity: identity.into(),
            op,
            description: "deny".into(),
            outcome,
            origin: Origin::Default,
            span: crate::config::Span::default(),
        };
        let pass = || Outcome::Pass("ok".into());
        let unsure = || Outcome::Inconclusive("fk".into());
        let report = Report {
            results: vec![
                case("ana", Op::Select, pass()),
                case("ana", Op::Delete, unsure()),
                case("ben", Op::Insert, Outcome::Fail("policy".into())),
                case("ben", Op::Delete, unsure()),
                case("cleo", Op::Select, pass()),
                case("cleo", Op::Delete, pass()),
            ],
            ..Report::default()
        };
        assert_eq!(
            plain(&render(&report)),
            "notes
  ?  ana   *  deny  1/2 ops · delete: fk
  ✗? ben   *  deny  0/2 ops · insert: policy · delete: fk
  ✓  cleo  *  deny  2/2 ops
coverage 0/0 cells · 0 unspecified (warn)
1 failed · 3 passed · 2 inconclusive
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
    fn lint_report_aligns_findings_and_counts_by_severity() {
        use crate::lint::{Finding, Rule, Skipped};

        let finding = |rule: Rule, object: &str, role: Option<&str>, hint: &str| Finding {
            rule,
            severity: rule.severity(),
            object: object.into(),
            role: role.map(str::to_owned),
            hint: hint.into(),
            table: None,
            function: None,
            view: None,
            identities: Vec::new(),
            stale_ignore: None,
        };
        let report = LintReport {
            spec: "rlsspec.yaml".into(),
            findings: vec![
                finding(Rule::Rls001, "tags", None, "no RLS"),
                finding(Rule::Rls004, "notes", Some("app"), "USING (true)"),
                finding(Rule::Rls006, "audit_log", None, "no policy"),
            ],
            ignored: 2,
            skipped: vec![Skipped {
                rule: Rule::Rls007,
                reason: "needs PostgreSQL 15",
            }],
        };
        let styled = render_lint(&report);
        assert!(styled.contains('\x1b'), "{styled:?}");
        assert_eq!(
            plain(&styled),
            "✗ RLS001  tags            no RLS
! RLS004  notes      app  USING (true)
i RLS006  audit_log       no policy
RLS007 skipped: needs PostgreSQL 15
1 error · 1 warning · 1 info · 2 ignored
"
        );
    }

    #[test]
    fn uuids_in_samples_are_shortened() {
        assert_eq!(
            shorten_uuids(
                "leaked 2 of the 2 rows it should not see: id=0191e6a2-0000-7000-8000-00000000000a, (tenant_id, id)=(0191E6A2-0000-7000-8000-00000000000B, 4)"
            ),
            "leaked 2 of the 2 rows it should not see: id=…000a, (tenant_id, id)=(…000B, 4)"
        );
        // Not UUIDs: too short, too long, or part of a longer token.
        for text in [
            "id=0191e6a2-0000-7000-8000-00000000000",
            "id=0191e6a2-0000-7000-8000-00000000000a0",
            "x0191e6a2-0000-7000-8000-00000000000a",
            "0191e6a2-0000-7000-8000-00000000000a-1",
            "0191e6a2_0000-7000-8000-00000000000a",
        ] {
            assert_eq!(shorten_uuids(text), text);
        }
    }

    #[test]
    fn long_descriptions_are_cut() {
        assert_eq!(cap("deny", 64), "deny");
        let long = "x".repeat(65);
        let cut = cap(&long, 64);
        assert_eq!(cut.chars().count(), 64);
        assert!(cut.ends_with("xx…"));
        assert_eq!(cap(&"é".repeat(64), 64), "é".repeat(64));
        let insert = format!("values (body = '{}') → deny", "y".repeat(80));
        let cut = cap_description(&insert);
        assert_eq!(cut.chars().count(), 64);
        assert!(cut.ends_with("yy… → deny"), "{cut}");
        assert_eq!(cap_description("where true → allow"), "where true → allow");
    }

    fn result(
        identity: &str,
        op: Op,
        origin: Origin,
        description: &str,
        outcome: Outcome,
    ) -> CaseResult {
        CaseResult {
            table: "notes".into(),
            identity: identity.into(),
            op,
            description: description.into(),
            outcome,
            origin,
            span: crate::config::Span::default(),
        }
    }

    #[test]
    fn a_row_past_120_columns_puts_its_detail_on_the_next_line() {
        let uuid = "0191e6a2-0000-7000-8000-00000000000a";
        let leak = format!(
            "leaked 3 of the 3 rows it should not see: id={uuid}, id={uuid}, id={uuid} · hidden 1 of the 2 rows it should see: id={uuid}"
        );
        let report = Report {
            results: vec![
                result(
                    "alice",
                    Op::Select,
                    Origin::Expect,
                    "all",
                    Outcome::Pass("6 visible".into()),
                ),
                result(
                    "alice",
                    Op::Insert,
                    Origin::Expect,
                    &format!(
                        "values (id = 7, tenant_id = '{uuid}', author_id = '{uuid}', body = 'a note that is long enough to be cut') → allow"
                    ),
                    Outcome::Pass("inserted".into()),
                ),
                result(
                    "alice",
                    Op::Select,
                    Origin::Expect,
                    "rows: published",
                    Outcome::Fail(leak),
                ),
            ],
            ..Report::default()
        };
        assert_eq!(
            plain(&render(&report)),
            "notes
  ✓ alice  select  all                                                               6 visible
  ✓ alice  insert  values (id = 7, tenant_id = '…000a', author_id = '…000a… → allow  inserted
  ✗ alice  select  rows: published
      leaked 3 of the 3 rows it should not see: id=…000a, id=…000a, id=…000a · hidden 1 of the 2 rows it should see: id=…000a
coverage 0/0 cells · 0 unspecified (warn)
1 failed · 2 passed · 0 inconclusive
"
        );
    }

    #[test]
    fn a_folded_line_past_120_columns_puts_one_operation_per_line() {
        let vacuous = "vacuous: \"public\".\"drafts\" has no rows, so this check cannot fail; add rows in setup (rlsspec.yaml:11:20)";
        let default =
            |identity: &str, op, outcome| result(identity, op, Origin::Default, "deny", outcome);
        let report = Report {
            results: vec![
                default("guest", Op::Select, Outcome::Inconclusive(vacuous.into())),
                default(
                    "guest",
                    Op::Insert,
                    Outcome::Pass("deny (no privilege)".into()),
                ),
                default("guest", Op::Update, Outcome::Inconclusive(vacuous.into())),
                default(
                    "guest",
                    Op::Delete,
                    Outcome::Skipped(crate::runner::Skip::EmptyTable),
                ),
                default(
                    "alice",
                    Op::Select,
                    Outcome::Fail("leaked 1 of the 1 row it should not see: id=1".into()),
                ),
                default(
                    "alice",
                    Op::Delete,
                    Outcome::Pass("permission denied".into()),
                ),
            ],
            ..Report::default()
        };
        assert_eq!(
            plain(&render(&report)),
            format!(
                "notes
  ? guest  *  deny  1/3 ops
      select: {vacuous}
      update: {vacuous}
      1 skipped (empty table)
  ✗ alice  *  deny  1/2 ops · select: leaked 1 of the 1 row it should not see: id=1
coverage 0/0 cells · 0 unspecified (warn)
1 failed · 2 passed · 2 inconclusive · 1 skipped
"
            )
        );
    }

    #[test]
    fn a_run_with_nothing_specified_ends_with_a_hint() {
        let hint = "nothing is specified yet: replace a todo with an expectation (rlsspec cover shows the matrix)\n";
        let mut report = Report {
            coverage: coverage(Unspecified::Warn, [[false; 4]; 2]),
            ..Report::default()
        };
        assert!(plain(&render(&report)).ends_with(&format!("0 inconclusive\n{hint}")));
        // One specified cell, or no cell at all, and there is nothing to hint at.
        report.coverage.tables[0].cells[0][0] = true;
        assert!(!render(&report).contains("nothing is specified"));
        assert!(!render(&Report::default()).contains("nothing is specified"));
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
