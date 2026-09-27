mod common;

use common::{Db, Output};

const SKIPPED_RLS007: &str = "RLS007 skipped: needs PostgreSQL 15 or later (security_invoker)\n";
const NOTHING: &str = "0 errors · 0 warnings · 0 info · 0 ignored";

fn lint(db: &Db, fixture: &str) -> Output {
    db.rlsspec(&["lint", "-c", &format!("tests/fixtures/lint/{fixture}.yaml")])
}

/// Lints `fixture` in a fresh database and returns stdout without the PostgreSQL 14 note, which
/// is checked here: it's printed exactly when the server can't have security invoker views.
fn run(fixture: &str, expected_code: i32) -> String {
    let db = Db::new();
    let out = lint(&db, fixture);
    assert_eq!(
        out.code,
        Some(expected_code),
        "stdout:\n{}\nstderr:\n{}",
        out.stdout,
        out.stderr
    );
    assert_eq!(out.stderr, "");
    assert_eq!(db.leftover_tables(), Vec::<String>::new());
    assert_eq!(
        out.stdout.contains(SKIPPED_RLS007),
        db.server_version() < 150000,
        "{}",
        out.stdout
    );
    out.stdout.replace(SKIPPED_RLS007, "")
}

/// The finding lines and the summary line of a lint report.
fn split(out: &str) -> (Vec<&str>, &str) {
    let mut lines: Vec<&str> = out.lines().collect();
    let summary = lines.pop().unwrap_or_default();
    (lines, summary)
}

fn assert_clean(fixture: &str) {
    assert_eq!(run(fixture, 0), format!("{NOTHING}\n"));
}

fn supports_security_invoker() -> bool {
    Db::new().server_version() >= 150000
}

#[test]
fn a_correct_setup_has_no_finding() {
    assert_clean("clean");
}

#[test]
fn rls001_table_without_rls() {
    let out = run("rls001_red", 1);
    assert_eq!(
        split(&out),
        (
            vec![
                "✗ RLS001  tags               row level security is not enabled: app sees every row",
                "i RLS001  schema_migrations  row level security is not enabled; no identity role has a privilege on it, but a grant would expose every row",
            ],
            "1 error · 0 warnings · 1 info · 0 ignored"
        )
    );
    assert_clean("rls001_green");
}

#[test]
fn rls002_identity_role_owns_a_table_without_force() {
    let out = run("rls002_red", 1);
    assert_eq!(
        split(&out).0,
        [
            "✗ RLS002  notes  app  has the owner's privileges and RLS is not forced, so no policy applies: FORCE ROW LEVEL SECURITY"
        ]
    );
    assert_clean("rls002_green");
}

#[test]
fn rls003_bypassing_identity_roles_once_per_role() {
    let db = Db::new();
    let out = lint(&db, "rls003_red");
    assert_eq!(out.code, Some(1), "{}", out.stderr);
    let out = out.stdout.replace(SKIPPED_RLS007, "");
    assert_eq!(
        split(&out),
        (
            vec![
                "✗ RLS003  postgres      superuser: it bypasses every policy; use a role without it",
                "✗ RLS003  lint_auditor  BYPASSRLS: it bypasses every policy; use a role without it",
            ],
            "2 errors · 0 warnings · 0 info · 0 ignored"
        )
    );
    assert!(!db.role_exists("lint_auditor"), "setup was not rolled back");
    assert_clean("rls003_green");
}

#[test]
fn rls004_write_policies_with_true_applying_to_an_identity_role() {
    let out = run("rls004_red", 0);
    insta::assert_snapshot!(out);
    assert_eq!(
        split(&out).0,
        [
            "! RLS004  notes  app  permissive DELETE policy `anyone_deletes` has USING (true): it allows every row",
            "! RLS004  notes  app  permissive UPDATE policy `anyone_edits` has USING (true): it allows every row",
        ]
    );
    assert_clean("rls004_green");
}

#[test]
fn rls005_security_definer_without_search_path() {
    let out = run("rls005_red", 0);
    assert_eq!(
        split(&out),
        (
            vec![
                "! RLS005  public.note_count()  app  SECURITY DEFINER without a pinned search_path: add SET search_path = ''"
            ],
            "0 errors · 1 warning · 0 info · 0 ignored"
        )
    );
    assert_clean("rls005_green");
}

