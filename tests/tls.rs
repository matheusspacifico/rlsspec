mod common;

use std::env;
use std::fs;
use std::path::PathBuf;
use std::process::Command;
use std::sync::{Arc, Mutex, Weak};

use common::{Db, Output};
use postgres::{Client, NoTls};
use testcontainers_modules::postgres::Postgres;
use testcontainers_modules::testcontainers::core::ExecCommand;
use testcontainers_modules::testcontainers::runners::SyncRunner;
use testcontainers_modules::testcontainers::{Container, ImageExt};

/// A CA and a server certificate for db.rlsspec.test, localhost and 127.0.0.1, generated when the
/// container starts, then a second CA that signed nothing. The server accepts TLS and plain sessions.
const START: &str = r#"set -e
mkdir /tls && cd /tls
openssl req -x509 -newkey rsa:2048 -nodes -days 2 -subj "/CN=rlsspec test CA" -keyout ca.key -out ca.pem 2>/dev/null
openssl req -newkey rsa:2048 -nodes -subj "/CN=db.rlsspec.test" -keyout server.key -out server.csr 2>/dev/null
echo "subjectAltName=DNS:db.rlsspec.test,DNS:localhost,IP:127.0.0.1" > san.ext
openssl x509 -req -in server.csr -CA ca.pem -CAkey ca.key -CAcreateserial -days 2 -extfile san.ext -out server.pem 2>/dev/null
openssl req -x509 -newkey rsa:2048 -nodes -days 2 -subj "/CN=another CA" -keyout other.key -out other.pem 2>/dev/null
chown postgres server.key server.pem && chmod 600 server.key
exec docker-entrypoint.sh postgres -c ssl=on -c ssl_cert_file=/tls/server.pem -c ssl_key_file=/tls/server.key
"#;

const ENCRYPTED: &str = "tests/fixtures/tls/encrypted.yaml";
const PLAIN: &str = "tests/fixtures/tls/plain.yaml";

static SERVER: Mutex<Weak<Server>> = Mutex::new(Weak::new());

struct Server {
    _container: Container<Postgres>,
    port: u16,
    ca: PathBuf,
    other_ca: PathBuf,
}

fn server() -> Arc<Server> {
    let mut shared = SERVER.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(server) = shared.upgrade() {
        return server;
    }
    let tag = env::var("RLSSPEC_PG_TAG").unwrap_or_else(|_| "17".into());
    let container = Postgres::default()
        .with_tag(tag)
        .with_cmd(["sh", "-c", START])
        .start()
        .unwrap();
    let port = container.get_host_port_ipv4(5432).unwrap();
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("tls");
    fs::create_dir_all(&dir).unwrap();
    let copy = |name: &str| {
        let pem = container
            .exec(ExecCommand::new(["cat", &format!("/tls/{name}")]))
            .unwrap()
            .stdout_to_vec()
            .unwrap();
        let path = dir.join(name);
        fs::write(&path, pem).unwrap();
        path
    };
    let (ca, other_ca) = (copy("ca.pem"), copy("other.pem"));
    Client::connect(
        &format!("postgres://postgres:postgres@127.0.0.1:{port}/postgres"),
        NoTls,
    )
    .unwrap()
    .batch_execute("CREATE ROLE app")
    .unwrap();
    let server = Arc::new(Server {
        _container: container,
        port,
        ca,
        other_ca,
    });
    *shared = Arc::downgrade(&server);
    server
}

impl Server {
    /// A URL for `host` (reached through hostaddr=127.0.0.1 when it isn't an address) with `params`.
    fn url(&self, host: &str, params: &str) -> String {
        let port = self.port;
        let mut url = format!("postgres://postgres:postgres@{host}:{port}/postgres?");
        if host.parse::<std::net::IpAddr>().is_err() && host != "localhost" {
            url.push_str("hostaddr=127.0.0.1&");
        }
        url.push_str(params);
        url.trim_end_matches(['?', '&']).to_owned()
    }

