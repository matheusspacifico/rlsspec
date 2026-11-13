mod common;

use common::Db;
use quick_junit::{Report, TestCaseStatus};
use serde_json::Value;

fn run(scenario: &str, format: &str, expected_code: i32) -> String {
    let db = Db::new();
    let config = format!("tests/fixtures/on_empty/{scenario}.yaml");
    let out = db.rlsspec(&["test", "-c", &config, "--format", format]);
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
fn without_the_key_wildcard_cases_on_an_empty_table_are_inconclusive_and_say_how_to_skip() {
    let out = run("inconclusive", "text", 2);
    assert_eq!(
        out.matches("or set on_empty: skip on this \"*\" default")
            .count(),
        4,
        "{out}"
    );
    insta::assert_snapshot!(out);
}

#[test]
fn skip_leaves_the_empty_table_unspecified_and_passes_under_warn() {
    insta::assert_snapshot!(run("skip_warn", "text", 0));
}

#[test]
fn skipped_cells_fail_the_run_under_fail() {
    insta::assert_snapshot!(run("skip_fail", "text", 1));
}

#[test]
fn skip_hides_no_failure_on_a_table_that_has_rows() {
    insta::assert_snapshot!(run("skip_leak", "text", 1));
}

#[test]
fn a_predicate_matching_nothing_on_a_table_with_rows_is_still_inconclusive() {
    insta::assert_snapshot!(run("no_match", "text", 2));
}

#[test]
fn json_lists_skipped_cases_apart_from_cases() {
    let out = run("skip_warn", "json", 0);
    let value: Value = serde_json::from_str(&out).unwrap();
    let skipped = value["skipped"].as_array().unwrap();
    assert_eq!(value["totals"]["skipped"], skipped.len());
    assert_eq!(skipped.len(), 4, "{out}");
    assert!(skipped.iter().all(|s| s["reason"] == "empty_table"));
    assert!(skipped.iter().all(|s| s["table"] == "drafts"));
    // `outcome` keeps its three values: a consumer that switches on it sees no new one.
    let cases = value["cases"].as_array().unwrap();
    assert!(cases.iter().all(|c| c["outcome"] == "pass"), "{out}");
    assert_eq!(value["totals"]["passed"], cases.len());
    // The skipped cells are unspecified: each is listed in the gaps.
    let gaps = value["coverage"]["gaps"].as_array().unwrap();
    let ops = |identity: &str| {
        gaps.iter()
            .find(|g| g["table"] == "drafts" && g["identity"] == identity)
            .map(|g| g["ops"].to_string())
    };
    assert_eq!(
        ops("guest").as_deref(),
        Some(r#"["select","update","delete"]"#)
    );
    assert_eq!(
        ops("alice").as_deref(),
        Some(r#"["select","insert","update","delete"]"#)
    );
    insta::assert_snapshot!(out);
}

#[test]
fn json_has_an_empty_skipped_array_when_nothing_is_skipped() {
    let out = run("inconclusive", "json", 2);
    let value: Value = serde_json::from_str(&out).unwrap();
    assert_eq!(value["skipped"], Value::Array(Vec::new()));
    assert_eq!(value["totals"]["skipped"], 0);
    assert_eq!(value["totals"]["inconclusive"], 4);
}

#[test]
fn junit_marks_skipped_cases_and_fails_their_cells_under_fail() {
    let out = run("skip_fail", "junit", 1);
    let report = Report::deserialize_from_str(&out).unwrap();
    assert_eq!(report.to_string().unwrap().trim_end(), out.trim_end());
    assert_eq!(report.errors, 0);
    let drafts = report
        .test_suites
        .iter()
        .find(|s| s.name.as_str() == "drafts")
        .unwrap();
    let skipped = drafts
        .test_cases
        .iter()
        .filter(|c| matches!(c.status, TestCaseStatus::Skipped { .. }))
        .count();
    assert_eq!((drafts.tests, skipped), (5, 4), "{out}");
    insta::assert_snapshot!(out);
}

#[test]
fn junit_skipped_cases_do_not_change_the_verdict_under_warn() {
    let out = run("skip_warn", "junit", 0);
    let report = Report::deserialize_from_str(&out).unwrap();
    assert_eq!((report.failures, report.errors), (0, 0), "{out}");
    assert!(out.contains("<skipped message=\"empty table\"/>"), "{out}");
}
