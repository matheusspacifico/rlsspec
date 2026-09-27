use super::{Context, Finding, Rule, Severity};

/// A table in scope without row level security. Info when no identity role holds a privilege on it.
pub fn check(cx: &Context) -> Vec<Finding> {
    cx.tables()
        .filter(|(_, facts)| !facts.rls_enabled)
        .map(|(table, facts)| {
            let roles: Vec<&str> = cx
                .subject_roles()
                .filter(|role| role.privileged.contains(&facts.oid))
                .map(|role| role.name.as_str())
                .collect();
            let hint = match roles.as_slice() {
                [] => "row level security is not enabled; no identity role has a privilege on it, but a grant would expose every row".to_owned(),
                [role] => format!("row level security is not enabled: {role} sees every row"),
                roles => format!(
                    "row level security is not enabled: {} see every row",
                    roles.join(", ")
                ),
            };
            let mut finding = cx.on_table(Rule::Rls001, table, hint);
            if roles.is_empty() {
                finding.severity = Severity::Info;
            }
            finding
        })
        .collect()
}
