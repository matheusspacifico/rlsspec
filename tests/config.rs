use std::path::Path;

use rlsspec::config::{self, Config, ConfigError, Spec, Writes};

fn env(name: &str) -> Option<String> {
    match name {
        "DATABASE_URL" => Some("postgres://postgres@localhost:5432/app".into()),
        _ => None,
    }
}

fn parse(yaml: &str) -> Result<Config, ConfigError> {
    config::parse(yaml, Path::new("rlsspec.yaml"), &env)
}

fn errors(yaml: &str) -> String {
    match parse(yaml) {
        Ok(config) => panic!("expected errors, got {config:#?}"),
        Err(err) => err.to_string(),
    }
}

#[test]
fn full_spec_resolves() {
    let config = parse(
        r#"
version: 1
database:
  url: ${env:DATABASE_URL}
  schemas: [public, app]
vars:
  tenant_a: 0191e6a2-0000-7000-8000-00000000000a
  tenant_b: 0191e6a2-0000-7000-8000-00000000000b
  quirky: "o'neil"
identities:
  public_web:
    role: web_anon
  alice:
    role: app
    gucs: { app.tenant_id: "${tenant_a}", app.name: "${quirky}" }
unspecified: fail
defaults:
  public_web:
    "*": { select: deny, insert: deny, update: deny, delete: deny }
expect:
  documents:
    alice:
      select: { rows: "tenant_id = ${tenant_a} and owner <> ${quirky}" }
      insert:
        - { values: { tenant_id: "${tenant_a}", title: "x", rank: 1.50, archived_at: ~ }, expect: allow }
        - { values: { tenant_id: "${tenant_b}", title: "x" }, expect: deny }
      update:
        - { where: "tenant_id = ${tenant_a}", expect: allow }
        - { where: "tenant_id = ${tenant_a}", set: { tenant_id: "${tenant_b}" }, expect: deny }
      delete: deny
    public_web:
      select: { rows: "published", subset: true }
lint:
  ignore:
    - { rule: RLS006, table: audit_log, reason: "write-only through a SECURITY DEFINER function" }
    - { rule: RLS008, identity: public_web, reason: "grants managed elsewhere" }
"#,
    )
    .unwrap();
    insta::assert_debug_snapshot!(config);
}

#[test]
fn minimal_spec_uses_defaults() {
    let config = parse(
        "version: 1\ndatabase: { url: postgres://localhost/db }\nidentities:\n  a: { role: app }\n",
    )
    .unwrap();
    assert_eq!(config.database.schemas, ["public"]);
    assert_eq!(config.unspecified, config::Unspecified::Warn);
    assert!(config.expect.is_empty() && config.defaults.is_empty() && config.setup.is_empty());
}

#[test]
fn syntax_error_has_location() {
    insta::assert_snapshot!(errors(
        "version: 1\ndatabase: { url: postgres://localhost/db }\nidentities:\n  a: [role\n"
    ));
}

#[test]
fn unknown_field_has_location() {
    insta::assert_snapshot!(errors(
        "version: 1\ndatabase: { url: postgres://localhost/db }\nidentities:\n  alice: { role: app, rol: x }\n"
    ));
}

#[test]
fn duplicate_key_is_rejected() {
    insta::assert_snapshot!(errors(
        "version: 1\ndatabase: { url: postgres://localhost/db }\nidentities:\n  alice: { role: app }\n  alice: { role: other }\n"
    ));
}

