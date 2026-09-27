use std::collections::HashSet;

use postgres::Transaction;

use super::{Location, Outcome, RunError};
use crate::catalog::Table;
use crate::config::{Identity, Select};
use crate::identity;
use crate::pg;

const SAMPLES: usize = 5;

pub fn describe(select: &Select) -> String {
    match select {
        Select::Deny => "deny".into(),
        Select::All => "all".into(),
        Select::Rows {
            predicate,
            subset: false,
        } => format!("rows: {predicate}"),
        Select::Rows {
            predicate,
            subset: true,
        } => format!("rows: {predicate} (subset)"),
    }
}

struct Row {
    ctid: String,
    label: String,
    expected: bool,
}

pub fn check(
    tx: &mut Transaction,
    table: &Table,
    identity: &Identity,
    select: &Select,
    at: Location,
) -> Result<Outcome, RunError> {
    let rows = match admin_rows(tx, table, select) {
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
    let expected = rows.iter().filter(|r| r.expected).count();
    match select {
        Select::Deny | Select::All if rows.is_empty() => {
            return Ok(Outcome::Inconclusive(format!(
                "vacuous: {} has no rows, so this check cannot fail; add rows in setup ({at})",
                table.sql_name()
            )));
        }
        Select::Rows { .. } if expected == 0 => {
            return Ok(Outcome::Inconclusive(format!(
                "vacuous: the predicate matches no rows, so this check cannot fail ({at})"
            )));
        }
        _ => {}
    }

    pg::set_local(tx, "row_security", "on")?;
    identity::apply(tx, identity)?;
    let query = format!("SELECT ctid::text FROM {}", table.sql_name());
    let (visible, denied): (HashSet<String>, bool) = match tx.query(&query, &[]) {
        Ok(found) => (found.iter().map(|r| r.get(0)).collect(), false),
        Err(err) if pg::is_denied(&err) => (HashSet::new(), true),
        Err(err) => {
            return Ok(Outcome::Inconclusive(format!(
                "query as `{}` failed: {}",
                identity.name,
                pg::describe(&err)
            )));
        }
    };

    let leaked: Vec<&Row> = rows
        .iter()
        .filter(|r| !r.expected && visible.contains(&r.ctid))
        .collect();
    let hidden: Vec<&Row> = rows
        .iter()
        .filter(|r| r.expected && !visible.contains(&r.ctid))
        .collect();
    let subset = matches!(select, Select::Rows { subset: true, .. });

    let mut problems = Vec::new();
    if !leaked.is_empty() {
        problems.push(format!(
            "leaked {} of {} rows: {}",
            leaked.len(),
            rows.len() - expected,
            samples(&leaked)
        ));
    }
    if !hidden.is_empty() && !subset {
        let mut text = format!(
            "hidden {} of {expected} rows: {}",
            hidden.len(),
            samples(&hidden)
        );
        if denied {
            text.push_str(" (permission denied)");
        }
        problems.push(text);
    }
    if !problems.is_empty() {
        return Ok(Outcome::Fail(problems.join(" · ")));
    }

    let seen = if denied {
        "permission denied".to_owned()
    } else if subset {
        format!("{} of {expected} visible", visible.len())
    } else {
        format!("{} visible", visible.len())
    };
    Ok(Outcome::Pass(seen))
}

fn admin_rows(
    tx: &mut Transaction,
    table: &Table,
    select: &Select,
) -> Result<Vec<Row>, postgres::Error> {
    let expected = match select {
        Select::Deny => "false".to_owned(),
        Select::All => "true".to_owned(),
        // Newlines so a trailing `--` comment in the predicate can't swallow the rest.
        Select::Rows { predicate, .. } => format!("(\n{predicate}\n) IS TRUE"),
    };
    let key: Vec<String> = table
        .primary_key
        .iter()
        .map(|c| format!("{}::text", pg::quote_ident(c)))
        .collect();
    let (key_array, order) = if key.is_empty() {
        ("ARRAY[]::text[]".to_owned(), "ctid".to_owned())
    } else {
        let columns: Vec<String> = table
            .primary_key
            .iter()
            .map(|c| pg::quote_ident(c))
            .collect();
        (format!("ARRAY[{}]", key.join(", ")), columns.join(", "))
    };
    let query = format!(
        "SELECT ctid::text, {key_array}, {expected} FROM {} ORDER BY {order}",
        table.sql_name()
    );
    pg::set_local(tx, "row_security", "off")?;
    let rows = tx.query(&query, &[])?;
    Ok(rows
        .iter()
        .map(|row| {
            let ctid: String = row.get(0);
            let values: Vec<Option<String>> = row.get(1);
            Row {
                label: label(&table.primary_key, &values, &ctid),
                ctid,
                expected: row.get(2),
            }
        })
        .collect())
}

fn label(columns: &[String], values: &[Option<String>], ctid: &str) -> String {
    let value = |v: &Option<String>| v.clone().unwrap_or_else(|| "NULL".into());
    match (columns, values) {
        ([], _) => format!("ctid={ctid}"),
        ([column], [v]) => format!("{column}={}", value(v)),
        _ => format!(
            "({})=({})",
            columns.join(", "),
            values.iter().map(value).collect::<Vec<_>>().join(", ")
        ),
    }
}

fn samples(rows: &[&Row]) -> String {
    let mut text = rows
        .iter()
        .take(SAMPLES)
        .map(|r| r.label.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    if rows.len() > SAMPLES {
        text.push_str(", …");
    }
    text
}
