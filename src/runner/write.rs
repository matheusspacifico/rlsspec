use postgres::Transaction;
use postgres::types::ToSql;

use super::{Location, Outcome, RunError};
use crate::catalog::{self, Table};
use crate::config::vars::quote_literal;
use crate::config::{Assignment, Expectation, Identity, InsertCase};
use crate::identity;
use crate::pg;

pub enum Modify<'a> {
    Update(Option<&'a [Assignment]>),
    Delete,
}

impl Modify<'_> {
    fn verb(&self) -> &'static str {
        match self {
            Modify::Update(_) => "update",
            Modify::Delete => "delete",
        }
    }
}

pub fn expectation(expect: Expectation) -> &'static str {
    match expect {
        Expectation::Allow => "allow",
        Expectation::Deny => "deny",
    }
}

pub fn describe_insert(case: &InsertCase) -> String {
    format!(
        "values ({}) → {}",
        describe_assignments(&case.values),
        expectation(case.expect)
    )
}

pub fn describe_modify(predicate: &str, set: Option<&[Assignment]>, expect: Expectation) -> String {
    match set {
        Some(set) => format!(
            "where {predicate} set {} → {}",
            describe_assignments(set),
            expectation(expect)
        ),
        None => format!("where {predicate} → {}", expectation(expect)),
    }
}

