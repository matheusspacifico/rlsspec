pub mod script;
mod target;
mod tls;

use std::io;
use std::path::{Path, PathBuf};

use postgres::error::SqlState;
use postgres::{Client, IsolationLevel, Transaction};

use crate::config::{ConfigError, Safety, Source};
pub use target::{SslMode, Target, TargetError};
pub use tls::TlsError;

const MIN_SERVER_VERSION: i32 = 140000;

#[derive(Debug, thiserror::Error)]
pub enum PgError {
    #[error("cannot connect to the database: {}", describe(.0))]
    Connect(postgres::Error),
    #[error(transparent)]
    Tls(#[from] TlsError),
    #[error("PostgreSQL {0} is not supported; rlsspec needs PostgreSQL 14 or later")]
    UnsupportedVersion(String),
    #[error("database error: {}", describe(.0))]
    Query(postgres::Error),
    #[error("cannot read setup file {}", path.display())]
    SetupRead {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
    #[error(transparent)]
    SetupTransactionControl(ConfigError),
    #[error("setup file {} failed: {detail}", path.display())]
    Setup { path: PathBuf, detail: String },
}

impl From<postgres::Error> for PgError {
    fn from(err: postgres::Error) -> Self {
        Self::Query(err)
    }
}

pub fn connect(target: &Target) -> Result<Client, PgError> {
    let mut client = target
        .config
        .connect(tls::connector(target)?)
        .map_err(PgError::Connect)?;
    let row = client.query_one(
        "SELECT current_setting('server_version_num')::int, current_setting('server_version')",
        &[],
    )?;
    let number: i32 = row.get(0);
    if number < MIN_SERVER_VERSION {
        return Err(PgError::UnsupportedVersion(row.get(1)));
    }
    Ok(client)
}

/// The one outer transaction. It is never committed: it ends with `rollback`, or with the
/// driver's rollback on drop, or with the server aborting it when the connection closes.
pub struct Session<'a> {
    tx: Transaction<'a>,
    safety: Safety,
}

impl<'a> Session<'a> {
    pub fn begin(client: &'a mut Client, safety: &Safety) -> Result<Self, PgError> {
        let tx = client
            .build_transaction()
            .isolation_level(IsolationLevel::RepeatableRead)
            .start()?;
        let mut session = Self {
            tx,
            safety: safety.clone(),
        };
        session.harden()?;
        Ok(session)
    }

    fn harden(&mut self) -> Result<(), postgres::Error> {
        for (name, value) in [
            ("lock_timeout", self.safety.lock_timeout.as_str()),
            ("statement_timeout", self.safety.statement_timeout.as_str()),
            ("standard_conforming_strings", "on"),
        ] {
            set_local(&mut self.tx, name, value)?;
        }
        Ok(())
    }

    pub fn run_setup(&mut self, files: &[PathBuf]) -> Result<(), PgError> {
        for path in files {
            let source = Source::read(path).map_err(|err| match err {
                ConfigError::Read { source, .. } => PgError::SetupRead {
                    path: path.clone(),
                    source,
                },
                other => PgError::SetupTransactionControl(other),
            })?;
            let found = script::transaction_control(&source.text);
            if !found.is_empty() {
                return Err(PgError::SetupTransactionControl(source.invalid(found)));
            }
            self.tx
                .batch_execute(&source.text)
                .map_err(|err| setup_failed(path, &err))?;
        }
        // A setup file may have switched role or loosened the session settings.
        self.tx.execute("RESET ROLE", &[])?;
        self.harden()?;
        Ok(())
    }

    pub fn tx(&mut self) -> &mut Transaction<'a> {
        &mut self.tx
    }

    /// Runs `f` inside a savepoint that is always rolled back, whatever `f` did.
    pub fn case<T>(&mut self, f: impl FnOnce(&mut Transaction) -> T) -> Result<T, postgres::Error> {
        let mut savepoint = self.tx.savepoint("rlsspec_case")?;
        let out = f(&mut savepoint);
        savepoint.rollback()?;
        Ok(out)
    }

