mod common;

use common::{Db, Output};

fn rlsspec(command: &str, scenario: &str, expected_code: i32) -> Output {
    let db = Db::new();
    let config = format!("tests/fixtures/filter/{scenario}.yaml");
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

fn report(scenario: &str, expected_code: i32) -> String {
    let out = rlsspec("test", scenario, expected_code);
    assert_eq!(out.stderr, "");
    out.stdout
}

#[test]
fn one_block_covers_every_table_that_has_the_column() {
    insta::assert_snapshot!(report("pass", 0));
}

#[test]
fn the_blanket_rule_catches_write_policies_open_to_every_tenant() {
    let out = report("broken", 1);
    assert!(
        out.contains("update: updated 1 of the 1 row it should not update: id=3"),
        "{out}"
    );
    assert!(
        out.contains("delete: deleted 1 of the 1 row it should not delete: id=3"),
        "{out}"
    );
    insta::assert_snapshot!(out);
}

#[test]
fn expect_beats_a_named_default_beats_a_filter_beats_the_wildcard() {
    insta::assert_snapshot!(report("precedence", 0));
}

#[test]
fn a_vacuous_filtered_case_names_its_key() {
    let out = report("empty", 2);
    assert!(
        out.contains("or set on_empty: skip on this \"*[tenant_id]\" default"),
        "{out}"
    );
}

#[test]
fn the_matrix_counts_the_cells_a_filter_gives() {
    let out = rlsspec("cover", "pass", 0);
    insta::assert_snapshot!(out.stdout);
}

#[test]
fn filters_matching_no_table_or_overlapping_are_located() {
    for command in ["test", "cover", "lint"] {
        let out = rlsspec(command, "errors", 2);
        assert_eq!(out.stdout, "");
        insta::assert_snapshot!("filter_errors", out.stderr);
    }
}