    fn cover(&self, config: &str, url: &str, flags: &[&str]) -> Output {
        let output = Command::new(env!("CARGO_BIN_EXE_rlsspec"))
            .args(["cover", "-c", config])
            .args(flags)
            .env("DATABASE_URL", url)
            .current_dir(env!("CARGO_MANIFEST_DIR"))
            .output()
            .unwrap();
        Output {
            code: output.status.code(),
            stdout: String::from_utf8(output.stdout).unwrap(),
            stderr: String::from_utf8(output.stderr).unwrap(),
        }
    }
}

fn connects(out: &Output) {
    assert_eq!(
        out.code,
        Some(0),
        "stdout:\n{}\nstderr:\n{}",
        out.stdout,
        out.stderr
    );
    assert!(out.stdout.contains("coverage 4/4 cells"), "{}", out.stdout);
}

/// A connection error, after any warning the safety guard printed.
fn refused_connection(out: &Output, reason: &str) {
    assert_eq!(out.code, Some(2), "{}", out.stdout);
    assert_eq!(out.stdout, "");
    let error = out
        .stderr
        .lines()
        .find(|line| !line.starts_with("warning: "))
        .unwrap_or_default();
    assert!(
        error.starts_with("error: cannot connect to the database: "),
        "{}",
        out.stderr
    );
    assert!(error.contains(reason), "{}", out.stderr);
}

#[test]
fn prefer_encrypts_when_the_server_supports_tls() {
    let server = server();
    connects(&server.cover(ENCRYPTED, &server.url("127.0.0.1", ""), &[]));
    let url = server.url("127.0.0.1", "sslmode=prefer");
    connects(&server.cover(ENCRYPTED, &url, &[]));
}

#[test]
fn require_encrypts_without_verifying_the_certificate() {
    let server = server();
    let url = server.url("127.0.0.1", "sslmode=require");
    connects(&server.cover(ENCRYPTED, &url, &[]));
}

#[test]
fn disable_stays_plain() {
    let server = server();
    let url = server.url("127.0.0.1", "sslmode=disable");
    let out = server.cover(PLAIN, &url, &[]);
    connects(&out);
    assert_eq!(out.stderr, "");
}

#[test]
fn verify_full_accepts_the_right_ca_and_host() {
    let server = server();
    let ca = format!("sslrootcert={}", server.ca.display());
    for host in ["127.0.0.1", "localhost", "db.rlsspec.test"] {
        let url = server.url(host, &format!("sslmode=verify-full&{ca}"));
        connects(&server.cover(ENCRYPTED, &url, &[]));
    }
    // The key=value form takes the same parameters.
    let url = format!(
        "host=localhost port={} user=postgres password=postgres dbname=postgres sslmode=verify-full sslrootcert='{}'",
        server.port,
        server.ca.display()
    );
    connects(&server.cover(ENCRYPTED, &url, &[]));
}

#[test]
fn verify_full_rejects_the_wrong_ca() {
    let server = server();
    let other = format!("sslrootcert={}", server.other_ca.display());
    let url = server.url("127.0.0.1", &format!("sslmode=verify-full&{other}"));
    refused_connection(&server.cover(ENCRYPTED, &url, &[]), "UnknownIssuer");
    // Without sslrootcert, the default roots (Mozilla's and the system's) don't know the test CA.
    let url = server.url("127.0.0.1", "sslmode=verify-full");
    refused_connection(&server.cover(ENCRYPTED, &url, &[]), "UnknownIssuer");
    let url = server.url("127.0.0.1", &format!("sslmode=verify-ca&{other}"));
    refused_connection(&server.cover(ENCRYPTED, &url, &[]), "UnknownIssuer");
}

#[test]
fn verify_ca_checks_the_chain_but_not_the_host_name() {
    let server = server();
    let ca = format!("sslrootcert={}", server.ca.display());
    let host = "elsewhere.rlsspec.test";
    let url = server.url(host, &format!("sslmode=verify-ca&{ca}"));
    connects(&server.cover(ENCRYPTED, &url, &["--allow-remote"]));
    let url = server.url(host, &format!("sslmode=verify-full&{ca}"));
    refused_connection(
        &server.cover(ENCRYPTED, &url, &["--allow-remote"]),
        "certificate not valid for name \"elsewhere.rlsspec.test\"",
    );
}

