use std::collections::{HashMap, HashSet};
use std::path::Path;

use serde_saphyr::Spanned;

use super::map::SpannedMap;
use super::raw::{RawConfig, RawDatabase, RawIdentity, RawOps, RawSelect, RawWrites, Value};
use super::vars::{self, Mode, Reference};
use super::{
    Assignments, Block, Config, Database, DeleteCase, Diagnostic, Identity, InsertCase, Ops,
    Select, SelectCase, Span, TableRef, UpdateCase, Writes,
};

const SUPPORTED_VERSION: u32 = 1;
const WILDCARD: &str = "*";

pub fn resolve(
    raw: RawConfig,
    base: &Path,
    env: &dyn Fn(&str) -> Option<String>,
) -> Result<Config, Vec<Diagnostic>> {
    let mut r = Resolver {
        diagnostics: Vec::new(),
        vars: HashMap::new(),
        env,
    };

    if raw.version.value != SUPPORTED_VERSION {
        r.error(
            span(&raw.version),
            format!(
                "unsupported version {}, expected {SUPPORTED_VERSION}",
                raw.version.value
            ),
        );
    }
    r.load_vars(raw.vars);
    let database = r.database(raw.database);
    let setup = raw
        .setup
        .into_iter()
        .map(|file| {
            let full = base.join(&file.value);
            if !full.is_file() {
                let message = format!(
                    "setup file `{}` not found next to the config file",
                    file.value
                );
                r.error(span(&file), message);
            }
            full
        })
        .collect();
    let identities = r.identities(raw.identities);
    let known: HashSet<String> = identities.iter().map(|i| i.name.clone()).collect();

    let mut defaults = Vec::new();
    for (identity, tables) in raw.defaults {
        r.check_identity(&identity, &known);
        for (table, ops) in tables {
            let table_ref = if table.value == WILDCARD {
                TableRef::All
            } else {
                TableRef::Named(table.value.clone())
            };
            defaults.push(r.block(table_ref, &identity.value, ops, span(&table)));
        }
    }

    let mut expect = Vec::new();
    for (table, by_identity) in raw.expect {
        if table.value == WILDCARD {
            r.error(span(&table), "`*` is only allowed under `defaults`".into());
        }
        for (identity, ops) in by_identity {
            r.check_identity(&identity, &known);
            let table_ref = TableRef::Named(table.value.clone());
            expect.push(r.block(table_ref, &identity.value, ops, span(&identity)));
        }
    }

    if r.diagnostics.is_empty() {
        Ok(Config {
            database,
            allowed_hosts: raw.safety.allowed_hosts,
            setup,
            identities,
            unspecified: raw.unspecified,
            defaults,
            expect,
        })
    } else {
        r.diagnostics.sort_by_key(|d| d.span);
        Err(r.diagnostics)
    }
}

fn span<T>(spanned: &Spanned<T>) -> Span {
    spanned.referenced.into()
}

struct Resolver<'a> {
    diagnostics: Vec<Diagnostic>,
    vars: HashMap<String, String>,
    env: &'a dyn Fn(&str) -> Option<String>,
}

