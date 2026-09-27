use super::{Context, Finding, Rule};

/// A table in scope without row level security.
pub fn check(cx: &Context) -> Vec<Finding> {
    cx.tables()
        .filter(|(_, facts)| !facts.rls_enabled)
        .map(|(table, _)| {
            let hint = "row level security is not enabled: every role with a grant sees every row";
            cx.on_table(Rule::Rls001, table, hint.into())
        })
        .collect()
}