#[test]
fn unsupported_tls_settings_are_config_errors() {
    let server = server();
    let error = |params: &str| {
        let out = server.cover(ENCRYPTED, &server.url("127.0.0.1", params), &[]);
        assert_eq!(out.code, Some(2), "{}", out.stdout);
        assert_eq!(out.stdout, "");
        out.stderr
    };
    assert_eq!(
        error("sslmode=allow"),
        "error: `sslmode=allow` is not supported; use disable, prefer, require, verify-ca or verify-full\n"
    );
    assert_eq!(
        error("sslmode=require&sslrootcert=ca.pem"),
        "error: `sslrootcert` only applies to sslmode=verify-ca or verify-full, not require\n"
    );
    assert_eq!(
        error("sslmode=verify-full&sslrootcert=tests/fixtures/tls/missing.pem"),
        "error: cannot read sslrootcert tests/fixtures/tls/missing.pem: I/O error: No such file or directory (os error 2)\n"
    );
    assert_eq!(
        error("sslmode=verify-full&sslrootcert=tests/fixtures/tls/encrypted.sql"),
        "error: no certificate found in sslrootcert tests/fixtures/tls/encrypted.sql\n"
    );
}

// The remote-host policy, exercised through `safety.allowed_hosts`: the fixtures list db.rlsspec.test,
// which the URLs point at 127.0.0.1 with `hostaddr`, so the guard sees a non-local host.

#[test]
fn a_listed_remote_host_is_refused_without_tls() {
    let server = server();
    let url = server.url("db.rlsspec.test", "sslmode=disable");
    let out = server.cover(PLAIN, &url, &[]);
    assert_eq!(out.code, Some(2));
    assert_eq!(out.stdout, "");
    assert_eq!(
        out.stderr,
        "error: refusing to connect to non-local host `db.rlsspec.test` without TLS (sslmode=disable); use sslmode=require or stronger, or pass --allow-insecure\n"
    );

    let out = server.cover(PLAIN, &url, &["--allow-insecure"]);
    connects(&out);
    assert_eq!(
        out.stderr,
        "warning: connecting to non-local host `db.rlsspec.test` without TLS (--allow-insecure)\n"
    );

    let out = server.cover(
        ENCRYPTED,
        &server.url("db.rlsspec.test", "sslmode=require"),
        &[],
    );
    connects(&out);
    assert_eq!(out.stderr, "");
}

#[test]
fn prefer_becomes_require_for_a_remote_host() {
    // The shared test server has no TLS: a local host falls back to plain, a remote one must not.
    let db = Db::new();
    let local = db.url("postgres", "postgres");
    let out = db.rlsspec_with_url(&["cover", "-c", PLAIN], &local);
    connects(&out);
    assert_eq!(out.stderr, "");

    let remote = format!(
        "{}?hostaddr=127.0.0.1",
        local.replace("@127.0.0.1:", "@db.rlsspec.test:")
    );
    refused_connection(
        &db.rlsspec_with_url(&["cover", "-c", PLAIN], &remote),
        "server does not support TLS",
    );

    let out = db.rlsspec_with_url(&["cover", "-c", PLAIN, "--allow-insecure"], &remote);
    connects(&out);
    assert_eq!(
        out.stderr,
        "warning: connecting to non-local host `db.rlsspec.test` without TLS if it doesn't offer it (sslmode=prefer, --allow-insecure)\n"
    );
}

#[test]
fn allow_remote_warns_with_the_host_only() {
    let server = server();
    let ca = format!("sslrootcert={}", server.ca.display());
    let url = server
        .url("localhost", &format!("sslmode=verify-ca&{ca}"))
        .replace("@localhost:", "@elsewhere.rlsspec.test:")
        .replace("/postgres?", "/postgres?hostaddr=127.0.0.1&");
    let out = server.cover(ENCRYPTED, &url, &["--allow-remote"]);
    connects(&out);
    assert_eq!(
        out.stderr,
        "warning: connecting to non-local host `elsewhere.rlsspec.test` (--allow-remote)\n"
    );
}
