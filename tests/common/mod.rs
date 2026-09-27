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
        self.rlsspec_with_url(&["test", "-c", config], database_url)
    }

    /// Runs the binary with `args`, `DATABASE_URL` pointing at this database as the superuser.
    pub fn rlsspec(&self, args: &[&str]) -> Output {
        self.rlsspec_with_url(args, &self.url("postgres", "postgres"))
    }

    pub fn rlsspec_with_url(&self, args: &[&str], database_url: &str) -> Output {
        let output = Command::new(env!("CARGO_BIN_EXE_rlsspec"))
            .args(args)
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

    /// Runs `sql` as the superuser, committed: for schemas that exist before rlsspec runs.
    pub fn execute(&self, sql: &str) {
        connect(&self.url("postgres", "postgres"))
            .batch_execute(sql)
            .unwrap();
    }

    pub fn count(&self, table: &str) -> i64 {
        connect(&self.url("postgres", "postgres"))
            .query_one(&format!("SELECT count(*) FROM {table}"), &[])
            .unwrap()
            .get(0)
    }

    pub fn server_version(&self) -> i32 {
        connect(&self.url("postgres", "postgres"))
            .query_one("SELECT current_setting('server_version_num')::int", &[])
            .unwrap()
            .get(0)
    }

    pub fn role_exists(&self, role: &str) -> bool {
        connect(&self.url("postgres", "postgres"))
            .query_one(
                "SELECT exists (SELECT FROM pg_roles WHERE rolname = $1)",
                &[&role],
            )
            .unwrap()
            .get(0)
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
