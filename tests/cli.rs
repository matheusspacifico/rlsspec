use std::process::Command;

struct Output {
    code: Option<i32>,
    stdout: String,
    stderr: String,
}

fn rlsspec(args: &[&str]) -> Output {
    rlsspec_with_env(args, &[])
}

fn rlsspec_with_env(args: &[&str], env: &[(&str, &str)]) -> Output {
    let output = Command::new(env!("CARGO_BIN_EXE_rlsspec"))
        .args(args)
        .envs(env.iter().copied())
        .current_dir(env!("CARGO_MANIFEST_DIR"))
        .output()
        .unwrap();
    Output {
        code: output.status.code(),
        stdout: String::from_utf8(output.stdout).unwrap(),
        stderr: String::from_utf8(output.stderr).unwrap(),
    }
}

#[test]
fn version_prints_the_crate_version() {
    let out = rlsspec(&["version"]);
    assert_eq!(out.code, Some(0));
    assert_eq!(
        out.stdout,
        format!("rlsspec {}\n", env!("CARGO_PKG_VERSION"))
    );
}

#[test]
fn test_on_invalid_config_prints_located_errors() {
    let out = rlsspec(&["test", "-c", "tests/fixtures/cli/invalid.yaml"]);
    assert_eq!(out.code, Some(2));
    insta::assert_snapshot!(out.stderr);
}

#[test]
fn test_on_missing_config_is_a_config_error() {
    let out = rlsspec(&["test", "-c", "tests/fixtures/cli/nope.yaml"]);
    assert_eq!(out.code, Some(2));
    assert!(
        out.stderr
            .starts_with("error: cannot read tests/fixtures/cli/nope.yaml: "),
        "{}",
        out.stderr
    );
}

#[test]
fn test_refuses_remote_hosts_without_leaking_the_url() {
    let out = rlsspec(&["test", "-c", "tests/fixtures/cli/remote.yaml"]);
    assert_eq!(out.code, Some(2));
    insta::assert_snapshot!(out.stderr);
    assert!(!out.stderr.contains("secret"));
}

#[test]
fn allow_remote_passes_the_safety_guard() {
    let out = rlsspec(&[
        "test",
        "--allow-remote",
        "-c",
        "tests/fixtures/cli/remote.yaml",
    ]);
    assert_eq!(out.code, Some(2));
    assert!(!out.stderr.contains("refusing"), "{}", out.stderr);
}

#[test]
fn test_on_unreachable_database_is_a_connection_error() {
    let out = rlsspec(&["test", "-c", "tests/fixtures/cli/valid.yaml"]);
    assert_eq!(out.code, Some(2));
    assert!(
        out.stderr
            .starts_with("error: cannot connect to the database: "),
        "{}",
        out.stderr
    );
    assert!(!out.stderr.contains("postgres://"), "{}", out.stderr);
}

#[test]
fn colour_can_be_forced_and_disabled() {
    let args = ["test", "-c", "tests/fixtures/cli/invalid.yaml"];
    let forced = [("CLICOLOR_FORCE", "1")];
    let coloured = rlsspec_with_env(&args, &forced);
    assert!(coloured.stderr.contains("\x1b["), "{:?}", coloured.stderr);
    assert_eq!(
        anstream::adapter::strip_str(&coloured.stderr).to_string(),
        rlsspec(&args).stderr
    );

    let plain = rlsspec_with_env(&["--no-color", args[0], args[1], args[2]], &forced);
    assert_eq!(plain.code, Some(2));
    assert!(!plain.stderr.contains('\x1b'), "{:?}", plain.stderr);
}

#[test]
fn init_refuses_remote_hosts() {
    let output = format!("{}/init_remote.yaml", env!("CARGO_TARGET_TMPDIR"));
    let out = rlsspec_with_env(
        &["init", "-o", &output],
        &[("DATABASE_URL", "postgres://u:secret@db.example.com/app")],
    );
    assert_eq!(out.code, Some(2));
    assert_eq!(
        out.stderr,
        "error: refusing to connect to non-local host `db.example.com`; list it under `safety.allowed_hosts` or pass --allow-remote\n"
    );
    assert!(!std::path::Path::new(&output).exists());
}

#[test]
fn init_needs_database_url() {
    let output = format!("{}/init_no_url.yaml", env!("CARGO_TARGET_TMPDIR"));
    let output = std::process::Command::new(env!("CARGO_BIN_EXE_rlsspec"))
        .args(["init", "-o", &output])
        .env_remove("DATABASE_URL")
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    assert!(
        String::from_utf8(output.stderr)
            .unwrap()
            .starts_with("error: `DATABASE_URL` is not set")
    );
}
