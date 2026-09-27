mod common;

use common::Db;
use serde_json::Value;

const SKIPPED_RLS007: &str = r#"  "skipped": [
    {
      "rule": "RLS007",
      "reason": "needs PostgreSQL 15 or later (security_invoker)"
    }
  ],"#;
const SKIPPED_NONE: &str = r#"  "skipped": [],"#;

/// Runs `args` against a fresh database and checks what every machine-readable report shares:
/// the exit code, nothing on stderr, no ANSI escapes, nothing left behind.
fn run(db: &Db, args: &[&str], expected_code: i32) -> String {
    let out = db.rlsspec(args);
    assert_eq!(
        out.code,
        Some(expected_code),
        "stdout:\n{}\nstderr:\n{}",
        out.stdout,
        out.stderr
    );
    assert_eq!(out.stderr, "");
    assert!(!out.stdout.contains('\x1b'), "{:?}", out.stdout);
    assert_eq!(db.leftover_tables(), Vec::<String>::new());
    out.stdout
}

/// The JSON report of `args`, parsed back to check its version and that it states the exit code.
fn json(args: &[&str], expected_code: i32) -> String {
    let db = Db::new();
    let mut args = args.to_vec();
    args.extend(["--format", "json"]);
    let out = run(&db, &args, expected_code);
    assert!(
        out.starts_with("{\n  \"schema_version\": 1,\n"),
        "schema_version must come first:\n{out}"
    );
    let value: Value = serde_json::from_str(&out).unwrap();
    assert_eq!(value["schema_version"], 1);
    assert_eq!(value["exit_code"], expected_code);
    out
}

#[test]
fn json_test_all_pass() {
    let out = json(
        &["test", "-c", "tests/fixtures/coverage/fail_covered.yaml"],
        0,
    );
    let value: Value = serde_json::from_str(&out).unwrap();
    let cases = value["cases"].as_array().unwrap();
    assert!(cases.iter().all(|c| c["outcome"] == "pass"), "{out}");
    assert_eq!(value["totals"]["passed"], cases.len());
    insta::assert_snapshot!(out);
}

#[test]
fn json_test_lists_every_case_unfolded_with_its_location() {
    let out = json(&["test", "-c", "tests/fixtures/format/mixed.yaml"], 2);
    let value: Value = serde_json::from_str(&out).unwrap();
    let outcomes = |origin: &str| {
        value["cases"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|c| c["origin"] == origin)
            .count()
    };
    // guest's four ops on two tables, one case each: text folds them into one line per table.
    assert_eq!(outcomes("default"), 8, "{out}");
    assert_eq!(value["totals"]["failed"], 2, "{out}");
    assert_eq!(value["totals"]["inconclusive"], 1, "{out}");
    insta::assert_snapshot!(out);
}

#[test]
fn json_test_lists_coverage_gaps_under_fail() {
    let out = json(
        &["test", "-c", "tests/fixtures/coverage/fail_new_table.yaml"],
        1,
    );
    insta::assert_snapshot!(out);
}

#[test]
fn json_cover_prints_the_matrix() {
    let out = json(
        &["cover", "-c", "tests/fixtures/coverage/fail_new_table.yaml"],
        1,
    );
    insta::assert_snapshot!(out);
}

#[test]
fn json_lint_reports_findings_ignored_and_stale_entries() {
    let db = Db::new();
    let out = run(
        &db,
        &[
            "lint",
            "-c",
            "tests/fixtures/format/lint.yaml",
            "--format",
            "json",
        ],
        1,
    );
    let value: Value = serde_json::from_str(&out).unwrap();
    assert_eq!(value["schema_version"], 1);
    assert_eq!(value["exit_code"], 1);
    // RLS007 is skipped on PostgreSQL 14 only; the rest of the document is the same on every version.
    let skipped = db.server_version() < 150000;
    assert_eq!(out.contains(SKIPPED_RLS007), skipped, "{out}");
    assert_eq!(out.contains(SKIPPED_NONE), !skipped, "{out}");
    insta::assert_snapshot!(out.replace(SKIPPED_RLS007, SKIPPED_NONE));
}
