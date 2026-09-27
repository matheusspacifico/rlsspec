use std::fmt::Write;
use std::path::Path;

use serde_saphyr::Location;

use crate::style;

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

pub fn render(path: &Path, source: &str, diagnostics: &[Diagnostic], styled: bool) -> String {
    let mut out = String::new();
    for diagnostic in diagnostics {
        render_one(&mut out, path, source, diagnostic, styled);
        out.push('\n');
    }
    let count = diagnostics.len();
    let plural = if count == 1 { "" } else { "s" };
    let message = format!("could not load {} ({count} error{plural})", path.display());
    heading(&mut out, &message, styled);
    out
}

fn heading(out: &mut String, message: &str, styled: bool) {
    let error = style::pick(styled, style::ERROR);
    let emphasis = style::pick(styled, style::EMPHASIS);
    let _ = writeln!(
        out,
        "{error}error{error:#}{emphasis}: {message}{emphasis:#}"
    );
}

fn render_one(out: &mut String, path: &Path, source: &str, diagnostic: &Diagnostic, styled: bool) {
    let Span { line, column } = diagnostic.span;
    let g = style::pick(styled, style::GUTTER);
    let e = style::pick(styled, style::ERROR);
    heading(out, &diagnostic.message, styled);
    let Some(text) = line.checked_sub(1).and_then(|i| source.lines().nth(i)) else {
        let _ = writeln!(out, " {g}-->{g:#} {}", path.display());
        return;
    };
    let gutter = " ".repeat(line.to_string().len());
    let caret = " ".repeat(column.saturating_sub(1));
    let _ = writeln!(
        out,
        "{gutter}{g}-->{g:#} {}:{line}:{column}",
        path.display()
    );
    let _ = writeln!(out, "{gutter} {g}|{g:#}");
    let _ = writeln!(out, "{g}{line} |{g:#} {text}");
    let _ = writeln!(out, "{gutter} {g}|{g:#} {caret}{e}^{e:#}");
}
