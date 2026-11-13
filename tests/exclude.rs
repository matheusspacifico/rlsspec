mod common;

use common::{Db, Output};

fn rlsspec(command: &str, scenario: &str, expected_code: i32) -> Output {
    let db = Db::new();
    let config = format!("tests/fixtures/exclude/{scenario}.yaml");
    let out = db.rlsspec(&[command, "-c", &config]);
    assert_eq!(
        out.code,
        Some(expected_code),
        "stdout:\n{}\nstderr:\n{}",
        out.stdout,
        out.stderr
    );
    assert_eq!(db.leftover_tables(), Vec::<String>::new());
    out
}

fn report(command: &str, scenario: &str, expected_code: i32) -> String {
    let out = rlsspec(command, scenario, expected_code);
    assert_eq!(out.stderr, "");
    // RLS007 is skipped on PostgreSQL 14 only.
    out.stdout.replace(
        "RLS007 skipped: needs PostgreSQL 15 or later (security_invoker)\n",
        "",
    )
}

#[test]
fn a_blanket_default_fails_on_a_bookkeeping_table_in_scope() {
    insta::assert_snapshot!(report("test", "in_scope", 1));
}

#[test]
fn lint_reports_a_bookkeeping_table_in_scope() {
    insta::assert_snapshot!(report("lint", "in_scope", 1));
}

#[test]
fn an_excluded_table_gets_no_case() {
    insta::assert_snapshot!(report("test", "excluded", 0));
}

#[test]
fn an_excluded_table_is_not_in_the_coverage_matrix() {
    let with = report("cover", "in_scope", 0);
    assert!(with.contains("schema_migrations"), "{with}");
    insta::assert_snapshot!(report("cover", "excluded", 0));
}

#[test]
fn an_excluded_table_is_not_linted() {
    assert_eq!(
        report("lint", "excluded", 0),
        "0 errors · 0 warnings · 0 info · 0 ignored\n"
    );
}

#[test]
fn a_lint_ignore_for_an_excluded_table_is_stale() {
    insta::assert_snapshot!(report("lint", "stale_ignore", 0));
}

#[test]
fn wrong_exclusions_and_entries_for_excluded_tables_are_located() {
    for command in ["test", "cover", "lint"] {
        let out = rlsspec(command, "errors", 2);
        assert_eq!(out.stdout, "");
        insta::assert_snapshot!("exclude_errors", out.stderr);
    }
}
