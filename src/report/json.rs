use std::path::Path;

use serde::Serialize;

use crate::config::Span;
use crate::lint::{self, LintReport, Severity};
use crate::runner::coverage::Coverage;
use crate::runner::{Origin, Outcome, Report};

/// Bumped on any breaking change to the documents below (a field renamed, removed or retyped).
pub const SCHEMA_VERSION: u32 = 1;

#[derive(Serialize)]
struct TestDocument<'a> {
    schema_version: u32,
    command: &'static str,
    cases: Vec<Case<'a>>,
    coverage: CoverageSummary<'a>,
    totals: TestTotals,
    exit_code: u8,
}

#[derive(Serialize)]
struct Case<'a> {
    table: &'a str,
    identity: &'a str,
    op: &'static str,
    origin: &'static str,
    description: &'a str,
    outcome: &'static str,
    detail: &'a str,
    location: Location,
}

#[derive(Serialize)]
struct Location {
    file: String,
    line: usize,
    column: usize,
}

impl Location {
    fn new(file: &Path, span: Span) -> Self {
        Self {
            file: file.display().to_string(),
            line: span.line,
            column: span.column,
        }
    }
}

#[derive(Serialize)]
struct TestTotals {
    passed: usize,
    failed: usize,
    inconclusive: usize,
}

#[derive(Serialize)]
struct CoverageSummary<'a> {
    policy: &'static str,
    total: usize,
    specified: usize,
    unspecified: usize,
    gaps: Vec<Gap<'a>>,
}

#[derive(Serialize)]
struct Gap<'a> {
    table: &'a str,
    identity: &'a str,
    ops: Vec<&'static str>,
}

fn coverage_summary(coverage: &Coverage) -> CoverageSummary<'_> {
    let gaps = coverage
        .gaps()
        .into_iter()
        .flat_map(|(table, gaps)| {
            gaps.into_iter().map(move |gap| Gap {
                table,
                identity: gap.identity,
                ops: gap.ops.iter().map(|op| op.as_str()).collect(),
            })
        })
        .collect();
    CoverageSummary {
        policy: coverage.policy.as_str(),
        total: coverage.total(),
        specified: coverage.specified(),
        unspecified: coverage.unspecified(),
        gaps,
    }
}

fn document(value: &impl Serialize) -> Result<String, serde_json::Error> {
    let mut out = serde_json::to_string_pretty(value)?;
    out.push('\n');
    Ok(out)
}

/// Every case, never folded: grouping `defaults` cases on one line is a text-only presentation.
pub fn render(report: &Report) -> Result<String, serde_json::Error> {
    let cases = report
        .results
        .iter()
        .map(|result| {
            let (outcome, detail) = match &result.outcome {
                Outcome::Pass(detail) => ("pass", detail),
                Outcome::Fail(detail) => ("fail", detail),
                Outcome::Inconclusive(detail) => ("inconclusive", detail),
            };
            Case {
                table: &result.table,
                identity: &result.identity,
                op: result.op.as_str(),
                origin: match result.origin {
                    Origin::Expect => "expect",
                    Origin::Default => "default",
                },
                description: &result.description,
                outcome,
                detail,
                location: Location::new(&report.spec, result.span),
            }
        })
        .collect();
    let totals = report.totals();
    document(&TestDocument {
        schema_version: SCHEMA_VERSION,
        command: "test",
        cases,
        coverage: coverage_summary(&report.coverage),
        totals: TestTotals {
            passed: totals.passed,
            failed: totals.failed,
            inconclusive: totals.inconclusive,
        },
        exit_code: report.exit_code(),
    })
}

#[derive(Serialize)]
struct CoverDocument<'a> {
    schema_version: u32,
    command: &'static str,
    identities: &'a [String],
    tables: Vec<CoverTable<'a>>,
    coverage: CoverageSummary<'a>,
    exit_code: u8,
}

#[derive(Serialize)]
struct CoverTable<'a> {
    table: &'a str,
    cells: Vec<Cell<'a>>,
}

#[derive(Serialize)]
struct Cell<'a> {
    identity: &'a str,
    select: bool,
    insert: bool,
    update: bool,
    delete: bool,
}

pub fn render_cover(coverage: &Coverage) -> Result<String, serde_json::Error> {
    let tables = coverage
        .tables
        .iter()
        .map(|table| CoverTable {
            table: &table.table,
            cells: coverage
                .identities
                .iter()
                .zip(&table.cells)
                .map(|(identity, &[select, insert, update, delete])| Cell {
                    identity,
                    select,
                    insert,
                    update,
                    delete,
                })
                .collect(),
        })
        .collect();
    document(&CoverDocument {
        schema_version: SCHEMA_VERSION,
        command: "cover",
        identities: &coverage.identities,
        tables,
        coverage: coverage_summary(coverage),
        exit_code: u8::from(coverage.fails()),
    })
}

#[derive(Serialize)]
struct LintDocument<'a> {
    schema_version: u32,
    command: &'static str,
    findings: Vec<Finding<'a>>,
    ignored: usize,
    skipped: Vec<Skipped>,
    totals: LintTotals,
    exit_code: u8,
}

#[derive(Serialize)]
struct Finding<'a> {
    rule: &'static str,
    severity: &'static str,
    object: &'a str,
    role: Option<&'a str>,
    hint: &'a str,
    table: Option<Name<'a>>,
    function: Option<Name<'a>>,
    view: Option<Name<'a>>,
    identities: &'a [String],
    stale_ignore: Option<Location>,
}

#[derive(Serialize)]
struct Name<'a> {
    schema: &'a str,
    name: &'a str,
}

impl<'a> Name<'a> {
    fn new(name: &'a Option<lint::Name>) -> Option<Self> {
        name.as_ref().map(|n| Self {
            schema: &n.schema,
            name: &n.name,
        })
    }
}

#[derive(Serialize)]
struct Skipped {
    rule: &'static str,
    reason: &'static str,
}

#[derive(Serialize)]
struct LintTotals {
    error: usize,
    warn: usize,
    info: usize,
}

pub fn render_lint(report: &LintReport) -> Result<String, serde_json::Error> {
    let findings = report
        .findings
        .iter()
        .map(|finding| Finding {
            rule: finding.rule.id(),
            severity: finding.severity.as_str(),
            object: &finding.object,
            role: finding.role.as_deref(),
            hint: &finding.hint,
            table: Name::new(&finding.table),
            function: Name::new(&finding.function),
            view: Name::new(&finding.view),
            identities: &finding.identities,
            stale_ignore: finding
                .stale_ignore
                .map(|span| Location::new(&report.spec, span)),
        })
        .collect();
    let skipped = report
        .skipped
        .iter()
        .map(|s| Skipped {
            rule: s.rule.id(),
            reason: s.reason,
        })
        .collect();
    document(&LintDocument {
        schema_version: SCHEMA_VERSION,
        command: "lint",
        findings,
        ignored: report.ignored,
        skipped,
        totals: LintTotals {
            error: report.count(Severity::Error),
            warn: report.count(Severity::Warn),
            info: report.count(Severity::Info),
        },
        exit_code: report.exit_code(),
    })
}
