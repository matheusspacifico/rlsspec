mod insert_deny;
mod select;
mod write;

use std::collections::HashMap;
use std::path::Path;

use crate::catalog::{self, Catalog, Table};
use crate::config::{
    Block, Config, ConfigError, Diagnostic, Expectation, Identity, Ops, Source, Span, TableRef,
    Writes,
};
use crate::identity;
use crate::pg::{self, PgError, Session};
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
    Unsupported,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CaseResult {
    pub table: String,
    pub identity: String,
    pub op: Op,
    pub description: String,
    pub outcome: Outcome,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Report {
    pub results: Vec<CaseResult>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Totals {
    pub passed: usize,
    pub failed: usize,
    pub inconclusive: usize,
}

impl Report {
    fn push(&mut self, table: &str, identity: &str, op: Op, description: String, outcome: Outcome) {
        self.results.push(CaseResult {
            table: table.to_owned(),
            identity: identity.to_owned(),
            op,
            description,
            outcome,
        });
    }

    pub fn totals(&self) -> Totals {
        let mut totals = Totals::default();
        for result in &self.results {
            match result.outcome {
                Outcome::Pass(_) => totals.passed += 1,
                Outcome::Fail(_) => totals.failed += 1,
                Outcome::Inconclusive(_) | Outcome::Unsupported => totals.inconclusive += 1,
            }
        }
        totals
    }

    /// 2 when anything is inconclusive (the run is incomplete), else 1 on failures, else 0.
    pub fn exit_code(&self) -> u8 {
        let totals = self.totals();
        if totals.inconclusive > 0 {
            2
        } else if totals.failed > 0 {
            1
        } else {
            0
        }
    }
}

pub fn run(config: &Config, source: &Source) -> Result<Report, RunError> {
    let mut client = pg::connect(&config.database.url)?;
    let mut session = Session::begin(&mut client, &config.safety)?;
    session.run_setup(&config.setup)?;
    let catalog = catalog::load(session.tx(), &config.database.schemas)?;

    let mut diagnostics = Vec::new();
    let tables = resolve_tables(config, &catalog, &mut diagnostics);
    for (block, table) in config.expect.iter().chain(&config.defaults).zip(&tables) {
        if let Some(table) = table {
            check_columns(&block.ops, table, &catalog, &mut diagnostics);
        }
    }
    preflight(&mut session, &config.identities, &mut diagnostics)?;
    if !diagnostics.is_empty() {
        diagnostics.sort_by_key(|d| d.span);
        diagnostics.dedup();
        return Err(RunError::Config(source.invalid(diagnostics)));
    }

    let identities: HashMap<&str, &Identity> = config
        .identities
        .iter()
        .map(|i| (i.name.as_str(), i))
        .collect();
    let (expect_tables, default_tables) = tables.split_at(config.expect.len());
    let mut report = Report::default();
    for (block, table) in config.expect.iter().zip(expect_tables) {
        let (Some(table), Some(identity)) = (table, identities.get(block.identity.as_str())) else {
            continue;
        };
        let cell = Cell {
            table,
            name: catalog.display_name(table),
            identity,
            path: &source.path,
        };
        cell.check(&mut session, &block.ops, &mut report)?;
    }
    for (block, table) in config.defaults.iter().zip(default_tables) {
        let name = table_name(block, *table, &catalog);
        if let Some(case) = &block.ops.select {
            let description = select::describe(&case.select);
            report.push(
                &name,
                &block.identity,
                Op::Select,
                description,
                Outcome::Unsupported,
            );
        }
        unsupported_writes(&name, block, &mut report);
    }
    session.rollback()?;
    Ok(report)
}

struct Cell<'a> {
    table: &'a Table,
    name: String,
    identity: &'a Identity,
    path: &'a Path,
}

impl Cell<'_> {
    fn at(&self, span: Span) -> Location<'_> {
        Location(self.path, span)
    }

    fn push(&self, report: &mut Report, op: Op, description: String, outcome: Outcome) {
        report.push(&self.name, &self.identity.name, op, description, outcome);
    }

    fn check(&self, session: &mut Session, ops: &Ops, report: &mut Report) -> Result<(), RunError> {
        let (table, identity) = (self.table, self.identity);
        if let Some(case) = &ops.select {
            let at = self.at(case.span);
            let outcome =
                session.case(|tx| select::check(tx, table, identity, &case.select, at))??;
            self.push(report, Op::Select, select::describe(&case.select), outcome);
        }
        match &ops.insert {
            // `insert: allow` is rejected when the config is loaded.
            Some(Writes::Shorthand { expect, .. }) => {
                let outcome =
                    session.case(|tx| insert_deny::check(tx, table, &self.name, identity))??;
                let description = write::expectation(*expect).to_owned();
                self.push(report, Op::Insert, description, outcome);
            }
            Some(Writes::Cases(cases)) => {
                for case in cases {
                    let at = self.at(case.span);
                    let outcome =
                        session.case(|tx| write::insert(tx, table, identity, case, at))??;
                    self.push(report, Op::Insert, write::describe_insert(case), outcome);
                }
            }
            None => {}
        }
        match &ops.update {
            Some(Writes::Shorthand { expect, span }) => {
                let at = self.at(*span);
                let outcome = session.case(|tx| {
                    write::modify(tx, table, identity, Modify::Update(None), None, *expect, at)
                })??;
                let description = write::expectation(*expect).to_owned();
                self.push(report, Op::Update, description, outcome);
            }
            Some(Writes::Cases(cases)) => {
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
                    self.push(report, Op::Update, description, outcome);
                }
            }
            None => {}
        }
        match &ops.delete {
            Some(Writes::Shorthand { expect, span }) => {
                let at = self.at(*span);
                let outcome = session.case(|tx| {
                    write::modify(tx, table, identity, Modify::Delete, None, *expect, at)
                })??;
                let description = write::expectation(*expect).to_owned();
                self.push(report, Op::Delete, description, outcome);
            }
            Some(Writes::Cases(cases)) => {
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
                    self.push(report, Op::Delete, description, outcome);
                }
            }
            None => {}
        }
        Ok(())
    }
}

