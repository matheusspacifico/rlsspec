use postgres::Transaction;

use crate::pg;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Table {
    pub schema: String,
    pub name: String,
    pub primary_key: Vec<String>,
}

impl Table {
    pub fn sql_name(&self) -> String {
        pg::qualified(&self.schema, &self.name)
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Catalog {
    schemas: Vec<String>,
    tables: Vec<Table>,
}

const TABLES: &str = "
SELECT n.nspname::text, c.relname::text,
       coalesce((SELECT array_agg(a.attname::text ORDER BY k.ord)
                 FROM pg_index i
                 CROSS JOIN LATERAL unnest(i.indkey) WITH ORDINALITY AS k(attnum, ord)
                 JOIN pg_attribute a ON a.attrelid = i.indrelid AND a.attnum = k.attnum
                 WHERE i.indrelid = c.oid AND i.indisprimary), '{}')
FROM pg_class c
JOIN pg_namespace n ON n.oid = c.relnamespace
WHERE c.relkind IN ('r', 'p') AND n.nspname = ANY($1)
ORDER BY 1, 2";

pub fn load(tx: &mut Transaction, schemas: &[String]) -> Result<Catalog, postgres::Error> {
    let tables = tx
        .query(TABLES, &[&schemas])?
        .into_iter()
        .map(|row| Table {
            schema: row.get(0),
            name: row.get(1),
            primary_key: row.get(2),
        })
        .collect();
    Ok(Catalog {
        schemas: schemas.to_vec(),
        tables,
    })
}

impl Catalog {
    #[cfg(test)]
    fn new(schemas: &[&str], tables: &[(&str, &str)]) -> Self {
        Self {
            schemas: schemas.iter().map(|s| s.to_string()).collect(),
            tables: tables
                .iter()
                .map(|(schema, name)| Table {
                    schema: schema.to_string(),
                    name: name.to_string(),
                    primary_key: Vec::new(),
                })
                .collect(),
        }
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
