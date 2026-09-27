pub mod coverage;
mod insert_deny;
pub(crate) mod plan;
mod select;
mod write;

use std::path::{Path, PathBuf};

use crate::catalog::{self, Catalog, Table};
use crate::config::{
    Block, Config, ConfigError, Diagnostic, Identity, Source, Span, Spec, TableRef, Writes,
};
use crate::identity;
use crate::pg::{self, PgError, Session};
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
            }
        }
        totals
    }

    /// 2 when anything is inconclusive (the run is incomplete), else 1 on failures or on
    /// unspecified cells under `unspecified: fail`, else 0.
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

pub fn run(config: &Config, source: &Source) -> Result<Report, RunError> {
    in_session(
        config,
        source,
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
pub fn cover(config: &Config, source: &Source) -> Result<Coverage, RunError> {
    in_session(config, source, Preflight::Apply, |_, catalog, plan| {
        Ok(coverage::compute(config, catalog, plan))
    })
}

/// Everything `cover` does, except that no identity is applied: their roles must only exist.
pub(crate) fn inspect<T>(
    config: &Config,
    source: &Source,
    f: impl FnOnce(&mut Session, &Catalog, &[plan::Entry]) -> Result<T, RunError>,
) -> Result<T, RunError> {
    in_session(config, source, Preflight::RolesExist, f)
}

fn in_session<T>(
    config: &Config,
    source: &Source,
    preflight: Preflight,
    f: impl FnOnce(&mut Session, &Catalog, &[plan::Entry]) -> Result<T, RunError>,
) -> Result<T, RunError> {
    let mut client = pg::connect(&config.database.url)?;
    let mut session = Session::begin(&mut client, &config.safety)?;
    session.run_setup(&config.setup)?;
    let catalog = catalog::load(session.tx(), &config.database.schemas)?;

    let mut diagnostics = Vec::new();
    let tables = resolve_tables(config, &catalog, &mut diagnostics);
    let plan = plan::build(config, &tables, &catalog, &mut diagnostics);
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

    let out = f(&mut session, &catalog, &plan)?;
    session.rollback()?;
    Ok(out)
}

struct Cell<'a> {
    entry: &'a plan::Entry<'a>,
    name: String,
    path: &'a Path,
}

impl Cell<'_> {
    fn at(&self, span: Span) -> Location<'_> {
        Location(self.path, span)
    }

    fn push(
        &self,
        report: &mut Report,
        op: Op,
        (origin, span): (Origin, Span),
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

    fn check(&self, session: &mut Session, report: &mut Report) -> Result<(), RunError> {
        let (table, identity) = (self.entry.table, self.entry.identity);
        if let Some((Spec::Given(case), origin)) = self.entry.select {
            let at = self.at(case.span);
            let outcome =
                session.case(|tx| select::check(tx, table, identity, &case.select, at))??;
            self.push(
                report,
                Op::Select,
                (origin, case.span),
                select::describe(&case.select),
                outcome,
            );
        }
        match self.entry.insert {
            // `insert: allow` is rejected when the config is loaded.
            Some((Spec::Given(Writes::Shorthand { expect, span }), origin)) => {
                let outcome =
                    session.case(|tx| insert_deny::check(tx, table, &self.name, identity))??;
                let description = write::expectation(*expect).to_owned();
                self.push(report, Op::Insert, (origin, *span), description, outcome);
            }
            Some((Spec::Given(Writes::Cases(cases)), origin)) => {
                for case in cases {
                    let at = self.at(case.span);
                    let outcome =
                        session.case(|tx| write::insert(tx, table, identity, case, at))??;
                    self.push(
                        report,
                        Op::Insert,
                        (origin, case.span),
                        write::describe_insert(case),
                        outcome,
                    );
                }
            }
            Some((Spec::Todo, _)) | None => {}
        }
        match self.entry.update {
            Some((Spec::Given(Writes::Shorthand { expect, span }), origin)) => {
                let at = self.at(*span);
                let outcome = session.case(|tx| {
                    write::modify(tx, table, identity, Modify::Update(None), None, *expect, at)
                })??;
                let description = write::expectation(*expect).to_owned();
                self.push(report, Op::Update, (origin, *span), description, outcome);
            }
            Some((Spec::Given(Writes::Cases(cases)), origin)) => {
                for case in cases {
                    let at = self.at(case.span);
                    let set = case.set.as_deref();
                    let outcome = session.case(|tx| {
                        let kind = Modify::Update(set);
                        write::modify(
                            tx,
                            table,
                            identity,
                            kind,
                            Some(&case.predicate),
                            case.expect,
                            at,
                        )
                    })??;
                    let description = write::describe_modify(&case.predicate, set, case.expect);
                    self.push(
                        report,
                        Op::Update,
                        (origin, case.span),
                        description,
                        outcome,
                    );
                }
            }
            Some((Spec::Todo, _)) | None => {}
        }
        match self.entry.delete {
            Some((Spec::Given(Writes::Shorthand { expect, span }), origin)) => {
                let at = self.at(*span);
                let outcome = session.case(|tx| {
                    write::modify(tx, table, identity, Modify::Delete, None, *expect, at)
                })??;
                let description = write::expectation(*expect).to_owned();
                self.push(report, Op::Delete, (origin, *span), description, outcome);
            }
            Some((Spec::Given(Writes::Cases(cases)), origin)) => {
                for case in cases {
                    let at = self.at(case.span);
                    let outcome = session.case(|tx| {
                        let predicate = Some(case.predicate.as_str());
                        write::modify(
                            tx,
                            table,
                            identity,
                            Modify::Delete,
                            predicate,
                            case.expect,
                            at,
                        )
                    })??;
                    let description = write::describe_modify(&case.predicate, None, case.expect);
                    self.push(
                        report,
                        Op::Delete,
                        (origin, case.span),
                        description,
                        outcome,
                    );
                }
            }
            Some((Spec::Todo, _)) | None => {}
        }
        Ok(())
    }
}

/// Every column in `values` and `set` must exist and be writable (not generated).
fn check_columns(entry: &plan::Entry, catalog: &Catalog, diagnostics: &mut Vec<Diagnostic>) {
    let table = entry.table;
    let inserts = match entry.insert {
        Some((Spec::Given(Writes::Cases(cases)), _)) => {
            cases.iter().map(|c| c.values.as_slice()).collect()
        }
        _ => Vec::new(),
    };
    let updates = match entry.update {
        Some((Spec::Given(Writes::Cases(cases)), _)) => {
            cases.iter().filter_map(|c| c.set.as_deref()).collect()
        }
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