#[test]
fn invalid_operation_forms() {
    insta::assert_snapshot!(errors(
        r#"version: 1
database: { url: postgres://localhost/db }
identities:
  alice: { role: app }
expect:
  documents:
    alice:
      select: maybe
"#
    ));
    insta::assert_snapshot!(errors(
        r#"version: 1
database: { url: postgres://localhost/db }
identities:
  alice: { role: app }
expect:
  documents:
    alice:
      delete: { where: "true", expect: deny }
"#
    ));
}

#[test]
fn semantic_errors_are_all_reported() {
    insta::assert_snapshot!(errors(
        r#"version: 2
database:
  url: ${env:MISSING_URL}
  schemas: []
setup:
  - tests/fixtures/does-not-exist.sql
vars:
  tenant_a: a
  bad-name: x
  chained: "${tenant_a}"
identities:
  alice:
    role: app
    gucs: { app.tenant_id: "${tenant_a}", app.user_id: "${alice_id}" }
  guest:
    role: ""
defaults:
  mallory:
    "*": { select: deny }
expect:
  documents:
    alice:
      select: { rows: "tenant_id = ${tenant_a" }
      insert: []
      update:
        - { where: "", set: {}, expect: allow }
    guest: {}
  "*":
    alice: { delete: deny }
"#
    ));
}

#[test]
fn empty_identities_is_an_error() {
    insta::assert_snapshot!(errors(
        "version: 1\ndatabase: { url: postgres://localhost/db }\nidentities: {}\n"
    ));
}

#[test]
fn setup_files_resolve_next_to_the_config() {
    let config = config::parse(
        "version: 1\ndatabase: { url: postgres://localhost/db }\nsetup: [seed.sql]\nidentities:\n  a: { role: app }\n",
        Path::new("tests/fixtures/rlsspec.yaml"),
        &env,
    )
    .unwrap();
    assert_eq!(config.setup, [Path::new("tests/fixtures/seed.sql")]);
}

#[test]
fn safety_timeouts_default_and_validate() {
    let config = parse(
        "version: 1\ndatabase: { url: postgres://localhost/db }\nidentities:\n  a: { role: app }\n",
    )
    .unwrap();
    assert_eq!(config.safety.lock_timeout, "5s");
    assert_eq!(config.safety.statement_timeout, "30s");

    let config = parse(
        "version: 1\ndatabase: { url: postgres://localhost/db }\nsafety: { lock_timeout: 250ms, statement_timeout: 2min }\nidentities:\n  a: { role: app }\n",
    )
    .unwrap();
    assert_eq!(config.safety.lock_timeout, "250ms");
    assert_eq!(config.safety.statement_timeout, "2min");

    insta::assert_snapshot!(errors(
        "version: 1\ndatabase: { url: postgres://localhost/db }\nsafety:\n  lock_timeout: 0s\n  statement_timeout: 5 seconds\nidentities:\n  a: { role: app }\n"
    ));
}

#[test]
fn insert_allow_shorthand_is_rejected() {
    insta::assert_snapshot!(errors(
        r#"version: 1
database: { url: postgres://localhost/db }
identities:
  alice: { role: app }
defaults:
  alice:
    "*": { insert: allow }
expect:
  documents:
    alice:
      insert: deny
      update: allow
"#
    ));
}

#[test]
fn todo_is_accepted_for_every_operation() {
    let config = parse(
        r#"version: 1
database: { url: postgres://localhost/db }
identities:
  alice: { role: app }
defaults:
  alice:
    "*": { select: todo, insert: todo, update: todo, delete: todo }
expect:
  documents:
    alice: { select: todo, insert: deny }
"#,
    )
    .unwrap();
    let ops = &config.defaults[0].ops;
    assert_eq!(ops.select, Some(Spec::Todo));
    assert_eq!(ops.insert, Some(Spec::Todo));
    assert_eq!(ops.update, Some(Spec::Todo));
    assert_eq!(ops.delete, Some(Spec::Todo));
    let ops = &config.expect[0].ops;
    assert_eq!(ops.select, Some(Spec::Todo));
    assert!(matches!(
        ops.insert,
        Some(Spec::Given(Writes::Shorthand { .. }))
    ));
}

#[test]
fn lint_ignore_entries_are_validated() {
    insta::assert_snapshot!(errors(
        r#"version: 1
database: { url: "${env:DATABASE_URL}" }
identities:
  alice: { role: app }
lint:
  ignore:
    - { rule: RLS006, table: audit_log }
    - { rule: RLS001, table: notes, reason: "  " }
    - { rule: RLS009, table: notes, reason: "typo" }
    - { rule: RLS001, identity: alice, reason: "RLS001 is about tables only" }
    - { rule: RLS005, table: notes, function: leak, reason: "functions, not tables" }
    - { rule: RLS003, identity: mallory, reason: "unknown identity" }
"#
    ));
}

#[test]
fn lint_ignore_rejects_unknown_keys() {
    let out = errors(
        r#"version: 1
database: { url: "${env:DATABASE_URL}" }
identities:
  alice: { role: app }
lint:
  ignore:
    - { rule: RLS004, policy: notes_edit, reason: "not a key" }
"#,
    );
    assert!(
        out.starts_with(
            "error: unknown field `policy`, expected one of rule, table, identity, function, view, reason\n --> rlsspec.yaml:7:23\n"
        ),
        "{out}"
    );
}

#[test]
fn top_level_x_keys_are_ignored_and_hold_anchors() {
    let config = parse(
        r#"version: 1
x-nobody: &nobody { select: deny, delete: deny }
x-notes: [anything, { goes: here }, "${not_a_var}"]
database: { url: postgres://localhost/db }
identities:
  alice: { role: app }
expect:
  notes:
    alice: *nobody
  tags:
    alice: { <<: *nobody, update: deny }
"#,
    )
    .unwrap();
    assert_eq!(config.expect.len(), 2);
    let given = |i: usize| {
        let ops = &config.expect[i].ops;
        [
            ops.select.is_some(),
            ops.insert.is_some(),
            ops.update.is_some(),
            ops.delete.is_some(),
        ]
    };
    assert_eq!(given(0), [true, false, false, true]);
    assert_eq!(given(1), [true, false, true, true]);
}

#[test]
fn x_keys_are_unknown_below_the_top_level_and_other_unknown_keys_keep_their_error() {
    let nested = errors(
        "version: 1\ndatabase: { url: postgres://localhost/db, x-note: 1 }\nidentities:\n  alice: { role: app }\n",
    );
    assert!(
        nested.starts_with(
            "error: unknown field `x-note`, expected one of url, schemas, exclude\n --> rlsspec.yaml:2:43\n"
        ),
        "{nested}"
    );
    let top = errors(
        "version: 1\nx-ok: 1\ndatabase: { url: postgres://localhost/db }\nxtra: 1\nidentities:\n  alice: { role: app }\n",
    );
    assert!(
        top.starts_with(
            "error: unknown field `xtra`, expected one of version, preset, database, safety, setup, vars, identities, unspecified, defaults, expect, lint\n --> rlsspec.yaml:4:1\n"
        ),
        "{top}"
    );
    let duplicate = errors(
        "version: 1\nx-a: 1\nx-a: 2\ndatabase: { url: postgres://localhost/db }\nidentities:\n  alice: { role: app }\n",
    );
    assert!(duplicate.contains("rlsspec.yaml:3:1"), "{duplicate}");
}

/// The parser's alias/anchor ratio check (100 aliases, more than 10 per anchor) is off: a spec
/// that shares a few anchors across many tables is the document `x-*` keys invite.
#[test]
fn many_aliases_of_one_anchor_are_accepted() {
    let mut yaml = String::from(
        "version: 1\nx-deny: &deny { select: deny }\ndatabase: { url: postgres://localhost/db }\nidentities:\n  alice: { role: app }\nexpect:\n",
    );
    for i in 0..150 {
        yaml.push_str(&format!("  t{i}:\n    alice: *deny\n"));
    }
    assert_eq!(parse(&yaml).unwrap().expect.len(), 150);
}

#[test]
fn database_exclude_takes_names_not_patterns() {
    let config = parse(
        "version: 1\ndatabase: { url: postgres://localhost/db, exclude: [goose_db_version, app.schema_migrations] }\nidentities:\n  a: { role: app }\n",
    )
    .unwrap();
    let excluded: Vec<&str> = config
        .database
        .exclude
        .iter()
        .map(|e| e.table.as_str())
        .collect();
    assert_eq!(excluded, ["goose_db_version", "app.schema_migrations"]);

    let out = errors(
        "version: 1\ndatabase:\n  url: postgres://localhost/db\n  exclude: [\"audit_*\", \"\"]\nidentities:\n  a: { role: app }\n",
    );
    assert!(
        out.starts_with(
            "error: `database.exclude` takes table names; patterns are not supported yet, found `audit_*`\n --> rlsspec.yaml:4:13\n"
        ),
        "{out}"
    );
    assert!(
        out.contains("error: table name is empty\n --> rlsspec.yaml:4:24\n"),
        "{out}"
    );
}

#[test]
fn on_empty_is_only_allowed_on_a_wildcard_default() {
    let config = parse(
        r#"version: 1
database: { url: postgres://localhost/db }
identities:
  alice: { role: app }
  bob: { role: app }
defaults:
  alice:
    "*": { select: deny, on_empty: skip }
  bob:
    "*": { select: deny, on_empty: inconclusive }
"#,
    )
    .unwrap();
    let on_empty: Vec<_> = config.defaults.iter().map(|b| b.on_empty).collect();
    assert_eq!(
        on_empty,
        [config::OnEmpty::Skip, config::OnEmpty::Inconclusive]
    );

    insta::assert_snapshot!(errors(
        r#"version: 1
database: { url: postgres://localhost/db }
identities:
  alice: { role: app }
defaults:
  alice:
    "*": { on_empty: skip }
    notes: { select: deny, on_empty: skip }
expect:
  tags:
    alice: { select: deny, on_empty: inconclusive }
"#
    ));
    let out = errors(
        "version: 1\ndatabase: { url: postgres://localhost/db }\nidentities:\n  alice: { role: app }\ndefaults:\n  alice:\n    \"*\": { select: deny, on_empty: ignore }\n",
    );
    assert!(
        out.contains("unknown variant `ignore`, expected one of inconclusive, skip"),
        "{out}"
    );
}

#[test]
fn rows_cases_resolve_for_update_and_delete() {
    let config = parse(
        r#"version: 1
database: { url: postgres://localhost/db }
vars: { tenant: 0191e6a2-0000-7000-8000-00000000000a }
identities:
  alice: { role: app }
defaults:
  alice:
    "*": { delete: { rows: "true", subset: true } }
expect:
  notes:
    alice:
      update: { set: { body: "x" }, rows: "tenant_id = ${tenant}" }
      delete:
        - { rows: "tenant_id = ${tenant}", subset: true }
        - { where: "id = 1", expect: deny }
"#,
    )
    .unwrap();
    let ops = &config.expect[0].ops;
    let Some(Spec::Given(Writes::Cases(update))) = &ops.update else {
        panic!("{ops:?}");
    };
    assert_eq!(update.len(), 1);
    assert_eq!(
        update[0].target,
        config::Affects::Rows {
            predicate: "tenant_id = '0191e6a2-0000-7000-8000-00000000000a'".into(),
            subset: false
        }
    );
    assert!(update[0].set.is_some());
    let Some(Spec::Given(Writes::Cases(delete))) = &ops.delete else {
        panic!("{ops:?}");
    };
    assert!(matches!(
        delete[0].target,
        config::Affects::Rows { subset: true, .. }
    ));
    assert_eq!(delete[1].target.expect(), Some(config::Expectation::Deny));
    assert_eq!(delete[0].target.expect(), None);
}

#[test]
fn invalid_rows_cases_are_located() {
    insta::assert_snapshot!(errors(
        r#"version: 1
database: { url: postgres://localhost/db }
identities:
  alice: { role: app }
expect:
  notes:
    alice:
      update:
        - { rows: "true" }
        - { set: { body: "x" }, rows: "true", where: "id = 1" }
        - { set: { body: "x" }, rows: "true", expect: deny }
        - { where: "id = 1", subset: true, expect: deny }
        - { where: "id = 1" }
        - { set: { body: "x" } }
      delete: { where: "id = 1", expect: deny }
  tags:
    alice:
      update: { rows: "true" }
      delete:
        - { expect: allow }
"#
    ));
    let out = errors(
        "version: 1\ndatabase: { url: postgres://localhost/db }\nidentities:\n  alice: { role: app }\nexpect:\n  notes:\n    alice:\n      delete: { rows: \"true\", set: { body: x } }\n",
    );
    assert!(
        out.starts_with(
            "error: unknown field `set`, expected one of where, expect, rows, subset\n"
        ),
        "{out}"
    );
    let out = errors(
        "version: 1\ndatabase: { url: postgres://localhost/db }\nidentities:\n  alice: { role: app }\nexpect:\n  notes:\n    alice:\n      insert: { values: { id: 1 }, expect: deny }\n",
    );
    assert!(
        out.starts_with(
            "error: invalid type: map, expected `allow`, `deny`, `todo` or a list of cases\n"
        ),
        "{out}"
    );
}

#[test]
fn column_filters_are_parsed_from_the_defaults_key() {
    let config = parse(
        r#"version: 1
database: { url: postgres://localhost/db }
identities:
  alice: { role: app }
defaults:
  alice:
    "*": { select: deny }
    "*[tenant_id]": { select: all, on_empty: skip }
    "*[ org_id , deleted_at ]": { delete: deny }
    notes: { select: all }
"#,
    )
    .unwrap();
    let tables: Vec<String> = config
        .defaults
        .iter()
        .map(|b| b.table.to_string())
        .collect();
    assert_eq!(
        tables,
        ["*", "*[tenant_id]", "*[org_id, deleted_at]", "notes"]
    );
    assert_eq!(
        config.defaults[2].table,
        config::TableRef::Having(vec!["org_id".into(), "deleted_at".into()])
    );
    assert_eq!(config.defaults[1].on_empty, config::OnEmpty::Skip);

    insta::assert_snapshot!(errors(
        r#"version: 1
database: { url: postgres://localhost/db }
identities:
  alice: { role: app }
defaults:
  alice:
    "*[]": { select: deny }
    "*[a,,b]": { select: deny }
    "*_log": { select: deny }
    "*[a": { select: deny }
expect:
  "*[tenant_id]":
    alice: { select: deny }
"#
    ));
}
