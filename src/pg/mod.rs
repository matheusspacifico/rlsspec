pub mod script;

use std::io;
use std::path::{Path, PathBuf};

use postgres::error::SqlState;
use postgres::{Client, IsolationLevel, NoTls, Transaction};

use crate::config::{ConfigError, Safety, Source};

const MIN_SERVER_VERSION: i32 = 140000;

#[derive(Debug, thiserror::Error)]
pub enum PgError {
    #[error("cannot connect to the database: {}", describe(.0))]
    Connect(#[source] postgres::Error),
    #[error("PostgreSQL {0} is not supported; rlsspec needs PostgreSQL 14 or later")]
    UnsupportedVersion(String),
    #[error("database error: {}", describe(.0))]
    Query(#[from] postgres::Error),
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

pub fn connect(url: &str) -> Result<Client, PgError> {
    let mut client = Client::connect(url, NoTls).map_err(PgError::Connect)?;
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
        self.tx.batch_execute("RESET ROLE")?;
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
    fn identifiers_are_quoted() {
        assert_eq!(quote_ident("app"), "\"app\"");
        assert_eq!(quote_ident("we\"ird"), "\"we\"\"ird\"");
        assert_eq!(qualified("public", "Notes"), "\"public\".\"Notes\"");
    }
}
