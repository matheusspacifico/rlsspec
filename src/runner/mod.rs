mod select;

use std::collections::HashMap;
use std::path::Path;

use crate::catalog::{self, Catalog, Table};
use crate::config::{
    Block, Config, ConfigError, Diagnostic, Expectation, Identity, Ops, Source, Span, TableRef,
    Writes,
};
use crate::identity;
use crate::pg::{self, PgError, Session};

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
    fn push(&mut self, table: &str, block: &Block, op: Op, description: String, outcome: Outcome) {
        self.results.push(CaseResult {
            table: table.to_owned(),
            identity: block.identity.clone(),
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
        let name = table_name(block, *table, &catalog);
        if let (Some(case), Some(table)) = (&block.ops.select, table) {
            let outcome = match identities.get(block.identity.as_str()) {
                Some(identity) => {
                    let at = Location(&source.path, case.span);
                    session.case(|tx| select::check(tx, table, identity, &case.select, at))??
                }
                None => Outcome::Inconclusive(format!("unknown identity `{}`", block.identity)),
            };
            let description = select::describe(&case.select);
            report.push(&name, block, Op::Select, description, outcome);
        }
        unsupported_writes(&name, block, &mut report);
    }
    for (block, table) in config.defaults.iter().zip(default_tables) {
        let name = table_name(block, *table, &catalog);
        if let Some(case) = &block.ops.select {
            let description = select::describe(&case.select);
            report.push(&name, block, Op::Select, description, Outcome::Unsupported);
        }
        unsupported_writes(&name, block, &mut report);
    }
    session.rollback()?;
    Ok(report)
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
        report.push(table, block, op, description, Outcome::Unsupported);
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
