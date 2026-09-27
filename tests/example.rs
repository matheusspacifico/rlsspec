mod common;

use std::fs;
use std::sync::Mutex;

use common::Db;

const EXAMPLE: &str = "examples/multitenant";
const TABLES: [&str; 4] = ["organizations", "memberships", "projects", "tasks"];

// Roles are cluster-wide: two tests creating them at once would race past the `if not exists`.
static SCHEMA: Mutex<()> = Mutex::new(());

/// The example's schema and policies, applied the way docker compose does: committed, before rlsspec runs.
fn example_db(extra: &[&str]) -> Db {
    let db = Db::new();
    let _guard = SCHEMA.lock().unwrap_or_else(|e| e.into_inner());
    for file in ["schema.sql", "policies.sql"].iter().chain(extra) {
        db.execute(&fs::read_to_string(format!("{EXAMPLE}/{file}")).unwrap());
    }
    db
}

fn run(db: &Db, expected_code: i32) -> String {
    let out = db.run(&format!("{EXAMPLE}/rlsspec.yaml"));
    assert_eq!(
        out.code,
        Some(expected_code),
        "stdout:\n{}\nstderr:\n{}",
        out.stdout,
        out.stderr
    );
    assert_eq!(out.stderr, "");
    for table in TABLES {
        assert_eq!(db.count(table), 0, "seed rows left in {table}");
    }
    out.stdout
}

#[test]
fn multitenant_example_is_green() {
    let db = example_db(&[]);
    insta::assert_snapshot!(run(&db, 0));
}

#[test]
fn multitenant_example_catches_the_broken_delete_policy() {
    let db = example_db(&["broken.sql"]);
    let out = run(&db, 1);
    let failures: Vec<&str> = out.lines().filter(|l| l.contains('✗')).collect();
    assert_eq!(failures.len(), 1, "{out}");
    insta::assert_snapshot!(out);
}

#[test]
fn multitenant_example_is_fully_covered() {
    let db = example_db(&[]);
    let out = db.rlsspec(&["cover", "-c", &format!("{EXAMPLE}/rlsspec.yaml")]);
    assert_eq!(out.code, Some(0), "{}", out.stderr);
    assert_eq!(out.stderr, "");
    insta::assert_snapshot!(out.stdout);
}