fn describe_assignments(assignments: &[Assignment]) -> String {
    assignments
        .iter()
        .map(|a| {
            let value = match a.value.as_deref() {
                None => "NULL".to_owned(),
                Some(v) if v.parse::<f64>().is_ok() || v == "true" || v == "false" => v.to_owned(),
                Some(v) => quote_literal(v),
            };
            format!("{} = {value}", a.column)
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// Column list, `$n::text::<type>` casts and the text parameters for `assignments`.
/// Columns were checked against the catalog before any case ran.
fn typed(
    table: &Table,
    assignments: &[Assignment],
) -> (Vec<String>, Vec<String>, Vec<Option<String>>) {
    let mut columns = Vec::new();
    let mut casts = Vec::new();
    let mut params = Vec::new();
    for (i, assignment) in assignments.iter().enumerate() {
        let sql_type = table
            .column(&assignment.column)
            .map_or("text", |c| c.sql_type.as_str());
        columns.push(pg::quote_ident(&assignment.column));
        casts.push(format!("${}::text::{sql_type}", i + 1));
        params.push(assignment.value.clone());
    }
    (columns, casts, params)
}

fn as_params(values: &[Option<String>]) -> Vec<&(dyn ToSql + Sync)> {
    values.iter().map(|v| v as &(dyn ToSql + Sync)).collect()
}

pub fn insert(
    tx: &mut Transaction,
    table: &Table,
    identity: &Identity,
    case: &InsertCase,
    at: Location,
) -> Result<Outcome, RunError> {
    let (columns, casts, params) = typed(table, &case.values);
    // No RETURNING: it would also need a SELECT policy on the new row and change the verdict.
    let sql = format!(
        "INSERT INTO {} ({}) VALUES ({})",
        table.sql_name(),
        columns.join(", "),
        casts.join(", ")
    );
    pg::set_local(tx, "row_security", "on")?;
    identity::apply(tx, identity)?;
    let outcome = match (tx.execute(&sql, &as_params(&params)), case.expect) {
        (Ok(0), _) => Outcome::Inconclusive(format!(
            "insert as `{}` affected no row (a trigger or rule skipped it) ({at})",
            identity.name
        )),
        (Ok(_), Expectation::Allow) => Outcome::Pass("inserted".into()),
        (Ok(_), Expectation::Deny) => {
            Outcome::Fail("inserted (expected a permission error)".into())
        }
        (Err(err), Expectation::Deny) if pg::is_denied(&err) => {
            Outcome::Pass("permission denied".into())
        }
        (Err(err), Expectation::Allow) if pg::is_denied(&err) => {
            Outcome::Fail(format!("denied: {}", pg::describe(&err)))
        }
        (Err(err), _) => Outcome::Inconclusive(format!(
            "insert as `{}` failed: {} ({at})",
            identity.name,
            pg::describe(&err)
        )),
    };
    Ok(outcome)
}

/// `predicate` is `None` for the shorthand, which targets every row of the table.
pub fn modify(
    tx: &mut Transaction,
    table: &Table,
    identity: &Identity,
    kind: Modify,
    predicate: Option<&str>,
    expect: Expectation,
    at: Location,
) -> Result<Outcome, RunError> {
    // Newlines so a trailing `--` comment in the predicate can't swallow the rest.
    let filter = format!("(\n{}\n)", predicate.unwrap_or("true"));
    pg::set_local(tx, "row_security", "off")?;
    let count_sql = format!("SELECT count(*) FROM {} WHERE {filter}", table.sql_name());
    let expected: i64 = match tx.query_one(&count_sql, &[]) {
        Ok(row) => row.get(0),
        Err(err) if pg::is_denied(&err) => {
            return Err(RunError::AdminFiltered {
                table: table.sql_name(),
                detail: pg::describe(&err),
            });
        }
        Err(err) => {
            return Ok(Outcome::Inconclusive(format!(
                "query as admin failed: {} ({at})",
                pg::describe(&err)
            )));
        }
    };
    if expected == 0 {
        return Ok(Outcome::Inconclusive(match predicate {
            Some(_) => {
                format!("vacuous: the predicate matches no rows, so this check cannot fail ({at})")
            }
            None => format!(
                "vacuous: {} has no rows, so this check cannot fail; add rows in setup ({at})",
                table.sql_name()
            ),
        }));
    }

    let verb = kind.verb();
    let (sql, params) = match kind {
        Modify::Update(Some(set)) => {
            let (columns, casts, params) = typed(table, set);
            let assignments: Vec<String> = columns
                .iter()
                .zip(&casts)
                .map(|(column, cast)| format!("{column} = {cast}"))
                .collect();
            let sql = format!(
                "UPDATE {} SET {} WHERE {filter}",
                table.sql_name(),
                assignments.join(", ")
            );
            (sql, params)
        }
        Modify::Update(None) => {
            let Some(column) = catalog::first_updatable_column(tx, table, &identity.role)? else {
                return Ok(match expect {
                    Expectation::Deny => Outcome::Pass("denied: no UPDATE privilege".into()),
                    Expectation::Allow => Outcome::Fail("no UPDATE privilege".into()),
                });
            };
            let column = pg::quote_ident(&column);
            let sql = format!(
                "UPDATE {} SET {column} = {column} WHERE {filter}",
                table.sql_name()
            );
            (sql, Vec::new())
        }
        Modify::Delete => (
            format!("DELETE FROM {} WHERE {filter}", table.sql_name()),
            Vec::new(),
        ),
    };

    pg::set_local(tx, "row_security", "on")?;
    identity::apply(tx, identity)?;
    let expected = u64::try_from(expected).unwrap_or(0);
    let outcome = match (tx.execute(&sql, &as_params(&params)), expect) {
        (Ok(n), Expectation::Allow) if n == expected => {
            Outcome::Pass(format!("affected {n} of {expected} rows"))
        }
        (Ok(n), Expectation::Allow) => Outcome::Fail(format!(
            "affected {n} of {expected} rows (expected {expected})"
        )),
        (Ok(0), Expectation::Deny) => Outcome::Pass(format!("affected 0 of {expected} rows")),
        (Ok(n), Expectation::Deny) => {
            Outcome::Fail(format!("affected {n} of {expected} rows (expected 0)"))
        }
        (Err(err), Expectation::Deny) if pg::is_denied(&err) => {
            Outcome::Pass("permission denied".into())
        }
        // A WITH CHECK violation on an allow case is a real finding, not an inconclusive run.
        (Err(err), Expectation::Allow) if pg::is_denied(&err) => Outcome::Fail(format!(
            "denied: {} (expected {expected} of {expected} rows)",
            pg::describe(&err)
        )),
        (Err(err), _) => Outcome::Inconclusive(format!(
            "{verb} as `{}` failed: {} ({at})",
            identity.name,
            pg::describe(&err)
        )),
    };
    Ok(outcome)
}