/// Every column in `values` and `set` must exist and be writable (not generated).
fn check_columns(ops: &Ops, table: &Table, catalog: &Catalog, diagnostics: &mut Vec<Diagnostic>) {
    let inserts = match &ops.insert {
        Some(Writes::Cases(cases)) => cases.iter().map(|c| c.values.as_slice()).collect(),
        _ => Vec::new(),
    };
    let updates = match &ops.update {
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

fn table_name(block: &Block, table: Option<&Table>, catalog: &Catalog) -> String {
    match (table, &block.table) {
        (Some(table), _) => catalog.display_name(table),
        (None, TableRef::Named(name)) => name.clone(),
        (None, TableRef::All) => "*".to_owned(),
    }
}

fn preflight(
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

fn unsupported_writes(table: &str, block: &Block, report: &mut Report) {
    let Ops {
        insert,
        update,
        delete,
        ..
    } = &block.ops;
    let mut push = |op, description| {
        report.push(
            table,
            &block.identity,
            op,
            description,
            Outcome::Unsupported,
        );
    };
    match insert {
        Some(Writes::Shorthand { expect, .. }) => push(Op::Insert, expectation(*expect).into()),
        Some(Writes::Cases(cases)) => {
            for case in cases {
                let columns: Vec<&str> = case.values.iter().map(|a| a.column.as_str()).collect();
                push(
                    Op::Insert,
                    format!(
                        "values ({}) → {}",
                        columns.join(", "),
                        expectation(case.expect)
                    ),
                );
            }
        }
        None => {}
    }
    match update {
        Some(Writes::Shorthand { expect, .. }) => push(Op::Update, expectation(*expect).into()),
        Some(Writes::Cases(cases)) => {
            for case in cases {
                push(
                    Op::Update,
                    format!("where {} → {}", case.predicate, expectation(case.expect)),
                );
            }
        }
        None => {}
    }
    match delete {
        Some(Writes::Shorthand { expect, .. }) => push(Op::Delete, expectation(*expect).into()),
        Some(Writes::Cases(cases)) => {
            for case in cases {
                push(
                    Op::Delete,
                    format!("where {} → {}", case.predicate, expectation(case.expect)),
                );
            }
        }
        None => {}
    }
}

fn expectation(expect: Expectation) -> &'static str {
    match expect {
        Expectation::Allow => "allow",
        Expectation::Deny => "deny",
    }
}
