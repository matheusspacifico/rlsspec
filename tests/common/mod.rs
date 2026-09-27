#![allow(dead_code)]

use std::env;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, Weak};

use postgres::{Client, NoTls};
use testcontainers_modules::postgres::Postgres;
use testcontainers_modules::testcontainers::runners::SyncRunner;
use testcontainers_modules::testcontainers::{Container, ImageExt};

const ROLES: &str = "
CREATE ROLE app;
CREATE ROLE web_anon;
CREATE ROLE owner_login LOGIN PASSWORD 'owner';
GRANT app, web_anon TO owner_login;
";

// Shared by the tests of one binary; the container is removed when the last `Db` is dropped.
static SERVER: Mutex<Weak<Server>> = Mutex::new(Weak::new());
static NEXT_DB: AtomicUsize = AtomicUsize::new(0);

struct Server {
    _container: Container<Postgres>,
    port: u16,
}

fn server() -> Arc<Server> {
    let mut shared = SERVER.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(server) = shared.upgrade() {
        return server;
    }
    let tag = env::var("RLSSPEC_PG_TAG").unwrap_or_else(|_| "17".into());
    let container = Postgres::default().with_tag(tag).start().unwrap();
    let port = container.get_host_port_ipv4(5432).unwrap();
    let server = Arc::new(Server {
        _container: container,
        port,
    });
    connect(&url(port, "postgres", "postgres", "postgres"))
        .batch_execute(ROLES)
        .unwrap();
    *shared = Arc::downgrade(&server);
    server
}

fn url(port: u16, user: &str, password: &str, db: &str) -> String {
    format!("postgres://{user}:{password}@127.0.0.1:{port}/{db}")
}

fn connect(url: &str) -> Client {
    Client::connect(url, NoTls).unwrap()
}

pub struct Output {
    pub code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
}

/// A fresh database per test, so parallel tests never see each other's (rolled back) tables.
pub struct Db {
    server: Arc<Server>,
    name: String,
}

impl Db {
    pub fn new() -> Self {
        let server = server();
        let name = format!("t{}", NEXT_DB.fetch_add(1, Ordering::Relaxed));
        let mut admin = connect(&url(server.port, "postgres", "postgres", "postgres"));
        admin
            .batch_execute(&format!("CREATE DATABASE {name}"))
            .unwrap();
        connect(&url(server.port, "postgres", "postgres", &name))
            .batch_execute("GRANT CREATE ON SCHEMA public TO owner_login")
            .unwrap();
        Self { server, name }
    }

    pub fn url(&self, user: &str, password: &str) -> String {
        url(self.server.port, user, password, &self.name)
    }

    pub fn run(&self, config: &str) -> Output {
        self.run_with_url(config, &self.url("postgres", "postgres"))
    }

    pub fn run_with_url(&self, config: &str, database_url: &str) -> Output {
        let output = Command::new(env!("CARGO_BIN_EXE_rlsspec"))
            .args(["test", "-c", config])
            .env("DATABASE_URL", database_url)
            .current_dir(env!("CARGO_MANIFEST_DIR"))
            .output()
            .unwrap();
        Output {
            code: output.status.code(),
            stdout: String::from_utf8(output.stdout).unwrap(),
            stderr: String::from_utf8(output.stderr).unwrap(),
        }
    }

    /// Relations left behind in the test database. Everything `setup` creates must be rolled back.
    pub fn leftover_tables(&self) -> Vec<String> {
        connect(&self.url("postgres", "postgres"))
            .query(
                "SELECT n.nspname || '.' || c.relname FROM pg_class c
                 JOIN pg_namespace n ON n.oid = c.relnamespace
                 WHERE n.nspname NOT IN ('pg_catalog', 'information_schema', 'pg_toast')
                 ORDER BY 1",
                &[],
            )
            .unwrap()
            .iter()
            .map(|row| row.get(0))
            .collect()
    }
}
