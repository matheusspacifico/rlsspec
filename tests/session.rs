mod common;

use common::{Db, Output};

fn run(db: &Db, scenario: &str, database_url: Option<String>) -> Output {
    let config = format!("tests/fixtures/session/{scenario}.yaml");
    let out = match database_url {
        Some(url) => db.run_with_url(&config, &url),
        None => db.run(&config),
    };
    assert_eq!(db.leftover_tables(), Vec::<String>::new());
    out
}

fn failing(scenario: &str) -> Output {
    let db = Db::new();
    let out = run(&db, scenario, None);
    assert_eq!(out.code, Some(2), "{}{}", out.stdout, out.stderr);
    out
}

#[test]
fn vacuous_predicate_is_inconclusive() {
    let out = failing("vacuous");
    insta::assert_snapshot!(out.stdout);
}

#[test]
fn predicates_cannot_smuggle_a_second_statement() {
    let out = failing("injection");
    insta::assert_snapshot!(out.stdout);
}

#[test]
fn unknown_ambiguous_and_out_of_scope_tables_and_roles_are_located() {
    let out = failing("tables");
    assert_eq!(out.stdout, "");
    insta::assert_snapshot!(out.stderr);
}

#[test]
fn transaction_control_in_setup_is_rejected() {
    let out = failing("commit_in_setup");
    assert_eq!(out.stdout, "");
    insta::assert_snapshot!(out.stderr);
}

#[test]
fn setup_timeout_is_an_error() {
    let out = failing("timeout");
    assert_eq!(out.stdout, "");
    insta::assert_snapshot!(out.stderr);
}

#[test]
fn owner_with_forced_rls_cannot_act_as_admin() {
    let db = Db::new();
    let out = run(&db, "owner_forced", Some(db.url("owner_login", "owner")));
    assert_eq!(out.code, Some(2), "{}{}", out.stdout, out.stderr);
    insta::assert_snapshot!(out.stderr);
}

#[test]
fn owner_without_forced_rls_can_act_as_admin() {
    let db = Db::new();
    let out = run(
        &db,
        "owner_not_forced",
        Some(db.url("owner_login", "owner")),
    );
    assert_eq!(out.code, Some(0), "{}{}", out.stdout, out.stderr);
    insta::assert_snapshot!(out.stdout);
}
