use super::Op;
use super::plan::Entry;
use crate::catalog::Catalog;
use crate::config::{Config, Unspecified};

/// Which cells of identities × tables in scope × operations have at least one case.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Coverage {
    pub policy: Unspecified,
    pub identities: Vec<String>,
    pub tables: Vec<TableCoverage>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TableCoverage {
    pub table: String,
    /// One per identity, in `Coverage::identities` order, indexed like `Op::ALL`.
    pub cells: Vec<[bool; 4]>,
}

/// The operations of one identity on one table that have no case.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Gap<'a> {
    pub identity: &'a str,
    pub ops: Vec<Op>,
}

impl Coverage {
    pub fn total(&self) -> usize {
        self.tables.len() * self.identities.len() * Op::ALL.len()
    }

    pub fn specified(&self) -> usize {
        self.tables
            .iter()
            .flat_map(|t| &t.cells)
            .flatten()
            .filter(|&&given| given)
            .count()
    }

    pub fn unspecified(&self) -> usize {
        self.total() - self.specified()
    }

    pub fn fails(&self) -> bool {
        self.policy == Unspecified::Fail && self.unspecified() > 0
    }

    /// Unspecified operations per table, then per identity, skipping identities with none.
    pub fn gaps(&self) -> Vec<(&str, Vec<Gap<'_>>)> {
        self.tables
            .iter()
            .map(|table| {
                let identities = self
                    .identities
                    .iter()
                    .zip(&table.cells)
                    .map(|(identity, cells)| {
                        let ops = Op::ALL
                            .into_iter()
                            .zip(cells)
                            .filter(|(_, given)| !**given)
                            .map(|(op, _)| op)
                            .collect::<Vec<_>>();
                        Gap {
                            identity: identity.as_str(),
                            ops,
                        }
                    })
                    .filter(|gap| !gap.ops.is_empty())
                    .collect::<Vec<_>>();
                (table.table.as_str(), identities)
            })
            .filter(|(_, identities)| !identities.is_empty())
            .collect()
    }
}

pub fn compute(config: &Config, catalog: &Catalog, plan: &[Entry]) -> Coverage {
    let tables = catalog
        .tables()
        .iter()
        .map(|table| TableCoverage {
            table: catalog.display_name(table),
            cells: config
                .identities
                .iter()
                .map(|identity| {
                    plan.iter()
                        .find(|e| e.table.oid == table.oid && e.identity.name == identity.name)
                        .map_or([false; 4], Entry::specified)
                })
                .collect(),
        })
        .collect();
    Coverage {
        policy: config.unspecified,
        identities: config.identities.iter().map(|i| i.name.clone()).collect(),
        tables,
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;
    use crate::config;
    use crate::runner::{plan, resolve_tables};

    fn coverage(yaml: &str, tables: &[(&str, &str)]) -> Coverage {
        let config = config::parse(yaml, Path::new("rlsspec.yaml"), &|_| None).unwrap();
        let catalog = Catalog::new(&["public"], tables);
        let mut diagnostics = Vec::new();
        let resolved = resolve_tables(&config, &catalog, &mut diagnostics);
        let plan = plan::build(&config, &resolved, &catalog, &mut diagnostics);
        assert_eq!(diagnostics, []);
        compute(&config, &catalog, &plan)
    }

    #[test]
    fn todo_cells_are_unspecified_and_beat_defaults() {
        let coverage = coverage(
            r#"version: 1
database: { url: postgres://localhost/db }
identities:
  alice: { role: app }
  bob: { role: app }
unspecified: fail
defaults:
  alice:
    "*": { select: deny, insert: deny, update: deny, delete: deny }
  bob:
    "*": { select: todo }
expect:
  notes:
    alice: { update: todo }
    bob: { delete: deny }
"#,
            &[("public", "notes"), ("public", "tags")],
        );
        assert_eq!(coverage.total(), 16);
        assert_eq!(coverage.specified(), 8);
        assert!(coverage.fails());
        let op = |ops: &[Op]| ops.iter().map(|o| o.as_str()).collect::<Vec<_>>();
        let gaps: Vec<_> = coverage
            .gaps()
            .into_iter()
            .flat_map(|(table, gaps)| {
                gaps.into_iter()
                    .map(move |gap| (table, gap.identity, op(&gap.ops)))
            })
            .collect();
        assert_eq!(
            gaps,
            [
                ("notes", "alice", vec!["update"]),
                ("notes", "bob", vec!["select", "insert", "update"]),
                ("tags", "bob", vec!["select", "insert", "update", "delete"]),
            ]
        );
    }

    #[test]
    fn full_coverage_never_fails() {
        let coverage = coverage(
            r#"version: 1
database: { url: postgres://localhost/db }
identities:
  alice: { role: app }
unspecified: fail
defaults:
  alice:
    "*": { select: deny, insert: deny, update: deny, delete: deny }
"#,
            &[("public", "notes")],
        );
        assert_eq!((coverage.specified(), coverage.total()), (4, 4));
        assert!(!coverage.fails());
        assert!(coverage.gaps().is_empty());
    }
}
