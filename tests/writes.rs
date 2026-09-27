mod common;

use common::{Db, Output};

fn run(scenario: &str, expected_code: i32) -> Output {
    let db = Db::new();
    let out = db.run(&format!("tests/fixtures/writes/{scenario}.yaml"));
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

fn report(scenario: &str, expected_code: i32) -> String {
    let out = run(scenario, expected_code);
    assert_eq!(out.stderr, "");
    out.stdout
}

#[test]
fn insert_pass() {
    insta::assert_snapshot!(report("insert_pass", 0));
}

#[test]
fn insert_check_true_lets_alice_write_into_the_other_tenant() {
    insta::assert_snapshot!(report("insert_check_true", 1));
}

#[test]
fn insert_without_a_policy_denies_the_allowed_row() {
    insta::assert_snapshot!(report("insert_no_policy", 1));
}

#[test]
fn update_pass() {
    insta::assert_snapshot!(report("update_pass", 0));
}

#[test]
fn update_using_true_lets_alice_edit_the_other_tenant() {
    insta::assert_snapshot!(report("update_using_true", 1));
}

#[test]
fn update_check_true_lets_alice_move_rows_to_the_other_tenant() {
    insta::assert_snapshot!(report("update_check_true", 1));
}

#[test]
fn update_author_only_is_a_partial_update() {
    insta::assert_snapshot!(report("update_author_only", 1));
}

#[test]
fn delete_pass() {
    insta::assert_snapshot!(report("delete_pass", 0));
}

#[test]
fn delete_using_true_lets_alice_delete_the_other_tenant() {
    insta::assert_snapshot!(report("delete_using_true", 1));
}

#[test]
fn vacuous_writes_are_inconclusive() {
    insta::assert_snapshot!(report("vacuous", 2));
}

#[test]
fn not_null_violation_is_inconclusive() {
    insta::assert_snapshot!(report("not_null", 2));
}

#[test]
fn unknown_and_generated_columns_are_located() {
    let out = run("columns", 2);
    assert_eq!(out.stdout, "");
    insta::assert_snapshot!(out.stderr);
}

#[test]
fn insert_deny_pass_without_a_privilege_or_a_policy() {
    insta::assert_snapshot!(report("insert_deny_pass", 0));
}

#[test]
fn insert_deny_fails_when_a_public_permissive_policy_applies() {
    insta::assert_snapshot!(report("insert_deny_public_policy", 1));
}

#[test]
fn defaults_expand_to_every_table_and_yield_to_specific_entries() {
    insta::assert_snapshot!(report("defaults_pass", 0));
}

#[test]
fn defaults_catch_a_catch_all_guest_policy() {
    insta::assert_snapshot!(report("defaults_guest_all_true", 1));
}

#[test]
fn two_blocks_for_the_same_table_and_identity_are_located() {
    let out = run("duplicate", 2);
    assert_eq!(out.stdout, "");
    insta::assert_snapshot!(out.stderr);
}

#[test]
fn only_a_constant_update_over_every_row_sees_past_the_select_policies() {
    insta::assert_snapshot!(report("update_hidden_by_select", 1));
}

#[test]
fn insert_deny_ignores_policies_whose_check_is_false() {
    insta::assert_snapshot!(report("insert_deny_check_false", 0));
}

#[test]
fn audit_trigger_behind_a_correct_policy_passes() {
    insta::assert_snapshot!(report("audited_pass", 0));
}

#[test]
fn a_denied_audit_trigger_is_not_the_policy_denying_the_write() {
    insta::assert_snapshot!(report("audited_broken", 2));
}
