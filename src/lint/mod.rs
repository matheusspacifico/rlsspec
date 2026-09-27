mod rls001;
mod rls002;
mod rls003;
mod rls004;
mod rls005;
mod rls006;
mod rls007;
mod rls008;
mod rule;

use std::path::{Path, PathBuf};

use crate::catalog::lint::{self as facts, LintCatalog, RlsTable, RoleFacts};
use crate::catalog::{Catalog, Table};
use crate::config::{Config, Ignore, Source, Span};
use crate::pg::Target;
use crate::runner::plan::Entry;
use crate::runner::{self, RunError};

pub use rule::{Key, Rule, Severity};

const RULES: [fn(&Context) -> Vec<Finding>; 8] = [
    rls001::check,
    rls002::check,
    rls003::check,
    rls004::check,
    rls005::check,
    rls006::check,
    rls007::check,
    rls008::check,
];

/// A schema-qualified object a finding is about.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Name {
    pub schema: String,
    pub name: String,
}

impl Name {
    /// `name` or `schema.name`, as written in a `lint.ignore` entry.
    fn matches(&self, pattern: &str) -> bool {
        pattern == self.name
            || pattern
                .split_once('.')
                .is_some_and(|(schema, name)| schema == self.schema && name == self.name)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finding {
    pub rule: Rule,
    pub severity: Severity,
    /// What it is about, as printed: a table, function, view or role, or a stale ignore's location.
    pub object: String,
    pub role: Option<String>,
    pub hint: String,
    pub table: Option<Name>,
    pub function: Option<Name>,
    pub view: Option<Name>,
    /// The identities concerned, which `lint.ignore` entries match on.
    pub identities: Vec<String>,
    /// For a stale ignore, where its `lint.ignore` entry is written.
    pub stale_ignore: Option<Span>,
}

impl Finding {
    fn new(rule: Rule, object: String, hint: String) -> Self {
        Self {
            rule,
            severity: rule.severity(),
            object,
            role: None,
            hint,
            table: None,
            function: None,
            view: None,
            identities: Vec::new(),
            stale_ignore: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Skipped {
    pub rule: Rule,
    pub reason: &'static str,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LintReport {
    /// The spec file the run loaded.
    pub spec: PathBuf,
    /// Grouped by rule; ignored findings are left out, stale ignores are warnings of their rule.
    pub findings: Vec<Finding>,
    pub ignored: usize,
    pub skipped: Vec<Skipped>,
}

impl LintReport {
    pub fn count(&self, severity: Severity) -> usize {
        self.findings
            .iter()
            .filter(|f| f.severity == severity)
            .count()
    }

    /// 1 when there is any error left after the ignore list; warnings and info never fail.
    pub fn exit_code(&self) -> u8 {
        u8::from(self.count(Severity::Error) > 0)
    }
}

/// Loads the spec like `cover` (setup, catalog, tables, identities' roles), then reads the catalog only.
pub fn run(config: &Config, source: &Source, target: &Target) -> Result<LintReport, RunError> {
    runner::inspect(config, source, target, |session, catalog, plan| {
        let mut roles: Vec<&str> = Vec::new();
        for identity in &config.identities {
            if !roles.contains(&identity.role.as_str()) {
                roles.push(&identity.role);
            }
        }
        let facts = facts::load(session.tx(), catalog, &config.database.schemas, &roles)?;
        let cx = Context {
            config,
            catalog,
            facts: &facts,
            plan,
        };
        Ok(check(&cx, &config.lint.ignore, &source.path))
    })
}

fn check(cx: &Context, ignore: &[Ignore], path: &Path) -> LintReport {
    let mut skipped = Vec::new();
    if cx.facts.server_version < facts::SECURITY_INVOKER_VERSION {
        skipped.push(Skipped {
            rule: Rule::Rls007,
            reason: "needs PostgreSQL 15 or later (security_invoker)",
        });
    }

    let mut used = vec![false; ignore.len()];
    let mut report = LintReport {
        spec: path.to_path_buf(),
        skipped,
        ..LintReport::default()
    };
    for finding in RULES.iter().flat_map(|rule| rule(cx)) {
        let mut hit = false;
        for (entry, used) in ignore.iter().zip(&mut used) {
            if matches(entry, &finding) {
                *used = true;
                hit = true;
            }
        }
        if hit {
            report.ignored += 1;
        } else {
            report.findings.push(finding);
        }
    }
    for (entry, _) in ignore.iter().zip(used).filter(|(_, used)| !used) {
        if report.skipped.iter().any(|s| s.rule == entry.rule) {
            continue;
        }
        let mut stale = Finding::new(
            entry.rule,
            format!("{}:{}", path.display(), entry.span.line),
            format!(
                "stale ignore: no {} finding matches it; remove it",
                entry.rule
            ),
        );
        stale.severity = Severity::Warn;
        stale.stale_ignore = Some(entry.span);
        report.findings.push(stale);
    }
    report.findings.sort_by_key(|f| f.rule);
    report
}

fn matches(entry: &Ignore, finding: &Finding) -> bool {
    let name = |pattern: &Option<String>, name: &Option<Name>| match (pattern, name) {
        (None, _) => true,
        (Some(pattern), Some(name)) => name.matches(pattern),
        (Some(_), None) => false,
    };
    entry.rule == finding.rule
        && name(&entry.table, &finding.table)
        && name(&entry.function, &finding.function)
        && name(&entry.view, &finding.view)
        && entry
            .identity
            .as_ref()
            .is_none_or(|identity| finding.identities.contains(identity))
}

/// What every rule reads: the spec, the tables in scope, the catalog facts and the resolved plan.
struct Context<'a> {
    config: &'a Config,
    catalog: &'a Catalog,
    facts: &'a LintCatalog,
    plan: &'a [Entry<'a>],
}

impl Context<'_> {
    fn tables(&self) -> impl Iterator<Item = (&Table, &RlsTable)> {
        self.facts.tables.iter().filter_map(|facts| {
            let table = self.catalog.tables().iter().find(|t| t.oid == facts.oid)?;
            Some((table, facts))
        })
    }

    fn table(&self, oid: u32) -> Option<&Table> {
        self.catalog.tables().iter().find(|t| t.oid == oid)
    }

    fn role(&self, name: &str) -> Option<&RoleFacts> {
        self.facts.roles.iter().find(|r| r.name == name)
    }

    /// Identity roles that RLS applies to. A superuser or `BYPASSRLS` role is only reported by
    /// RLS003: no policy, grant, FORCE or view setting changes anything for it.
    fn subject_roles(&self) -> impl Iterator<Item = &RoleFacts> {
        self.facts.roles.iter().filter(|r| !r.bypasses_rls())
    }

    fn identities_of(&self, role: &str) -> Vec<String> {
        self.config
            .identities
            .iter()
            .filter(|i| i.role == role)
            .map(|i| i.name.clone())
            .collect()
    }

    fn on_table(&self, rule: Rule, table: &Table, hint: String) -> Finding {
        let mut finding = Finding::new(rule, self.catalog.display_name(table), hint);
        finding.table = Some(Name {
            schema: table.schema.clone(),
            name: table.name.clone(),
        });
        finding
    }

    fn for_role(&self, mut finding: Finding, role: &RoleFacts) -> Finding {
        finding.identities = self.identities_of(&role.name);
        finding.role = Some(role.name.clone());
        finding
    }
}
