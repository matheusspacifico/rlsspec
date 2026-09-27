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
    pub allowed_hosts: Vec<String>,
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
pub struct Identity {
    pub name: String,
    pub role: String,
    pub gucs: Vec<(String, String)>,
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
    pub identity: String,
    pub ops: Ops,
    pub span: Span,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Ops {
    pub select: Option<SelectCase>,
    pub insert: Option<Writes<InsertCase>>,
    pub update: Option<Writes<UpdateCase>>,
    pub delete: Option<Writes<DeleteCase>>,
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

pub type Assignments = Vec<(String, Option<String>)>;

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
    #[error("{}", diagnostic::render(path, text, diagnostics))]
    Invalid {
        path: PathBuf,
        text: String,
        diagnostics: Vec<Diagnostic>,
    },
}

pub fn load(path: &Path) -> Result<Config, ConfigError> {
    let text = fs::read_to_string(path).map_err(|source| ConfigError::Read {
        path: path.to_path_buf(),
        source,
    })?;
    parse(&text, path, &|name| std::env::var(name).ok())
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
