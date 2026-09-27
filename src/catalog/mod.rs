use postgres::Transaction;

use crate::pg;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Table {
    pub oid: u32,
    pub schema: String,
    pub name: String,
    pub primary_key: Vec<String>,
    pub columns: Vec<Column>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Column {
    pub name: String,
    /// `format_type(atttypid, atttypmod)`: valid SQL for a cast target, e.g. `numeric(10,2)`.
    pub sql_type: String,
    pub generated: bool,
}

impl Table {
    pub fn sql_name(&self) -> String {
        pg::qualified(&self.schema, &self.name)
    }

    pub fn column(&self, name: &str) -> Option<&Column> {
        self.columns.iter().find(|c| c.name == name)
    }
}

/// What the static `insert: deny` check needs to know about one role and one table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InsertAccess {
    pub privilege: bool,
    pub bypasses_rls: bool,
    pub rls_enabled: bool,
    pub rls_forced: bool,
    pub owner: bool,
    pub permissive_policies: Vec<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Catalog {
    schemas: Vec<String>,
    tables: Vec<Table>,
}

const TABLES: &str = "
SELECT c.oid, n.nspname::text, c.relname::text,
       coalesce((SELECT array_agg(a.attname::text ORDER BY k.ord)
                 FROM pg_index i
                 CROSS JOIN LATERAL unnest(i.indkey) WITH ORDINALITY AS k(attnum, ord)
                 JOIN pg_attribute a ON a.attrelid = i.indrelid AND a.attnum = k.attnum
                 WHERE i.indrelid = c.oid AND i.indisprimary), '{}')
FROM pg_class c
JOIN pg_namespace n ON n.oid = c.relnamespace
WHERE c.relkind IN ('r', 'p') AND n.nspname = ANY($1)
ORDER BY 1, 2";

const COLUMNS: &str = "
SELECT a.attrelid, a.attname::text, format_type(a.atttypid, a.atttypmod), a.attgenerated <> ''
FROM pg_attribute a
WHERE a.attrelid = ANY($1) AND a.attnum > 0 AND NOT a.attisdropped
ORDER BY a.attrelid, a.attnum";

// A policy applies to `role` when it targets PUBLIC (oid 0) or a role whose privileges `role` has.
const INSERT_ACCESS: &str = "
SELECT has_table_privilege($2::name, c.oid, 'INSERT') OR has_any_column_privilege($2::name, c.oid, 'INSERT'),
       r.rolsuper OR r.rolbypassrls,
       c.relrowsecurity,
       c.relforcerowsecurity,
       pg_has_role($2::name, c.relowner, 'USAGE'),
       coalesce((SELECT array_agg(p.polname::text ORDER BY p.polname)
                 FROM pg_policy p
                 WHERE p.polrelid = c.oid AND p.polpermissive AND p.polcmd IN ('a', '*')
                   AND (0 = ANY(p.polroles)
                        OR EXISTS (SELECT 1 FROM unnest(p.polroles) AS pr(oid)
                                   WHERE pr.oid <> 0 AND pg_has_role($2::name, pr.oid, 'USAGE')))),
                '{}')
FROM pg_class c, pg_roles r
WHERE c.oid = $1 AND r.rolname = $2::name";

const FIRST_UPDATABLE_COLUMN: &str = "
SELECT a.attname::text
FROM pg_attribute a
WHERE a.attrelid = $1 AND a.attnum > 0 AND NOT a.attisdropped AND a.attgenerated = ''
  AND has_column_privilege($2::name, a.attrelid, a.attnum, 'UPDATE')
ORDER BY a.attnum
LIMIT 1";

pub fn load(tx: &mut Transaction, schemas: &[String]) -> Result<Catalog, postgres::Error> {
    let mut tables: Vec<Table> = tx
        .query(TABLES, &[&schemas])?
        .into_iter()
        .map(|row| Table {
            oid: row.get(0),
            schema: row.get(1),
            name: row.get(2),
            primary_key: row.get(3),
            columns: Vec::new(),
        })
        .collect();
    let oids: Vec<u32> = tables.iter().map(|t| t.oid).collect();
    for row in tx.query(COLUMNS, &[&oids])? {
        let oid: u32 = row.get(0);
        if let Some(table) = tables.iter_mut().find(|t| t.oid == oid) {
            table.columns.push(Column {
                name: row.get(1),
                sql_type: row.get(2),
                generated: row.get(3),
            });
        }
    }
    Ok(Catalog {
        schemas: schemas.to_vec(),
        tables,
    })
}

pub fn insert_access(
    tx: &mut Transaction,
    table: &Table,
    role: &str,
) -> Result<InsertAccess, postgres::Error> {
    let row = tx.query_one(INSERT_ACCESS, &[&table.oid, &role])?;
    Ok(InsertAccess {
        privilege: row.get(0),
        bypasses_rls: row.get(1),
        rls_enabled: row.get(2),
        rls_forced: row.get(3),
        owner: row.get(4),
        permissive_policies: row.get(5),
    })
}

/// The first column, in `attnum` order, that `role` may update and that isn't generated.
pub fn first_updatable_column(
    tx: &mut Transaction,
    table: &Table,
    role: &str,
) -> Result<Option<String>, postgres::Error> {
    let row = tx.query_opt(FIRST_UPDATABLE_COLUMN, &[&table.oid, &role])?;
    Ok(row.map(|row| row.get(0)))
}

impl Catalog {
    #[cfg(test)]
    pub(crate) fn new(schemas: &[&str], tables: &[(&str, &str)]) -> Self {
        Self {
            schemas: schemas.iter().map(|s| s.to_string()).collect(),
            tables: tables
                .iter()
                .zip(1..)
                .map(|((schema, name), oid)| Table {
                    oid,
                    schema: schema.to_string(),
                    name: name.to_string(),
                    primary_key: Vec::new(),
                    columns: Vec::new(),
                })
                .collect(),
        }
    }

    pub fn tables(&self) -> &[Table] {
        &self.tables
    }

    /// The shortest name that resolves back to `table`.
    pub fn display_name(&self, table: &Table) -> String {
        match self.resolve(&table.name) {
            Ok(found) if found == table => table.name.clone(),
            _ => format!("{}.{}", table.schema, table.name),
        }
    }

    /// Resolves `table` or `schema.table` against the schemas in scope.
    pub fn resolve(&self, name: &str) -> Result<&Table, String> {
        let find = |schema: &str, table: &str| {
            self.tables
                .iter()
                .find(|t| t.schema == schema && t.name == table)
        };
        if let Some((schema, table)) = name.split_once('.') {
            if !self.schemas.iter().any(|s| s == schema) {
                return Err(format!(
                    "schema `{schema}` is not listed in `database.schemas`"
                ));
            }
            return find(schema, table).ok_or_else(|| format!("table `{name}` not found"));
        }
        let hits: Vec<&Table> = self
            .schemas
            .iter()
            .filter_map(|schema| find(schema, name))
            .collect();
        match hits.as_slice() {
            [table] => Ok(table),
            [] => Err(format!(
                "table `{name}` not found in {} {}",
                if self.schemas.len() == 1 {
                    "schema"
                } else {
                    "schemas"
                },
                self.schemas.join(", ")
            )),
            [first, ..] => Err(format!(
                "table `{name}` is ambiguous: it exists in schemas {}; write `{}.{name}`",
                hits.iter()
                    .map(|t| t.schema.as_str())
                    .collect::<Vec<_>>()
                    .join(", "),
                first.schema
            )),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolves_bare_and_qualified_names() {
        let catalog = Catalog::new(&["public", "app"], &[("app", "notes"), ("public", "tags")]);
        assert_eq!(catalog.resolve("notes").unwrap().schema, "app");
        assert_eq!(catalog.resolve("app.notes").unwrap().name, "notes");
        assert_eq!(catalog.resolve("tags").unwrap().schema, "public");
        let notes = catalog.resolve("app.notes").unwrap();
        assert_eq!(catalog.display_name(notes), "notes");
    }

    #[test]
    fn reports_missing_ambiguous_and_out_of_scope_tables() {
        let catalog = Catalog::new(
            &["public", "app"],
            &[("app", "notes"), ("public", "notes"), ("other", "x")],
        );
        assert_eq!(
            catalog.resolve("nope").unwrap_err(),
            "table `nope` not found in schemas public, app"
        );
        assert_eq!(
            catalog.resolve("notes").unwrap_err(),
            "table `notes` is ambiguous: it exists in schemas public, app; write `public.notes`"
        );
        let notes = catalog.resolve("app.notes").unwrap();
        assert_eq!(catalog.display_name(notes), "app.notes");
        assert_eq!(
            catalog.resolve("other.x").unwrap_err(),
            "schema `other` is not listed in `database.schemas`"
        );
        assert_eq!(
            catalog.resolve("app.nope").unwrap_err(),
            "table `app.nope` not found"
        );
    }
}
