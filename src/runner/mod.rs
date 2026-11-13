mod affected;
pub mod coverage;
mod insert_deny;
pub(crate) mod plan;
mod rows;
mod select;
mod write;

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use postgres::Transaction;

use crate::catalog::{self, Catalog, ExcludeError, Table};
use crate::config::{
    Affects, Assignment, Block, Config, ConfigError, Diagnostic, Identity, Source, Span, TableRef,
    Writes,
};
use crate::identity;
use crate::pg::{self, PgError, Session, Target};
use coverage::Coverage;
use write::Modify;

#[derive(Debug, thiserror::Error)]
pub enum RunError {
    #[error(transparent)]
    Pg(#[from] PgError),
    #[error(transparent)]
    Config(ConfigError),
    #[error(
        "database.url role must be superuser, have BYPASSRLS, or own the tables without FORCE (reading {table} with row_security = off failed: {detail})"
    )]
    AdminFiltered { table: String, detail: String },
}

impl From<postgres::Error> for RunError {
    fn from(err: postgres::Error) -> Self {
        Self::Pg(PgError::Query(err))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Op {
    Select,
    Insert,
    Update,
    Delete,
}

impl Op {
    pub const ALL: [Op; 4] = [Op::Select, Op::Insert, Op::Update, Op::Delete];

    pub fn as_str(self) -> &'static str {
        match self {
            Op::Select => "select",
            Op::Insert => "insert",
            Op::Update => "update",
            Op::Delete => "delete",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    Pass(String),
    Fail(String),
    Inconclusive(String),
    /// Not run, on purpose: not a pass, not inconclusive. Its cell counts as unspecified.
    Skipped(Skip),
}

/// Why a case was not run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Skip {
    /// A wildcard default with `on_empty: skip`, on a table with no rows.
    EmptyTable,
}

impl Skip {
    /// The stable identifier, as written in machine-readable reports.
    pub fn id(self) -> &'static str {
        match self {
            Skip::EmptyTable => "empty_table",
        }
    }

    pub fn text(self) -> &'static str {
        match self {
            Skip::EmptyTable => "empty table",
        }
    }
}

