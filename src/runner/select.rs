use postgres::Transaction;

use super::rows::{self, Row, samples, the_rows};
use super::{Location, Outcome, RunError, empty_table, no_match};
use crate::catalog::Table;
use crate::config::{Identity, Select};
use crate::identity;
use crate::pg;

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

pub fn check(
    tx: &mut Transaction,
    table: &Table,
    identity: &Identity,
    select: &Select,
    at: Location,
) -> Result<Outcome, RunError> {
    let expected = match select {
        Select::Deny => "false".to_owned(),
        Select::All => "true".to_owned(),
        // Newlines so a trailing `--` comment in the predicate can't swallow the rest.
        Select::Rows { predicate, .. } => format!("(\n{predicate}\n) IS TRUE"),
    };
    let rows = match rows::admin_rows(tx, table, &expected) {
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
            return Ok(Outcome::Inconclusive(empty_table(table, at, None)));
        }
        Select::Rows { .. } if expected == 0 => {
            return Ok(Outcome::Inconclusive(no_match(at)));
        }
        _ => {}
    }

    pg::set_local(tx, "row_security", "on")?;
    identity::apply(tx, identity)?;
    let (visible, denied) = match rows::keys(tx, table) {
        Ok(found) => (found, false),
        Err(err) if pg::is_denied(&err) => (Default::default(), true),
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
        .filter(|r| !r.expected && visible.contains(&r.key))
        .collect();
    let hidden: Vec<&Row> = rows
        .iter()
        .filter(|r| r.expected && !visible.contains(&r.key))
        .collect();
    let subset = matches!(select, Select::Rows { subset: true, .. });

    let mut problems = Vec::new();
    if !leaked.is_empty() {
        problems.push(format!(
            "leaked {} of {} it should not see: {}",
            leaked.len(),
            the_rows(rows.len() - expected),
            samples(&leaked)
        ));
    }
    if !hidden.is_empty() && !subset {
        let mut text = format!(
            "hidden {} of {} it should see: {}",
            hidden.len(),
            the_rows(expected),
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