impl Resolver<'_> {
    fn error(&mut self, span: Span, message: String) {
        self.diagnostics.push(Diagnostic { span, message });
    }

    fn lookup(&self, reference: Reference, allow_vars: bool) -> Result<String, String> {
        match reference {
            Reference::Env(name) => {
                (self.env)(name).ok_or_else(|| format!("environment variable `{name}` is not set"))
            }
            Reference::Var(name) if !allow_vars => Err(format!(
                "`vars` can only reference `${{env:NAME}}`, found `${{{name}}}`"
            )),
            Reference::Var(name) => self
                .vars
                .get(name)
                .cloned()
                .ok_or_else(|| format!("undefined variable `{name}`")),
        }
    }

    fn substitute(&mut self, text: &str, at: Span, mode: Mode) -> Option<String> {
        let result = vars::substitute(text, mode, |reference| self.lookup(reference, true));
        result.map_err(|message| self.error(at, message)).ok()
    }

    fn load_vars(&mut self, raw: SpannedMap<Spanned<String>>) {
        for (name, value) in raw {
            if !vars::is_identifier(&name.value) {
                self.error(
                    span(&name),
                    format!("invalid variable name `{}`", name.value),
                );
                continue;
            }
            let result = vars::substitute(&value.value, Mode::Raw, |reference| {
                self.lookup(reference, false)
            });
            match result {
                Ok(resolved) => {
                    self.vars.insert(name.value, resolved);
                }
                Err(message) => self.error(span(&value), message),
            }
        }
    }

    fn database(&mut self, raw: RawDatabase) -> Database {
        let url = match self.substitute(&raw.url.value, span(&raw.url), Mode::Raw) {
            Some(url) if url.trim().is_empty() => {
                self.error(span(&raw.url), "`database.url` is empty".into());
                url
            }
            url => url.unwrap_or_default(),
        };
        let schemas = match raw.schemas {
            None => vec!["public".to_owned()],
            Some(list) => {
                if list.value.is_empty() {
                    self.error(span(&list), "`database.schemas` must not be empty".into());
                }
                list.value
                    .into_iter()
                    .map(|schema| {
                        if schema.value.trim().is_empty() {
                            self.error(span(&schema), "schema name is empty".into());
                        }
                        schema.value
                    })
                    .collect()
            }
        };
        Database { url, schemas }
    }

    fn identities(&mut self, raw: Spanned<SpannedMap<RawIdentity>>) -> Vec<Identity> {
        if raw.value.is_empty() {
            self.error(
                span(&raw),
                "`identities` must define at least one identity".into(),
            );
        }
        raw.value
            .into_iter()
            .map(|(name, identity)| {
                if identity.role.value.trim().is_empty() {
                    self.error(
                        span(&identity.role),
                        format!("identity `{}` has an empty role", name.value),
                    );
                }
                let gucs = identity
                    .gucs
                    .into_iter()
                    .map(|(key, value)| {
                        if key.value.trim().is_empty() {
                            self.error(span(&key), "setting name is empty".into());
                        }
                        let value = self.substitute(&value.value, span(&value), Mode::Raw);
                        (key.value, value.unwrap_or_default())
                    })
                    .collect();
                Identity {
                    name: name.value,
                    role: identity.role.value,
                    gucs,
                }
            })
            .collect()
    }

    fn check_identity(&mut self, identity: &Spanned<String>, known: &HashSet<String>) {
        if !known.contains(&identity.value) {
            self.error(
                span(identity),
                format!(
                    "unknown identity `{}`; declare it under `identities`",
                    identity.value
                ),
            );
        }
    }

    fn block(&mut self, table: TableRef, identity: &str, raw: RawOps, at: Span) -> Block {
        if raw.select.is_none()
            && raw.insert.is_none()
            && raw.update.is_none()
            && raw.delete.is_none()
        {
            self.error(
                at,
                "no operations given; expected select, insert, update or delete".into(),
            );
        }
        let ops = Ops {
            select: raw.select.map(|select| self.select(select)),
            insert: raw.insert.map(|writes| {
                self.writes(writes, |r, case| InsertCase {
                    values: r.assignments(&case.value.values, "values"),
                    expect: case.value.expect,
                    span: span(&case),
                })
            }),
            update: raw.update.map(|writes| {
                self.writes(writes, |r, case| UpdateCase {
                    predicate: r.predicate(&case.value.predicate),
                    set: case.value.set.as_ref().map(|set| r.assignments(set, "set")),
                    expect: case.value.expect,
                    span: span(&case),
                })
            }),
            delete: raw.delete.map(|writes| {
                self.writes(writes, |r, case| DeleteCase {
                    predicate: r.predicate(&case.value.predicate),
                    expect: case.value.expect,
                    span: span(&case),
                })
            }),
        };
        Block {
            table,
            identity: identity.to_owned(),
            ops,
            span: at,
        }
    }

    fn select(&mut self, raw: Spanned<RawSelect>) -> SelectCase {
        let at = span(&raw);
        let select = match raw.value {
            RawSelect::Deny => Select::Deny,
            RawSelect::All => Select::All,
            RawSelect::Rows { rows, subset } => Select::Rows {
                predicate: self.predicate(&rows),
                subset,
            },
        };
        SelectCase { select, span: at }
    }

    fn writes<R, C>(
        &mut self,
        raw: Spanned<RawWrites<R>>,
        mut case: impl FnMut(&mut Self, Spanned<R>) -> C,
    ) -> Writes<C> {
        let at = span(&raw);
        match raw.value {
            RawWrites::Shorthand(expect) => Writes::Shorthand { expect, span: at },
            RawWrites::Cases(cases) => {
                if cases.is_empty() {
                    self.error(at, "list of cases is empty".into());
                }
                Writes::Cases(cases.into_iter().map(|c| case(self, c)).collect())
            }
        }
    }

    fn predicate(&mut self, raw: &Spanned<String>) -> String {
        if raw.value.trim().is_empty() {
            self.error(span(raw), "predicate is empty".into());
        }
        self.substitute(&raw.value, span(raw), Mode::SqlLiteral)
            .unwrap_or_default()
    }

    fn assignments(&mut self, raw: &Spanned<SpannedMap<Value>>, field: &str) -> Assignments {
        if raw.value.is_empty() {
            self.error(
                span(raw),
                format!("`{field}` must name at least one column"),
            );
        }
        raw.value
            .iter()
            .map(|(column, value)| {
                let resolved = value.value.as_ref().map(|text| {
                    self.substitute(text, span(value), Mode::Raw)
                        .unwrap_or_default()
                });
                (column.value.clone(), resolved)
            })
            .collect()
    }
}