/// Where a case came from: an `expect` entry, or `defaults` (a named table or `*`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Origin {
    Expect,
    Default,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CaseResult {
    pub table: String,
    pub identity: String,
    pub op: Op,
    pub description: String,
    pub outcome: Outcome,
    pub origin: Origin,
    /// Where the case is written in the spec: the `expect` entry, or the `defaults` one it came from.
    pub span: Span,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Report {
    /// The spec file the run loaded.
    pub spec: PathBuf,
    pub results: Vec<CaseResult>,
    pub coverage: Coverage,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Totals {
    pub passed: usize,
    pub failed: usize,
    pub inconclusive: usize,
    pub skipped: usize,
}

impl Report {
    fn push(&mut self, result: CaseResult) {
        self.results.push(result);
    }

    pub fn totals(&self) -> Totals {
        let mut totals = Totals::default();
        for result in &self.results {
            match result.outcome {
                Outcome::Pass(_) => totals.passed += 1,
                Outcome::Fail(_) => totals.failed += 1,
                Outcome::Inconclusive(_) => totals.inconclusive += 1,
                Outcome::Skipped(_) => totals.skipped += 1,
            }
        }
        totals
    }

    /// 2 when anything is inconclusive (the run is incomplete), else 1 on failures or on
    /// unspecified cells under `unspecified: fail`, else 0. A skipped case counts through its
    /// cell, which is unspecified.
    pub fn exit_code(&self) -> u8 {
        let totals = self.totals();
        if totals.inconclusive > 0 {
            2
        } else if totals.failed > 0 || self.coverage.fails() {
            1
        } else {
            0
        }
    }
}

/// How `in_session` checks the identities before handing over: by applying each one, or, for
/// `lint`, which never acts as an identity, only by checking that its role exists.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Preflight {
    Apply,
    RolesExist,
}

pub fn run(config: &Config, source: &Source, target: &Target) -> Result<Report, RunError> {
    in_session(
        config,
        source,
        target,
        Preflight::Apply,
        |session, catalog, plan| {
            let mut report = Report {
                spec: source.path.clone(),
                results: Vec::new(),
                coverage: coverage::compute(config, catalog, plan),
            };
            for entry in plan {
                let cell = Cell {
                    entry,
                    name: catalog.display_name(entry.table),
                    path: &source.path,
                };
                cell.check(session, &mut report)?;
            }
            Ok(report)
        },
    )
}

/// Everything `run` does before the first case (setup, catalog, spec validation), then the coverage.
pub fn cover(config: &Config, source: &Source, target: &Target) -> Result<Coverage, RunError> {
    in_session(
        config,
        source,
        target,
        Preflight::Apply,
        |_, catalog, plan| Ok(coverage::compute(config, catalog, plan)),
    )
}

/// Everything `cover` does, except that no identity is applied: their roles must only exist.
pub(crate) fn inspect<T>(
    config: &Config,
    source: &Source,
    target: &Target,
    f: impl FnOnce(&mut Session, &Catalog, &[plan::Entry]) -> Result<T, RunError>,
) -> Result<T, RunError> {
    in_session(config, source, target, Preflight::RolesExist, f)
}

fn in_session<T>(
    config: &Config,
    source: &Source,
    target: &Target,
    preflight: Preflight,
    f: impl FnOnce(&mut Session, &Catalog, &[plan::Entry]) -> Result<T, RunError>,
) -> Result<T, RunError> {
    let mut client = pg::connect(target)?;
    let mut session = Session::begin(&mut client, &config.safety)?;
    session.run_setup(&config.setup)?;
    let mut catalog = catalog::load(session.tx(), &config.database.schemas)?;

    let mut diagnostics = Vec::new();
    for entry in &config.database.exclude {
        // An exclusion that excludes nothing is a typo or a leftover, like an unknown table in `expect`.
        if let Err(err) = catalog.exclude(&entry.table, entry.span.line) {
            let message = match err {
                ExcludeError::Unknown(reason) => {
                    format!("{reason}; a `database.exclude` entry must name a table in scope")
                }
                ExcludeError::Twice(line) => {
                    format!("table `{}` is already excluded at line {line}", entry.table)
                }
            };
            diagnostics.push(Diagnostic {
                span: entry.span,
                message,
            });
        }
    }
    let tables = resolve_tables(config, &catalog, &mut diagnostics);
    let mut plan = plan::build(config, &tables, &catalog, &mut diagnostics);
    for entry in &plan {
        check_columns(entry, &catalog, &mut diagnostics);
    }
    match preflight {
        Preflight::Apply => apply_identities(&mut session, &config.identities, &mut diagnostics)?,
        Preflight::RolesExist => check_roles(&mut session, &config.identities, &mut diagnostics)?,
    }
    if !diagnostics.is_empty() {
        diagnostics.sort_by_key(|d| d.span);
        diagnostics.dedup();
        return Err(RunError::Config(source.invalid(diagnostics)));
    }

    mark_empty(&mut session, &mut plan)?;
    let out = f(&mut session, &catalog, &plan)?;
    session.rollback()?;
    Ok(out)
}

struct Cell<'a> {
    entry: &'a plan::Entry<'a>,
    name: String,
    path: &'a Path,
}

/// Where a case came from and where it is written.
type Written = (Origin, Span);

