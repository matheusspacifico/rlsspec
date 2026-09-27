use std::fmt::Write;
use std::path::Path;

use serde_saphyr::Location;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct Span {
    pub line: usize,
    pub column: usize,
}

impl From<Location> for Span {
    fn from(location: Location) -> Self {
        Self {
            line: usize::try_from(location.line()).unwrap_or(0),
            column: usize::try_from(location.column()).unwrap_or(0),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Diagnostic {
    pub span: Span,
    pub message: String,
}

pub fn render(path: &Path, source: &str, diagnostics: &[Diagnostic]) -> String {
    let mut out = String::new();
    for diagnostic in diagnostics {
        render_one(&mut out, path, source, diagnostic);
        out.push('\n');
    }
    let count = diagnostics.len();
    let plural = if count == 1 { "" } else { "s" };
    let _ = writeln!(
        out,
        "error: could not load {} ({count} error{plural})",
        path.display()
    );
    out
}

fn render_one(out: &mut String, path: &Path, source: &str, diagnostic: &Diagnostic) {
    let Span { line, column } = diagnostic.span;
    let _ = writeln!(out, "error: {}", diagnostic.message);
    let Some(text) = line.checked_sub(1).and_then(|i| source.lines().nth(i)) else {
        let _ = writeln!(out, " --> {}", path.display());
        return;
    };
    let gutter = " ".repeat(line.to_string().len());
    let caret = " ".repeat(column.saturating_sub(1));
    let _ = writeln!(out, "{gutter}--> {}:{line}:{column}", path.display());
    let _ = writeln!(out, "{gutter} |");
    let _ = writeln!(out, "{line} | {text}");
    let _ = writeln!(out, "{gutter} | {caret}^");
}
