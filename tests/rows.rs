mod common;

use common::Db;

fn run(scenario: &str, expected_code: i32) -> String {
    let db = Db::new();
    let out = db.run(&format!("tests/fixtures/rows/{scenario}.yaml"));
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
fn tenant_policies_pass_in_both_forms() {
    insta::assert_snapshot!(run("pass", 0));
}

#[test]
fn an_extra_permissive_policy_is_caught_where_the_where_cases_pass() {
    let out = run("extra_permissive", 1);
    let marks: Vec<char> = out
        .lines()
        .filter_map(|l| l.trim().chars().next())
        .collect();
    assert_eq!(marks[1..5], ['✓', '✗', '✓', '✗'], "{out}");
    insta::assert_snapshot!(out);
}

#[test]
fn a_too_narrow_policy_reports_the_untouched_rows() {
    insta::assert_snapshot!(run("too_narrow", 1));
}

#[test]
fn subset_accepts_fewer_rows() {
    insta::assert_snapshot!(run("subset_pass", 0));
}

#[test]
fn subset_still_fails_on_a_row_outside_the_predicate() {
    insta::assert_snapshot!(run("subset_leak", 1));
}

#[test]
fn a_trigger_changing_another_row_of_the_table_is_inconclusive() {
    let out = run("trigger", 2);
    assert!(
        out.contains("the statement affected 3 rows, but 4 rows of notes changed"),
        "{out}"
    );
    insta::assert_snapshot!(out);
}

#[test]
fn a_cascade_deleting_another_row_of_the_table_is_inconclusive() {
    let out = run("cascade", 2);
    assert_eq!(
        out.matches("the statement affected 3 rows, but 4 rows of notes changed")
            .count(),
        2,
        "{out}"
    );
    insta::assert_snapshot!(out);
}

#[test]
fn a_trigger_writing_to_another_table_does_not_get_in_the_way() {
    insta::assert_snapshot!(run("audited", 0));
}

#[test]
fn a_permission_error_from_a_trigger_is_inconclusive() {
    insta::assert_snapshot!(run("audited_broken", 2));
}

#[test]
fn cases_that_cannot_fail_are_inconclusive() {
    insta::assert_snapshot!(run("vacuous", 2));
}

#[test]
fn a_permission_error_on_the_target_means_no_row_and_other_errors_are_inconclusive() {
    insta::assert_snapshot!(run("errors", 2));
}

#[test]
fn partitioned_table_passes() {
    insta::assert_snapshot!(run("partitioned_pass", 0));
}

#[test]
fn partitioned_table_names_the_rows_of_the_other_partition() {
    insta::assert_snapshot!(run("partitioned_broken", 1));
}