impl Cell<'_> {
    fn at(&self, span: Span) -> Location<'_> {
        Location(self.path, span)
    }

    fn push(
        &self,
        report: &mut Report,
        op: Op,
        (origin, span): Written,
        description: String,
        outcome: Outcome,
    ) {
        report.push(CaseResult {
            table: self.name.clone(),
            identity: self.entry.identity.name.clone(),
            op,
            description,
            outcome,
            origin,
            span,
        });
    }

    /// Records a case of `op`: skipped when its wildcard default says so and the table is empty,
    /// otherwise run in its own savepoint.
    fn case(
        &self,
        session: &mut Session,
        report: &mut Report,
        op: Op,
        from: Written,
        description: String,
        run: impl FnOnce(&mut Transaction, Location) -> Result<Outcome, RunError>,
    ) -> Result<(), RunError> {
        let outcome = if self.entry.skips(op) {
            Outcome::Skipped(Skip::EmptyTable)
        } else {
            let at = self.at(from.1);
            let outcome = session.case(|tx| run(tx, at))??;
            self.hinted(op, outcome, at)
        };
        self.push(report, op, from, description, outcome);
        Ok(())
    }

    /// A vacuous case that a wildcard default put on an empty table says how to skip it.
    fn hinted(&self, op: Op, outcome: Outcome, at: Location) -> Outcome {
        match outcome {
            Outcome::Inconclusive(text)
                if text.starts_with(VACUOUS) && self.entry.hints_on_empty(op) =>
            {
                Outcome::Inconclusive(empty_table(self.entry.table, at, true))
            }
            other => other,
        }
    }

    fn check(&self, session: &mut Session, report: &mut Report) -> Result<(), RunError> {
        self.select(session, report)?;
        self.insert(session, report)?;
        self.update(session, report)?;
        self.delete(session, report)
    }

    fn select(&self, session: &mut Session, report: &mut Report) -> Result<(), RunError> {
        let (table, identity) = (self.entry.table, self.entry.identity);
        let Some(planned) = self.entry.select else {
            return Ok(());
        };
        let Some(case) = planned.given() else {
            return Ok(());
        };
        let from = (planned.origin, case.span);
        let description = select::describe(&case.select);
        self.case(session, report, Op::Select, from, description, |tx, at| {
            select::check(tx, table, identity, &case.select, at)
        })
    }

    fn insert(&self, session: &mut Session, report: &mut Report) -> Result<(), RunError> {
        let (table, identity) = (self.entry.table, self.entry.identity);
        let Some(planned) = self.entry.insert else {
            return Ok(());
        };
        match planned.given() {
            // `insert: allow` is rejected when the config is loaded.
            Some(Writes::Shorthand { expect, span }) => {
                let description = write::expectation(*expect).to_owned();
                let from = (planned.origin, *span);
                self.case(session, report, Op::Insert, from, description, |tx, _| {
                    Ok(insert_deny::check(tx, table, &self.name, identity)?)
                })?;
            }
            Some(Writes::Cases(cases)) => {
                for case in cases {
                    let description = write::describe_insert(case);
                    let from = (planned.origin, case.span);
                    self.case(session, report, Op::Insert, from, description, |tx, at| {
                        write::insert(tx, table, identity, case, at)
                    })?;
                }
            }
            None => {}
        }
        Ok(())
    }

    fn update(&self, session: &mut Session, report: &mut Report) -> Result<(), RunError> {
        let (table, identity) = (self.entry.table, self.entry.identity);
        let Some(planned) = self.entry.update else {
            return Ok(());
        };
        match planned.given() {
            Some(Writes::Shorthand { expect, span }) => {
                let description = write::expectation(*expect).to_owned();
                let from = (planned.origin, *span);
                self.case(session, report, Op::Update, from, description, |tx, at| {
                    write::modify(tx, table, identity, Modify::Update(None), None, *expect, at)
                })?;
            }
            Some(Writes::Cases(cases)) => {
                for case in cases {
                    let set = case.set.as_deref();
                    let from = (planned.origin, case.span);
                    self.modify(session, report, Op::Update, from, &case.target, set)?;
                }
            }
            None => {}
        }
        Ok(())
    }

    fn delete(&self, session: &mut Session, report: &mut Report) -> Result<(), RunError> {
        let (table, identity) = (self.entry.table, self.entry.identity);
        let Some(planned) = self.entry.delete else {
            return Ok(());
        };
        match planned.given() {
            Some(Writes::Shorthand { expect, span }) => {
                let description = write::expectation(*expect).to_owned();
                let from = (planned.origin, *span);
                self.case(session, report, Op::Delete, from, description, |tx, at| {
                    write::modify(tx, table, identity, Modify::Delete, None, *expect, at)
                })?;
            }
            Some(Writes::Cases(cases)) => {
                for case in cases {
                    let from = (planned.origin, case.span);
                    self.modify(session, report, Op::Delete, from, &case.target, None)?;
                }
            }
            None => {}
        }
        Ok(())
    }

    /// One update (with its `set`, if any) or delete case, in either form.
    fn modify(
        &self,
        session: &mut Session,
        report: &mut Report,
        op: Op,
        from: Written,
        target: &Affects,
        set: Option<&[Assignment]>,
    ) -> Result<(), RunError> {
        let (table, identity) = (self.entry.table, self.entry.identity);
        let kind = match op {
            Op::Delete => Modify::Delete,
            _ => Modify::Update(set),
        };
        match target {
            Affects::Where { predicate, expect } => {
                let description = write::describe_modify(predicate, set, *expect);
                self.case(session, report, op, from, description, |tx, at| {
                    write::modify(tx, table, identity, kind, Some(predicate), *expect, at)
                })
            }
            Affects::Rows { predicate, subset } => {
                let description = affected::describe(predicate, set, *subset);
                self.case(session, report, op, from, description, |tx, at| {
                    affected::check(tx, table, identity, kind, predicate, *subset, at)
                })
            }
        }
    }
}

/// How every message of a case that cannot fail starts (hard rule 2).
const VACUOUS: &str = "vacuous: ";

/// A case over a table with no rows. `wildcard`: it was expanded from a wildcard default, which
/// can say `on_empty: skip`.
fn empty_table(table: &Table, at: Location, wildcard: bool) -> String {
    let way_out = if wildcard {
        "add rows in setup, or set on_empty: skip on this \"*\" default"
    } else {
        "add rows in setup"
    };
    format!(
        "{VACUOUS}{} has no rows, so this check cannot fail; {way_out} ({at})",
        table.sql_name()
    )
}

