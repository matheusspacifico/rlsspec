use std::collections::HashMap;

use super::Origin;
use crate::catalog::{Catalog, Table};
use crate::config::{
    Block, Config, DeleteCase, Diagnostic, Expectation, Identity, InsertCase, Select, SelectCase,
    Span, Spec, TableRef, UpdateCase, Writes,
};

/// Everything to check for one identity on one table, each operation taken from the most
/// specific place that defines it: an `expect` entry, then a named-table default, then `*`.
pub struct Entry<'c> {
    pub table: &'c Table,
    pub identity: &'c Identity,
    pub select: Option<(&'c Spec<SelectCase>, Origin)>,
    pub insert: Option<(&'c Spec<Writes<InsertCase>>, Origin)>,
    pub update: Option<(&'c Spec<Writes<UpdateCase>>, Origin)>,
    pub delete: Option<(&'c Spec<Writes<DeleteCase>>, Origin)>,
}

impl Entry<'_> {
    /// Whether each of select, insert, update and delete has at least one case (not `todo`).
    pub fn specified(&self) -> [bool; 4] {
        fn given<T>(op: Option<(&Spec<T>, Origin)>) -> bool {
            matches!(op, Some((Spec::Given(_), _)))
        }
        [
            given(self.select),
            given(self.insert),
            given(self.update),
            given(self.delete),
        ]
    }

    /// Whether every operation is denied: `select: deny`, and for each write the `deny` shorthand
    /// or only `expect: deny` cases. A `todo` or missing operation is not a deny.
    pub fn denies_everything(&self) -> bool {
        fn writes<C>(
            op: Option<(&Spec<Writes<C>>, Origin)>,
            expect: fn(&C) -> Expectation,
        ) -> bool {
            match op {
                Some((Spec::Given(Writes::Shorthand { expect, .. }), _)) => {
                    *expect == Expectation::Deny
                }
                Some((Spec::Given(Writes::Cases(cases)), _)) => {
                    cases.iter().all(|c| expect(c) == Expectation::Deny)
                }
                Some((Spec::Todo, _)) | None => false,
            }
        }
        let select = matches!(
            self.select,
            Some((
                Spec::Given(SelectCase {
                    select: Select::Deny,
                    ..
                }),
                _
            ))
        );
        select
            && writes(self.insert, |c| c.expect)
            && writes(self.update, |c| c.expect)
            && writes(self.delete, |c| c.expect)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Rank {
    Expect,
    NamedDefault,
    Wildcard,
}

/// `tables` holds the resolved table of each block, `expect` blocks first, then `defaults`.
pub fn build<'c>(
    config: &'c Config,
    tables: &[Option<&'c Table>],
    catalog: &'c Catalog,
    diagnostics: &mut Vec<Diagnostic>,
) -> Vec<Entry<'c>> {
    let identities: HashMap<&str, &Identity> = config
        .identities
        .iter()
        .map(|i| (i.name.as_str(), i))
        .collect();
    let mut plan = Plan {
        entries: Vec::new(),
        seen: HashMap::new(),
        catalog,
        diagnostics,
    };
    let blocks: Vec<(&Block, Option<&Table>)> = config
        .expect
        .iter()
        .chain(&config.defaults)
        .zip(tables.iter().copied())
        .collect();
    let (expect, defaults) = blocks.split_at(config.expect.len());
    let named = defaults.iter().filter(|(b, _)| b.table != TableRef::All);
    let wildcards = defaults.iter().filter(|(b, _)| b.table == TableRef::All);

    for (rank, (block, table)) in expect
        .iter()
        .map(|b| (Rank::Expect, b))
        .chain(named.map(|b| (Rank::NamedDefault, b)))
    {
        if let (Some(table), Some(identity)) = (table, identities.get(block.identity.as_str())) {
            plan.add(table, identity, block, rank);
        }
    }
    for (block, _) in wildcards {
        if let Some(identity) = identities.get(block.identity.as_str()) {
            for table in catalog.tables() {
                plan.add(table, identity, block, Rank::Wildcard);
            }
        }
    }
    plan.entries
}

struct Plan<'c, 'd> {
    entries: Vec<Entry<'c>>,
    seen: HashMap<(u32, &'c str, Rank), Span>,
    catalog: &'c Catalog,
    diagnostics: &'d mut Vec<Diagnostic>,
}

impl<'c> Plan<'c, '_> {
    fn add(&mut self, table: &'c Table, identity: &'c Identity, block: &'c Block, rank: Rank) {
        let key = (table.oid, identity.name.as_str(), rank);
        if rank != Rank::Wildcard {
            if let Some(first) = self.seen.get(&key) {
                self.diagnostics.push(Diagnostic {
                    span: block.table_span,
                    message: format!(
                        "table {} already has a block for `{}` at line {}; merge the two",
                        self.catalog.display_name(table),
                        identity.name,
                        first.line
                    ),
                });
                return;
            }
            self.seen.insert(key, block.table_span);
        }

        let position = self
            .entries
            .iter()
            .position(|e| e.table.oid == table.oid && e.identity.name == identity.name);
        let entry = match position {
            Some(i) => &mut self.entries[i],
            None => {
                self.entries.push(Entry {
                    table,
                    identity,
                    select: None,
                    insert: None,
                    update: None,
                    delete: None,
                });
                let last = self.entries.len() - 1;
                &mut self.entries[last]
            }
        };
        let origin = match rank {
            Rank::Expect => Origin::Expect,
            Rank::NamedDefault | Rank::Wildcard => Origin::Default,
        };
        // Blocks arrive from the most to the least specific, so the first to define an op wins.
        let ops = &block.ops;
        entry.select = entry.select.or(ops.select.as_ref().map(|c| (c, origin)));
        entry.insert = entry.insert.or(ops.insert.as_ref().map(|c| (c, origin)));
        entry.update = entry.update.or(ops.update.as_ref().map(|c| (c, origin)));
        entry.delete = entry.delete.or(ops.delete.as_ref().map(|c| (c, origin)));
    }
}
