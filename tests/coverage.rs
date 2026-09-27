mod common;

use common::{Db, Output};

fn report(scenario: &str, expected_code: i32) -> String {
    let db = Db::new();
    let out = db.run(&format!("tests/fixtures/coverage/{scenario}.yaml"));
    assert_eq!(
        out.code,
        Some(expected_code),
        "stdout:\n{}\nstderr:\n{}",
        out.stdout,
        out.stderr
    );
    assert_eq!(out.stderr, "");
    assert_eq!(db.leftover_tables(), Vec::<String>::new());
    out.stdout
}

#[test]
fn warn_lists_the_gaps_and_passes() {
    insta::assert_snapshot!(report("warn", 0));
}

#[test]
fn ignore_prints_only_the_coverage_line() {
    let out = report("ignore", 0);
    assert!(!out.contains('○'), "{out}");
    insta::assert_snapshot!(out);
}

#[test]
fn todo_in_expect_beats_a_default_and_runs_nothing() {
    let out = report("warn", 0);
    let notes: Vec<&str> = out
        .lines()
        .skip_while(|l| *l != "notes")
        .take_while(|l| *l != "tags")
        .filter(|l| l.contains("alice"))
        .collect();
    assert_eq!(notes.len(), 2, "{out}");
    assert!(notes.iter().all(|l| !l.contains("update")), "{out}");
}

#[test]
fn a_table_added_in_setup_without_a_spec_fails() {
    insta::assert_snapshot!(report("fail_new_table", 1));
}

#[test]
fn full_coverage_passes_with_fail() {
    insta::assert_snapshot!(report("fail_covered", 0));
}

fn cover(scenario: &str, expected_code: i32) -> Output {
    let db = Db::new();
    let config = format!("tests/fixtures/coverage/{scenario}.yaml");
    let out = db.rlsspec(&["cover", "-c", &config]);
    assert_eq!(
        out.code,
        Some(expected_code),
        "stdout:\n{}\nstderr:\n{}",
        out.stdout,
        out.stderr
    );
    assert_eq!(out.stderr, "");
    assert_eq!(db.leftover_tables(), Vec::<String>::new());
    out
}

#[test]
fn cover_prints_the_matrix_and_passes_with_warn() {
    insta::assert_snapshot!(cover("warn", 0).stdout);
}

#[test]
fn cover_fails_on_gaps_with_fail() {
    insta::assert_snapshot!(cover("fail_new_table", 1).stdout);
}

#[test]
fn cover_passes_with_fail_when_every_cell_is_specified() {
    let out = cover("fail_covered", 0).stdout;
    assert!(
        out.ends_with("coverage 16/16 cells (100.0%) · 0 unspecified (fail)\n"),
        "{out}"
    );
}

#[test]
fn cover_runs_no_case() {
    let db = Db::new();
    let config = "tests/fixtures/writes/vacuous.yaml";
    assert_eq!(db.rlsspec(&["test", "-c", config]).code, Some(2));
    let out = db.rlsspec(&["cover", "-c", config]);
    assert_eq!(out.code, Some(0), "{}", out.stderr);
    assert!(!out.stdout.contains("vacuous"), "{}", out.stdout);
}

#[test]
fn cover_reports_config_errors_like_test() {
    let db = Db::new();
    let out = db.rlsspec(&["cover", "-c", "tests/fixtures/writes/columns.yaml"]);
    assert_eq!(out.code, Some(2));
    assert_eq!(
        out.stderr,
        db.rlsspec(&["test", "-c", "tests/fixtures/writes/columns.yaml"])
            .stderr
    );
}