/// A case whose predicate matches no row.
fn no_match(at: Location) -> String {
    format!("{VACUOUS}the predicate matches no rows, so this check cannot fail ({at})")
}

/// Looks up whether the table is empty for the entries that have cases from a wildcard default.
fn mark_empty(session: &mut Session, plan: &mut [plan::Entry]) -> Result<(), RunError> {
    let mut known: HashMap<u32, bool> = HashMap::new();
    for entry in plan {
        if !Op::ALL.into_iter().any(|op| entry.asks_about_empty(op)) {
            continue;
        }
        entry.empty = match known.get(&entry.table.oid) {
            Some(empty) => *empty,
            None => {
                let empty = is_empty(session, entry.table)?;
                known.insert(entry.table.oid, empty);
                empty
            }
        };
    }
    Ok(())
}

/// Whether `table` has no rows, as the admin. When that can't be read the table counts as not
/// empty: its cases run and report the problem themselves.
fn is_empty(session: &mut Session, table: &Table) -> Result<bool, RunError> {
    let sql = format!("SELECT NOT EXISTS (SELECT FROM {})", table.sql_name());
    let empty = session.case(|tx| {
        pg::set_local(tx, "row_security", "off")?;
        tx.query_one(&sql, &[]).map(|row| row.get::<_, bool>(0))
    })?;
    Ok(empty.unwrap_or(false))
}

/// Every column in `values` and `set` must exist and be writable (not generated).
fn check_columns(entry: &plan::Entry, catalog: &Catalog, diagnostics: &mut Vec<Diagnostic>) {
    let table = entry.table;
    let inserts = match entry.insert.and_then(|p| p.given()) {
        Some(Writes::Cases(cases)) => cases.iter().map(|c| c.values.as_slice()).collect(),
        _ => Vec::new(),
    };
    let updates = match entry.update.and_then(|p| p.given()) {
        Some(Writes::Cases(cases)) => cases.iter().filter_map(|c| c.set.as_deref()).collect(),
        _ => Vec::new(),
    };
    let name = catalog.display_name(table);
    for assignment in inserts.into_iter().chain(updates).flatten() {
        let message = match table.column(&assignment.column) {
            None => format!("column `{}` not found in table {name}", assignment.column),
            Some(column) if column.generated => format!(
                "column `{}` of {name} is generated and cannot be written",
                assignment.column
            ),
            Some(_) => continue,
        };
        diagnostics.push(Diagnostic {
            span: assignment.span,
            message,
        });
    }
}

#[derive(Debug, Clone, Copy)]
struct Location<'a>(&'a Path, Span);

impl std::fmt::Display for Location<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "{}:{}:{}", self.0.display(), self.1.line, self.1.column)
    }
}

fn resolve_tables<'c>(
    config: &Config,
    catalog: &'c Catalog,
    diagnostics: &mut Vec<Diagnostic>,
) -> Vec<Option<&'c Table>> {
    let mut resolve = |block: &Block| match &block.table {
        TableRef::All => None,
        TableRef::Named(name) => match catalog.resolve(name) {
            Ok(table) => Some(table),
            Err(message) => {
                // Dropping the entry silently would turn written expectations into nothing.
                let message = match catalog.excluded_at(name) {
                    Some(line) => format!(
                        "table `{name}` is excluded by `database.exclude` at line {line}, so nothing written for it would run; remove this entry or the exclusion"
                    ),
                    None => message,
                };
                diagnostics.push(Diagnostic {
                    span: block.table_span,
                    message,
                });
                None
            }
        },
    };
    config
        .expect
        .iter()
        .chain(&config.defaults)
        .map(&mut resolve)
        .collect()
}

fn apply_identities(
    session: &mut Session,
    identities: &[Identity],
    diagnostics: &mut Vec<Diagnostic>,
) -> Result<(), RunError> {
    for identity in identities {
        if let Err(err) = session.case(|tx| identity::apply(tx, identity))? {
            diagnostics.push(Diagnostic {
                span: identity.span,
                message: format!(
                    "cannot act as identity `{}`: {}",
                    identity.name,
                    pg::describe(&err)
                ),
            });
        }
    }
    Ok(())
}

fn check_roles(
    session: &mut Session,
    identities: &[Identity],
    diagnostics: &mut Vec<Diagnostic>,
) -> Result<(), RunError> {
    let roles: Vec<&str> = identities.iter().map(|i| i.role.as_str()).collect();
    let missing = catalog::lint::missing_roles(session.tx(), &roles)?;
    for identity in identities.iter().filter(|i| missing.contains(&i.role)) {
        diagnostics.push(Diagnostic {
            span: identity.span,
            message: format!(
                "role `{}` of identity `{}` does not exist",
                identity.role, identity.name
            ),
        });
    }
    Ok(())
}
