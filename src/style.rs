use anstyle::{AnsiColor, Style};

pub const ERROR: Style = AnsiColor::Red.on_default().bold();
pub const WARNING: Style = AnsiColor::Yellow.on_default().bold();
pub const EMPHASIS: Style = Style::new().bold();
pub const GUTTER: Style = AnsiColor::Blue.on_default().bold();
pub const PASS: Style = AnsiColor::Green.on_default();
pub const FAIL: Style = AnsiColor::Red.on_default().bold();
pub const INCONCLUSIVE: Style = AnsiColor::Yellow.on_default();
pub const MUTED: Style = Style::new().dimmed();

/// `style` when `styled`, otherwise a style that renders nothing.
pub fn pick(styled: bool, style: Style) -> Style {
    if styled { style } else { Style::new() }
}
