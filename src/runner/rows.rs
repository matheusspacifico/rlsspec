use std::collections::HashSet;

use postgres::Transaction;

use crate::catalog::Table;
use crate::pg;

const SAMPLES: usize = 5;

/// What identifies a row for the length of a case: `(tableoid, ctid)`. A `ctid` alone repeats
/// across the partitions of a partitioned table.
pub type RowKey = (u32, String);

/// A row as the admin sees it, with `row_security = off`.
pub struct Row {
    pub key: RowKey,
    /// Its primary key, or its `ctid` (and partition) when the table has none.
    pub label: String,
    /// Whether the case's predicate holds for it.
    pub expected: bool,
}

/// Every row of `table`, flagged with `expected`, a SQL boolean expression over its columns.
/// Sets `row_security = off`: the caller turns it back on before acting as an identity.
pub fn admin_rows(
    tx: &mut Transaction,
    table: &Table,
    expected: &str,
) -> Result<Vec<Row>, postgres::Error> {
    let key: Vec<String> = table
        .primary_key
        .iter()
        .map(|c| format!("{}::text", pg::quote_ident(c)))
        .collect();
    let (key_array, order) = if key.is_empty() {
        // Qualified: a bare `tableoid` would be read as one of the output columns.
        let name = table.sql_name();
        (
            "ARRAY[]::text[]".to_owned(),
            format!("{name}.tableoid, {name}.ctid"),
        )
    } else {
        let columns: Vec<String> = table
            .primary_key
            .iter()
            .map(|c| pg::quote_ident(c))
            .collect();
        (format!("ARRAY[{}]", key.join(", ")), columns.join(", "))
    };
    let query = format!(
        "SELECT tableoid, ctid::text, tableoid::regclass::text, {key_array}, {expected} FROM {} ORDER BY {order}",
        table.sql_name()
    );
    pg::set_local(tx, "row_security", "off")?;
    let rows = tx.query(&query, &[])?;
    Ok(rows
        .iter()
        .map(|row| {
            let tableoid: u32 = row.get(0);
            let ctid: String = row.get(1);
            let partition: String = row.get(2);
            let values: Vec<Option<String>> = row.get(3);
            // A row of a partition lives in another relation than the table the spec names.
            let place = if tableoid == table.oid {
                format!("ctid={ctid}")
            } else {
                format!("ctid={ctid} in {partition}")
            };
            Row {
                label: label(&table.primary_key, &values, &place),
                key: (tableoid, ctid),
                expected: row.get(4),
            }
        })
        .collect())
}

/// The `(tableoid, ctid)` of every row `table` holds for the current role and `row_security`.
pub fn keys(tx: &mut Transaction, table: &Table) -> Result<HashSet<RowKey>, postgres::Error> {
    let query = format!("SELECT tableoid, ctid::text FROM {}", table.sql_name());
    let rows = tx.query(&query, &[])?;
    Ok(rows.iter().map(|row| (row.get(0), row.get(1))).collect())
}

fn label(columns: &[String], values: &[Option<String>], place: &str) -> String {
    let value = |v: &Option<String>| v.clone().unwrap_or_else(|| "NULL".into());
    match (columns, values) {
        ([], _) => place.to_owned(),
        ([column], [v]) => format!("{column}={}", value(v)),
        _ => format!(
            "({})=({})",
            columns.join(", "),
            values.iter().map(value).collect::<Vec<_>>().join(", ")
        ),
    }
}

/// `the 3 rows`: the whole set a count is out of.
pub fn the_rows(count: usize) -> String {
    let unit = if count == 1 { "row" } else { "rows" };
    format!("the {count} {unit}")
}

pub fn samples(rows: &[&Row]) -> String {
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
