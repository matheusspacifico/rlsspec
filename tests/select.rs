mod common;

use common::Db;

fn run(scenario: &str, expected_code: i32) -> String {
    let db = Db::new();
    let out = db.run(&format!("tests/fixtures/select/{scenario}.yaml"));
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
fn rows_pass() {
    insta::assert_snapshot!(run("rows_pass", 0));
}

#[test]
fn rows_using_true_leaks_the_other_tenant() {
    insta::assert_snapshot!(run("rows_using_true", 1));
}

#[test]
fn rows_wrong_column_leaks_and_hides() {
    insta::assert_snapshot!(run("rows_wrong_column", 1));
}

#[test]
fn rows_permissive_policy_widens_the_tenant_policy() {
    insta::assert_snapshot!(run("rows_permissive_widens", 1));
}

#[test]
fn subset_pass_when_fewer_rows_are_visible() {
    insta::assert_snapshot!(run("subset_pass", 0));
}

#[test]
fn subset_using_true_leaks() {
    insta::assert_snapshot!(run("subset_using_true", 1));
}

#[test]
fn all_pass() {
    insta::assert_snapshot!(run("all_pass", 0));
}

#[test]
fn all_too_strict_hides_rows() {
    insta::assert_snapshot!(run("all_too_strict", 1));
}

#[test]
fn deny_pass_without_a_policy() {
    insta::assert_snapshot!(run("deny_no_policy", 0));
}

#[test]
fn deny_pass_without_a_grant() {
    insta::assert_snapshot!(run("deny_no_grant", 0));
}

#[test]
fn deny_using_true_leaks_everything() {
    insta::assert_snapshot!(run("deny_using_true", 1));
}

#[test]
fn deny_public_permissive_policy_leaks_published_rows() {
    insta::assert_snapshot!(run("deny_permissive_widens", 1));
}
