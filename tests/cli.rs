use std::process::Command;

struct Output {
    code: Option<i32>,
    stdout: String,
    stderr: String,
}

fn rlsspec(args: &[&str]) -> Output {
    let output = Command::new(env!("CARGO_BIN_EXE_rlsspec"))
        .args(args)
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
fn test_on_valid_config_reaches_the_runner() {
    let out = rlsspec(&["test", "-c", "tests/fixtures/cli/valid.yaml"]);
    assert_eq!(out.code, Some(2));
    assert_eq!(out.stderr, "error: running cases is not implemented yet\n");
}