#[test]
fn rls006_rls_without_policies_is_info() {
    let out = run("rls006_red", 0);
    assert_eq!(
        split(&out),
        (
            vec![
                "i RLS006  audit_log  RLS is enabled with no policy: every row is denied (a forgotten migration?)"
            ],
            "0 errors · 0 warnings · 1 info · 0 ignored"
        )
    );
    assert_clean("rls006_green");
}

#[test]
fn rls007_views_reading_as_a_bypassing_owner() {
    let out = run("rls007_red", 0);
    if !supports_security_invoker() {
        assert_eq!(out, format!("{NOTHING}\n"));
        return;
    }
    insta::assert_snapshot!(out);
    assert_eq!(
        split(&out).0,
        [
            "! RLS007  public.note_counts     app  materialized view: its rows are stored without RLS, every reader sees all of them",
            "! RLS007  public.notes_all       app  reads as its owner postgres, who bypasses RLS: set security_invoker = true",
            "! RLS007  public.notes_by_owner  app  reads as its owner owner_login, who owns notes: set security_invoker = true",
        ]
    );
    assert_clean("rls007_green");
}

#[test]
fn rls008_grants_to_a_role_every_identity_of_which_is_denied_everything() {
    let out = run("rls008_red", 0);
    assert_eq!(
        split(&out),
        (
            vec![
                "! RLS008  notes  web_anon  `guest` is denied every operation, yet its role holds privileges here: revoke them"
            ],
            "0 errors · 1 warning · 0 info · 0 ignored"
        )
    );
    assert_clean("rls008_green");
}

#[test]
fn ignore_entries_match_on_their_keys_and_stale_ones_are_reported() {
    let out = run("ignore", 0);
    insta::assert_snapshot!(out);
    assert_eq!(
        split(&out),
        (
            vec![
                "! RLS004  notes                               app  permissive UPDATE policy `anyone_edits` has USING (true): it allows every row",
                "! RLS006  tests/fixtures/lint/ignore.yaml:12       stale ignore: no RLS006 finding matches it; remove it",
            ],
            "0 errors · 2 warnings · 0 info · 2 ignored"
        )
    );
}

#[test]
fn ignore_entries_without_a_table_match_every_table() {
    assert_eq!(
        run("ignore_every_table", 0),
        "0 errors · 0 warnings · 0 info · 3 ignored\n"
    );
}

#[test]
fn ignore_entries_of_a_skipped_rule_are_not_stale() {
    let out = run("ignore_views", 0);
    let expected = if supports_security_invoker() {
        "0 errors · 0 warnings · 0 info · 3 ignored\n"
    } else {
        "0 errors · 0 warnings · 0 info · 0 ignored\n"
    };
    assert_eq!(out, expected);
}

#[test]
fn invalid_ignore_entries_are_config_errors() {
    let db = Db::new();
    let out = lint(&db, "ignore_invalid");
    assert_eq!(out.code, Some(2));
    assert_eq!(out.stdout, "");
    insta::assert_snapshot!(out.stderr);
}

#[test]
fn an_identity_role_that_does_not_exist_is_located() {
    let db = Db::new();
    let out = lint(&db, "unknown_role");
    assert_eq!(out.code, Some(2));
    assert_eq!(out.stdout, "");
    assert_eq!(
        out.stderr,
        "error: role `lint_nobody` of identity `ghost` does not exist
 --> tests/fixtures/lint/unknown_role.yaml:6:18
  |
6 |   ghost: { role: lint_nobody }
  |                  ^

error: stopped before running any case (1 error in tests/fixtures/lint/unknown_role.yaml)
"
    );
}

#[test]
fn lint_applies_no_identity() {
    // Connected as owner_login, which can't SET ROLE to postgres: `test` stops at the preflight.
    let db = Db::new();
    let url = db.url("owner_login", "owner");
    let config = "tests/fixtures/lint/no_set_role.yaml";
    let test = db.rlsspec_with_url(&["test", "-c", config], &url);
    assert_eq!(test.code, Some(2));
    assert!(
        test.stderr
            .starts_with("error: cannot act as identity `root`: permission denied to set role"),
        "{}",
        test.stderr
    );
    let out = db.rlsspec_with_url(&["lint", "-c", config], &url);
    assert_eq!(out.code, Some(1), "{}", out.stderr);
    assert!(
        out.stdout
            .starts_with("✗ RLS003  postgres  superuser: it bypasses every policy"),
        "{}",
        out.stdout
    );
}