    pub fn rollback(self) -> Result<(), postgres::Error> {
        self.tx.rollback()
    }
}

fn setup_failed(path: &Path, err: &postgres::Error) -> PgError {
    PgError::Setup {
        path: path.to_path_buf(),
        detail: describe(err),
    }
}

pub fn set_local(tx: &mut Transaction, name: &str, value: &str) -> Result<(), postgres::Error> {
    tx.execute("SELECT set_config($1, $2, true)", &[&name, &value])?;
    Ok(())
}

pub fn quote_ident(name: &str) -> String {
    format!("\"{}\"", name.replace('"', "\"\""))
}

pub fn qualified(schema: &str, name: &str) -> String {
    format!("{}.{}", quote_ident(schema), quote_ident(name))
}

pub fn is_denied(err: &postgres::Error) -> bool {
    err.code() == Some(&SqlState::INSUFFICIENT_PRIVILEGE)
}

/// A `42501` the statement raised about `schema.table` itself (§4.3): no `CONTEXT`, so not from a
/// trigger or a function, and the message names the table or its schema. Names, not wording, so any
/// `lc_messages` works.
pub fn is_denied_on(err: &postgres::Error, schema: &str, table: &str) -> bool {
    let Some(db) = err.as_db_error().filter(|_| is_denied(err)) else {
        return false;
    };
    let nested = db
        .where_()
        .is_some_and(|context| !context.trim().is_empty());
    !nested && (names(db.message(), table) || names(db.message(), schema))
}

/// `describe`, plus the outermost frame of the error's context: the trigger or function it came from.
pub fn describe_with_context(err: &postgres::Error) -> String {
    let text = describe(err);
    match err
        .as_db_error()
        .and_then(|db| db.where_())
        .and_then(|context| context.lines().rev().find(|l| !l.trim().is_empty()))
    {
        Some(frame) => format!("{text}, from {}", frame.trim()),
        None => text,
    }
}

/// Whether `word` appears in `text` with no identifier character on either side.
fn names(text: &str, word: &str) -> bool {
    let is_ident = |c: char| c.is_alphanumeric() || c == '_' || c == '$';
    !word.is_empty()
        && text.match_indices(word).any(|(at, _)| {
            let before = text[..at].chars().next_back();
            let after = text[at + word.len()..].chars().next();
            !before.is_some_and(is_ident) && !after.is_some_and(is_ident)
        })
}

pub fn describe(err: &postgres::Error) -> String {
    match err.as_db_error() {
        Some(db) => format!("{} (SQLSTATE {})", db.message(), db.code().code()),
        None => {
            let mut text = err.to_string();
            let mut source = std::error::Error::source(err);
            while let Some(inner) = source {
                text.push_str(&format!(": {inner}"));
                source = inner.source();
            }
            text
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_match_whole_words_only() {
        let rls = "new row violates row-level security policy for table \"notes\"";
        assert!(names(rls, "notes"));
        assert!(!names(rls, "note"));
        assert!(names("permission denied for table notes", "notes"));
        assert!(!names(
            "permission denied for sequence notes_id_seq",
            "notes"
        ));
        assert!(!names("permission denied for table audit_notes", "notes"));
        assert!(!names("permission denied for table notes2", "notes"));
        assert!(names("permission denied for schema app", "app"));
        assert!(names("permissão negada para tabela promoções", "promoções"));
        assert!(!names("anything", ""));
    }

    #[test]
    fn identifiers_are_quoted() {
        assert_eq!(quote_ident("app"), "\"app\"");
        assert_eq!(quote_ident("we\"ird"), "\"we\"\"ird\"");
        assert_eq!(qualified("public", "Notes"), "\"public\".\"Notes\"");
    }
}
