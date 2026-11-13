use std::collections::HashMap;

use super::{Op, Origin};
use crate::catalog::{Catalog, Table};
use crate::config::{
    Block, Config, DeleteCase, Diagnostic, Expectation, Identity, InsertCase, OnEmpty, Select,
    SelectCase, Span, Spec, TableRef, UpdateCase, Writes,
};

/// One operation of an entry: its cases, and the block they were taken from.
#[derive(Debug)]
pub struct Planned<'c, T> {
    pub spec: &'c Spec<T>,
    pub origin: Origin,
    pub block: &'c Block,
}

impl<T> Clone for Planned<'_, T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T> Copy for Planned<'_, T> {}

impl<'c, T> Planned<'c, T> {
    /// The cases, unless the operation is `todo`.
    pub fn given(&self) -> Option<&'c T> {
        self.spec.given()
    }

    /// Whether the cases were expanded from a wildcard default rather than written for this table.
    pub fn is_wildcard(&self) -> bool {
        self.block.table == TableRef::All
    }
}

/// Everything to check for one identity on one table, each operation taken from the most
/// specific place that defines it: an `expect` entry, then a named-table default, then `*`.
pub struct Entry<'c> {
    pub table: &'c Table,
    pub identity: &'c Identity,
    pub select: Option<Planned<'c, SelectCase>>,
    pub insert: Option<Planned<'c, Writes<InsertCase>>>,
    pub update: Option<Planned<'c, Writes<UpdateCase>>>,
    pub delete: Option<Planned<'c, Writes<DeleteCase>>>,
    /// Whether the table has no rows. Only looked up when a wildcard case needs to know.
    pub empty: bool,
}

impl Entry<'_> {
    /// Whether each of select, insert, update and delete has at least one case that runs: not
    /// `todo`, and not skipped on an empty table.
    pub fn specified(&self) -> [bool; 4] {
        let given = [
            self.select.is_some_and(|p| p.given().is_some()),
            self.insert.is_some_and(|p| p.given().is_some()),
            self.update.is_some_and(|p| p.given().is_some()),
            self.delete.is_some_and(|p| p.given().is_some()),
        ];
        let mut specified = [false; 4];
        for (i, op) in Op::ALL.into_iter().enumerate() {
            specified[i] = given[i] && !self.skips(op);
        }
        specified
    }

    /// The block `op` was taken from, when that is a wildcard default and the operation has cases.
    fn wildcard(&self, op: Op) -> Option<&Block> {
        fn block<'c, T>(planned: Option<Planned<'c, T>>) -> Option<&'c Block> {
            planned
                .filter(|p| p.is_wildcard() && p.given().is_some())
                .map(|p| p.block)
        }
        match op {
            Op::Select => block(self.select),
            Op::Insert => block(self.insert),
            Op::Update => block(self.update),
            Op::Delete => block(self.delete),
        }
    }

    /// Whether the cases of `op` need a row to mean anything and came from a wildcard default:
    /// the cases `on_empty` is about. An insert never is: `insert: deny` is static, and a `values`
    /// case brings its row.
    pub fn asks_about_empty(&self, op: Op) -> bool {
        op != Op::Insert && self.wildcard(op).is_some()
    }

    /// Whether the cases of `op` are skipped: the table is empty and their wildcard default says
    /// `on_empty: skip`.
    pub fn skips(&self, op: Op) -> bool {
        self.empty
            && self.asks_about_empty(op)
            && self
                .wildcard(op)
                .is_some_and(|block| block.on_empty == OnEmpty::Skip)
    }

    /// Whether a vacuous case of `op` should point at `on_empty`: it came from a wildcard default
    /// and the table has no rows.
    pub fn hints_on_empty(&self, op: Op) -> bool {
        self.empty && self.asks_about_empty(op)
    }

    /// Whether every operation is denied: `select: deny`, and for each write the `deny` shorthand
    /// or only `expect: deny` cases. A `todo` or missing operation is not a deny.
    pub fn denies_everything(&self) -> bool {
        // A `rows` case has no expectation: it compares rows, so it is not a deny.
        fn writes<C>(
            op: Option<Planned<Writes<C>>>,
            expect: fn(&C) -> Option<Expectation>,
        ) -> bool {
            match op.and_then(|p| p.given()) {
                Some(Writes::Shorthand { expect, .. }) => *expect == Expectation::Deny,
                Some(Writes::Cases(cases)) => {
                    cases.iter().all(|c| expect(c) == Some(Expectation::Deny))
                }
                None => false,
            }
        }
        let select = matches!(
            self.select.and_then(|p| p.given()),
            Some(SelectCase {
                select: Select::Deny,
                ..
            })
        );
        select
            && writes(self.insert, |c| Some(c.expect))
            && writes(self.update, |c| c.target.expect())
            && writes(self.delete, |c| c.target.expect())
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
                    empty: false,
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
        fn planned<'c, T>(
            spec: &'c Option<Spec<T>>,
            origin: Origin,
            block: &'c Block,
        ) -> Option<Planned<'c, T>> {
            spec.as_ref().map(|spec| Planned {
                spec,
                origin,
                block,
            })
        }
        let ops = &block.ops;
        entry.select = entry.select.or(planned(&ops.select, origin, block));
        entry.insert = entry.insert.or(planned(&ops.insert, origin, block));
        entry.update = entry.update.or(planned(&ops.update, origin, block));
        entry.delete = entry.delete.or(planned(&ops.delete, origin, block));
    }
}
