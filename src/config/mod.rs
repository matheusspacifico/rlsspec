mod diagnostic;
mod map;
mod raw;
mod resolve;
pub mod vars;

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::Deserialize;
use serde_saphyr::{DefaultMessageFormatter, MessageFormatter};

pub use diagnostic::{Diagnostic, Span};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Config {
    pub database: Database,
    pub safety: Safety,
    pub setup: Vec<PathBuf>,
    pub identities: Vec<Identity>,
    pub unspecified: Unspecified,
    pub defaults: Vec<Block>,
    pub expect: Vec<Block>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Database {
    pub url: String,
    pub schemas: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Safety {
    pub allowed_hosts: Vec<String>,
    pub lock_timeout: String,
    pub statement_timeout: String,
}

pub const DEFAULT_LOCK_TIMEOUT: &str = "5s";
pub const DEFAULT_STATEMENT_TIMEOUT: &str = "30s";

impl Default for Safety {
    fn default() -> Self {
        Self {
            allowed_hosts: Vec::new(),
            lock_timeout: DEFAULT_LOCK_TIMEOUT.to_owned(),
            statement_timeout: DEFAULT_STATEMENT_TIMEOUT.to_owned(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Identity {
    pub name: String,
    pub role: String,
    pub gucs: Vec<(String, String)>,
    pub span: Span,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Unspecified {
    Ignore,
    #[default]
    Warn,
    Fail,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TableRef {
    All,
    Named(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Block {
    pub table: TableRef,
    pub table_span: Span,
    pub identity: String,
    pub ops: Ops,
    pub span: Span,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Ops {
    pub select: Option<Spec<SelectCase>>,
    pub insert: Option<Spec<Writes<InsertCase>>>,
    pub update: Option<Spec<Writes<UpdateCase>>>,
    pub delete: Option<Spec<Writes<DeleteCase>>>,
}

/// An operation is either `todo` (known, deliberately left unspecified) or given cases.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Spec<T> {
    Todo,
    Given(T),
}

impl<T> Spec<T> {
    pub fn given(&self) -> Option<&T> {
        match self {
            Spec::Todo => None,
            Spec::Given(value) => Some(value),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Expectation {
    Allow,
    Deny,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Select {
    Deny,
    All,
    Rows { predicate: String, subset: bool },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SelectCase {
    pub select: Select,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Writes<C> {
    Shorthand { expect: Expectation, span: Span },
    Cases(Vec<C>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Assignment {
    pub column: String,
    pub value: Option<String>,
    pub span: Span,
}

pub type Assignments = Vec<Assignment>;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InsertCase {
    pub values: Assignments,
    pub expect: Expectation,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateCase {
    pub predicate: String,
    pub set: Option<Assignments>,
    pub expect: Expectation,
    pub span: Span,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeleteCase {
    pub predicate: String,
    pub expect: Expectation,
    pub span: Span,
}

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("cannot read {}", path.display())]
    Read {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error("{}", diagnostic::render(path, text, diagnostics, *stage, false))]
    Invalid {
        path: PathBuf,
        text: String,
        diagnostics: Vec<Diagnostic>,
        stage: Stage,
    },
}

/// When a located error was found: while loading the config, or at run time (setup files,
/// table and column resolution, identity preflight), before any case ran.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stage {
    Load,
    Run,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Source {
    pub path: PathBuf,
    pub text: String,
}

impl Source {
    pub fn read(path: &Path) -> Result<Self, ConfigError> {
        let text = fs::read_to_string(path).map_err(|source| ConfigError::Read {
            path: path.to_path_buf(),
            source,
        })?;
        Ok(Self {
            path: path.to_path_buf(),
            text,
        })
    }

    /// A run-time error located in this file.
    pub fn invalid(&self, diagnostics: Vec<Diagnostic>) -> ConfigError {
        ConfigError::Invalid {
            path: self.path.clone(),
            text: self.text.clone(),
            diagnostics,
            stage: Stage::Run,
        }
    }
}

impl ConfigError {
    /// The located diagnostics with terminal styles, for `Invalid`; `None` otherwise.
    pub fn styled(&self) -> Option<String> {
        match self {
            ConfigError::Invalid {
                path,
                text,
                diagnostics,
                stage,
            } => Some(diagnostic::render(path, text, diagnostics, *stage, true)),
            ConfigError::Read { .. } => None,
        }
    }
}

pub fn load(path: &Path) -> Result<(Config, Source), ConfigError> {
    let source = Source::read(path)?;
    let config = parse(&source.text, path, &|name| std::env::var(name).ok())?;
    Ok((config, source))
}

pub fn parse(
    text: &str,
    path: &Path,
    env: &dyn Fn(&str) -> Option<String>,
) -> Result<Config, ConfigError> {
    let invalid = |diagnostics| ConfigError::Invalid {
        path: path.to_path_buf(),
        text: text.to_owned(),
        diagnostics,
        stage: Stage::Load,
    };
    let options = serde_saphyr::options! { with_snippet: false };
    let raw = serde_saphyr::from_str_with_options(text, options).map_err(|err| {
        let message = escape_control(&DefaultMessageFormatter.format_message(&err));
        // serde-saphyr appends advice about its own Options API, which means nothing to users.
        let message = match message.split_once(", set DuplicateKeyPolicy") {
            Some((head, _)) => head.to_owned(),
            None => message,
        };
        let span = err.location().map(Span::from).unwrap_or_default();
        invalid(vec![Diagnostic { span, message }])
    })?;
    let base = path.parent().unwrap_or(Path::new(""));
    resolve::resolve(raw, base, env).map_err(invalid)
}

fn escape_control(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        if c.is_control() {
            out.extend(c.escape_debug());
        } else {
            out.push(c);
        }
    }
    out
}
