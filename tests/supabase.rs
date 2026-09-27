//! The supabase-todo example on the real `supabase/postgres` image: slow to pull and start, so these
//! only run with `cargo test --test supabase -- --ignored` (a separate CI job).

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use postgres::{Client, NoTls};
use rlsspec::config::{self, Spec};
use testcontainers_modules::testcontainers::core::{Healthcheck, IntoContainerPort, WaitFor};
use testcontainers_modules::testcontainers::runners::SyncRunner;
use testcontainers_modules::testcontainers::{Container, GenericImage, ImageExt};

const IMAGE: &str = "supabase/postgres";
// Keep in step with examples/supabase-todo/docker-compose.yml.
const TAG: &str = "17.6.1.143";
const EXAMPLE: &str = "examples/supabase-todo";
const TABLES: [&str; 3] = ["lists", "todos", "shares"];

struct Supabase {
    _container: Container<GenericImage>,
    url: String,
}

/// A fresh container per test: the image's background workers stay connected to its only database
/// with the auth schema, so it can't be cloned as a template, and the broken variant changes a policy.
fn supabase(extra: &[&str]) -> Supabase {
    let container = GenericImage::new(IMAGE, TAG)
        .with_exposed_port(5432.tcp())
        .with_wait_for(WaitFor::healthcheck())
        .with_env_var("POSTGRES_PASSWORD", "postgres")
        // Over TCP: during initialisation the server only listens on its unix socket.
        .with_health_check(
            Healthcheck::cmd(["pg_isready", "-h", "127.0.0.1", "-U", "postgres"])
                .with_interval(Duration::from_secs(1))
                .with_timeout(Duration::from_secs(3))
                .with_retries(120),
        )
        .with_startup_timeout(Duration::from_secs(180))
        .start()
        .unwrap();
    let port = container.get_host_port_ipv4(5432).unwrap();
    let url = format!("postgres://postgres:postgres@127.0.0.1:{port}/postgres");
    let mut client = Client::connect(&url, NoTls).unwrap();
    for file in ["schema.sql", "policies.sql"].iter().chain(extra) {
        let sql = fs::read_to_string(format!("{EXAMPLE}/{file}")).unwrap();
        client.batch_execute(&sql).unwrap();
    }
    Supabase {
        _container: container,
        url,
    }
}

struct Output {
    code: Option<i32>,
    stdout: String,
    stderr: String,
}

impl Supabase {
    fn rlsspec(&self, args: &[&str]) -> Output {
        let output = Command::new(env!("CARGO_BIN_EXE_rlsspec"))
            .args(args)
            .env("DATABASE_URL", &self.url)
            .current_dir(env!("CARGO_MANIFEST_DIR"))
            .output()
            .unwrap();
        Output {
            code: output.status.code(),
            stdout: String::from_utf8(output.stdout).unwrap(),
            stderr: String::from_utf8(output.stderr).unwrap(),
        }
    }

    fn run(&self, expected_code: i32) -> String {
        let out = self.rlsspec(&["test", "-c", &format!("{EXAMPLE}/rlsspec.yaml")]);
        assert_eq!(
            out.code,
            Some(expected_code),
            "stdout:\n{}\nstderr:\n{}",
            out.stdout,
            out.stderr
        );
        assert_eq!(out.stderr, "");
        let mut client = Client::connect(&self.url, NoTls).unwrap();
        for table in TABLES {
            let count: i64 = client
                .query_one(&format!("SELECT count(*) FROM {table}"), &[])
                .unwrap()
                .get(0);
            assert_eq!(count, 0, "seed rows left in {table}");
        }
        out.stdout
    }
}

#[test]
#[ignore = "needs the supabase/postgres image"]
fn supabase_todo_example_is_green_and_fully_covered() {
    let db = supabase(&[]);
    insta::assert_snapshot!("supabase_todo_example_is_green", db.run(0));

    let out = db.rlsspec(&["cover", "-c", &format!("{EXAMPLE}/rlsspec.yaml")]);
    assert_eq!(out.code, Some(0), "{}", out.stderr);
    assert_eq!(out.stderr, "");
    insta::assert_snapshot!("supabase_todo_example_is_fully_covered", out.stdout);
}

#[test]
#[ignore = "needs the supabase/postgres image"]
fn supabase_todo_example_catches_the_broken_delete_policy() {
    let db = supabase(&["broken.sql"]);
    let out = db.run(1);
    let failures: Vec<&str> = out.lines().filter(|l| l.contains('✗')).collect();
    assert_eq!(failures.len(), 1, "{out}");
    insta::assert_snapshot!(out);
}

fn scratch(test: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(test);
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

#[test]
#[ignore = "needs the supabase/postgres image"]
fn init_with_the_supabase_preset_scaffolds_the_example_with_every_cell_todo() {
    let db = supabase(&[]);
    let spec = scratch("init_supabase").join("rlsspec.yaml");
    let spec_arg = spec.to_str().unwrap();

    let out = db.rlsspec(&["init", "--preset", "supabase", "-o", spec_arg]);
    assert_eq!(out.code, Some(0), "{}", out.stderr);
    assert_eq!(out.stderr, "");
    assert_eq!(
        out.stdout,
        format!("wrote {spec_arg}: 3 tables × 2 identities, 24 cells todo\n")
    );
    let text = fs::read_to_string(&spec).unwrap();
    insta::assert_snapshot!(text);

    let env = |name: &str| (name == "DATABASE_URL").then(|| db.url.clone());
    let config = config::parse(&text, &spec, &env).unwrap();
    let roles: Vec<_> = config.identities.iter().map(|i| i.role.as_str()).collect();
    assert_eq!(roles, ["anon", "authenticated"]);
    assert_eq!(config.expect.len(), 6);
    for block in &config.expect {
        let ops = &block.ops;
        assert_eq!(ops.select, Some(Spec::Todo));
        assert_eq!(ops.insert, Some(Spec::Todo));
        assert_eq!(ops.update, Some(Spec::Todo));
        assert_eq!(ops.delete, Some(Spec::Todo));
    }

    let test = db.rlsspec(&["test", "-c", spec_arg]);
    assert_eq!(test.code, Some(0), "{}", test.stderr);
    assert!(
        test.stdout
            .starts_with("coverage 0/24 cells (0.0%) · 24 unspecified (warn)\n"),
        "{}",
        test.stdout
    );
    assert!(
        test.stdout
            .ends_with("0 failed · 0 passed · 0 inconclusive\n")
    );
    assert_eq!(test.stdout.matches('○').count(), 6, "{}", test.stdout);
}
