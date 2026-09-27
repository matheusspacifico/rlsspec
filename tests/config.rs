use std::path::Path;

use rlsspec::config::{self, Config, ConfigError};

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
