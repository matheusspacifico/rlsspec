//! `rows` cases for update and delete: the statement runs over the whole table, and the rows it
//! affected are compared with the rows the predicate matches.
//!
//! The statement has no `WHERE` and no `RETURNING` and assigns only bound constants, so it reads
//! no column and Postgres applies the UPDATE or DELETE policies alone: a statement that reads a
//! column also needs the SELECT policies to pass. Which rows it touched is read as the admin,
//! before and after: an updated row gets a new `ctid`, a deleted row has none, so the affected
//! rows are the `(tableoid, ctid)` pairs that are gone.

use std::collections::HashSet;

use postgres::Transaction;

use super::rows::{self, Row, RowKey, samples, the_rows};
use super::write::{self, Modify};
use super::{Location, Outcome, RunError, VACUOUS, empty_table, no_match};
use crate::catalog::Table;
use crate::config::{Assignment, Identity};
use crate::identity;
use crate::pg;

pub fn describe(predicate: &str, set: Option<&[Assignment]>, subset: bool) -> String {
    let mut text = match set {
        Some(set) => format!(
            "set {} · rows: {predicate}",
            write::describe_assignments(set)
        ),
        None => format!("rows: {predicate}"),
    };
    if subset {
        text.push_str(" (subset)");
    }
    text
}

/// The words of a verdict, per operation.
struct Verb {
    /// `update`, as in "should update".
    plain: &'static str,
    /// `updated`.
    past: &'static str,
}

pub fn check(
    tx: &mut Transaction,
    table: &Table,
    identity: &Identity,
    kind: Modify,
    predicate: &str,
    subset: bool,
    at: Location,
) -> Result<Outcome, RunError> {
    // Newlines so a trailing `--` comment in the predicate can't swallow the rest.
    let expected = format!("(\n{predicate}\n) IS TRUE");
    let before = match rows::admin_rows(tx, table, &expected) {
        Ok(rows) => rows,
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
    let should: Vec<&Row> = before.iter().filter(|r| r.expected).collect();
    let should_not = before.len() - should.len();
    if before.is_empty() {
        return Ok(Outcome::Inconclusive(empty_table(table, at, false)));
    }
    if should.is_empty() {
        return Ok(Outcome::Inconclusive(no_match(at)));
    }
    if subset && should_not == 0 {
        // No row is left that could be wrongly touched.
        return Ok(Outcome::Inconclusive(format!(
            "{VACUOUS}the predicate matches every row, so with `subset` this check cannot fail ({at})"
        )));
    }

    let verb = match kind {
        Modify::Update(_) => Verb {
            plain: "update",
            past: "updated",
        },
        Modify::Delete => Verb {
            plain: "delete",
            past: "deleted",
        },
    };
    let (sql, params) = match kind {
        Modify::Update(set) => {
            // A `rows` case without `set` is rejected when the config is loaded.
            let (columns, casts, params) = write::typed(table, set.unwrap_or_default());
            let assignments: Vec<String> = columns
                .iter()
                .zip(&casts)
                .map(|(column, cast)| format!("{column} = {cast}"))
                .collect();
            let sql = format!("UPDATE {} SET {}", table.sql_name(), assignments.join(", "));
            (sql, params)
        }
        Modify::Delete => (format!("DELETE FROM {}", table.sql_name()), Vec::new()),
    };

    pg::set_local(tx, "row_security", "on")?;
    identity::apply(tx, identity)?;
    let affected = match tx.execute(&sql, &write::as_params(&params)) {
        Ok(affected) => affected,
        // A permission error about the target is one way of the affected set being empty: the
        // whole statement failed, so no row was touched (§4.3).
        Err(err) if write::denied_on(&err, table) => {
            return Ok(if subset {
                Outcome::Pass("permission denied".into())
            } else {
                Outcome::Fail(format!(
                    "did not {} {} of {} it should {} ({})",
                    verb.plain,
                    should.len(),
                    the_rows(should.len()),
                    verb.plain,
                    pg::describe(&err)
                ))
            });
        }
        Err(err) => return Ok(write::failed(verb.plain, identity, table, &err, at)),
    };

    // Back to the admin, who sees every row: the identity's role stays set until the savepoint ends.
    tx.execute("RESET ROLE", &[])?;
    pg::set_local(tx, "row_security", "off")?;
    let after: HashSet<RowKey> = rows::keys(tx, table)?;
    let gone: Vec<&Row> = before.iter().filter(|r| !after.contains(&r.key)).collect();

    // The statement's own rows all lost their `ctid`, and there are `affected` of them. When more
    // rows are gone, something else changed rows of this table inside the statement, and the two
    // can't be told apart: never a pass, never a guessed failure.
    if u64::try_from(gone.len()).ok() != Some(affected) {
        return Ok(Outcome::Inconclusive(format!(
            "the statement affected {affected} {}, but {} {} of {} changed: a trigger or a cascade also changed rows of this table, so the affected rows can't be identified; use `where` cases here ({at})",
            plural(usize::try_from(affected).unwrap_or(usize::MAX)),
            gone.len(),
            plural(gone.len()),
            table.name
        )));
    }

    let wrongly: Vec<&Row> = gone.iter().copied().filter(|r| !r.expected).collect();
    let untouched: Vec<&Row> = should
        .iter()
        .copied()
        .filter(|r| after.contains(&r.key))
        .collect();
    let touched = should.len() - untouched.len();

    let mut problems = Vec::new();
    if !wrongly.is_empty() {
        problems.push(format!(
            "{} {} of {} it should not {}: {}",
            verb.past,
            wrongly.len(),
            the_rows(should_not),
            verb.plain,
            samples(&wrongly)
        ));
    }
    if !untouched.is_empty() && !subset {
        problems.push(format!(
            "did not {} {} of {} it should {}: {}",
            verb.plain,
            untouched.len(),
            the_rows(should.len()),
            verb.plain,
            samples(&untouched)
        ));
    }
    if !problems.is_empty() {
        return Ok(Outcome::Fail(problems.join(" · ")));
    }
    Ok(Outcome::Pass(if subset {
        format!("{} {touched} of {} rows", verb.past, should.len())
    } else {
        format!("{} {touched} {}", verb.past, plural(touched))
    }))
}

fn plural(count: usize) -> &'static str {
    if count == 1 { "row" } else { "rows" }
}
