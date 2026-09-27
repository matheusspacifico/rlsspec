use super::{Context, Finding, Rule};

/// A permissive write policy applying to an identity role with `USING (true)` or `WITH CHECK (true)`.
/// SELECT policies are left out: a public read is often intended.
pub fn check(cx: &Context) -> Vec<Finding> {
    let mut findings = Vec::new();
    for role in cx.subject_roles() {
        for policy in &role.true_policies {
            let Some(table) = cx.table(policy.table) else {
                continue;
            };
            let command = match policy.command.as_str() {
                "a" => "INSERT",
                "w" => "UPDATE",
                "d" => "DELETE",
                _ => "ALL",
            };
            let clauses = match (policy.using_true, policy.check_true) {
                (true, true) => "USING (true) and WITH CHECK (true)",
                (true, false) => "USING (true)",
                _ => "WITH CHECK (true)",
            };
            let hint = format!(
                "permissive {command} policy `{}` has {clauses}: it allows every row",
                policy.name
            );
            let finding = cx.on_table(Rule::Rls004, table, hint);
            findings.push(cx.for_role(finding, role));
        }
    }
    findings
}
