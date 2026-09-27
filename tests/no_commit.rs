use std::fs;
use std::path::Path;

fn rust_files(dir: &Path, out: &mut Vec<std::path::PathBuf>) {
    for entry in fs::read_dir(dir).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            rust_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

// Hard rule 1: no code path may issue COMMIT. The setup lexer is the only place that names it.
#[test]
fn source_never_commits() {
    let mut files = Vec::new();
    rust_files(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("src"),
        &mut files,
    );
    for file in files {
        let text = fs::read_to_string(&file).unwrap();
        let lexer = file.ends_with("src/pg/script.rs");
        for (i, line) in text.lines().enumerate() {
            let lower = line.to_ascii_lowercase();
            let offending = lower.contains(".commit(")
                || (!lexer && lower.contains("commit") && !line.trim_start().starts_with("//"));
            assert!(!offending, "{}:{}: {line}", file.display(), i + 1);
        }
    }
}
