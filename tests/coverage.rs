mod common;

use common::Db;

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
