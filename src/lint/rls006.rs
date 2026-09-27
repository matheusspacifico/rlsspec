use super::{Context, Finding, Rule};

/// RLS enabled with no policy at all: every row is denied, often on purpose, sometimes a forgotten migration.
pub fn check(cx: &Context) -> Vec<Finding> {
    cx.tables()
        .filter(|(_, facts)| facts.rls_enabled && facts.policies == 0)
        .map(|(table, _)| {
            let hint =
                "RLS is enabled with no policy: every row is denied (a forgotten migration?)";
            cx.on_table(Rule::Rls006, table, hint.into())
        })
        .collect()
}
