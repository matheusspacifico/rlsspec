use super::{Context, Finding, Rule};

/// RLS enabled but not forced on a table whose owner's privileges an identity role has.
pub fn check(cx: &Context) -> Vec<Finding> {
    let mut findings = Vec::new();
    for role in cx.subject_roles() {
        for (table, facts) in cx.tables() {
            if facts.rls_enabled && !facts.rls_forced && role.owns.contains(&table.oid) {
                let hint = "has the owner's privileges and RLS is not forced, so no policy applies: FORCE ROW LEVEL SECURITY";
                let finding = cx.on_table(Rule::Rls002, table, hint.into());
                findings.push(cx.for_role(finding, role));
            }
        }
    }
    findings
}
