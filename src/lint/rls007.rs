use super::{Context, Finding, Name, Rule};

/// A view readable by an identity role that reads RLS tables with its owner's rights: owned by a
/// role bypassing RLS or by an underlying table's owner, without `security_invoker`. Materialized
/// views store their rows without RLS, so they are reported whoever owns them.
pub fn check(cx: &Context) -> Vec<Finding> {
    let mut findings = Vec::new();
    for role in cx.subject_roles() {
        for view in &role.views {
            let hint = if view.materialized {
                "materialized view: its rows are stored without RLS, every reader sees all of them"
                    .to_owned()
            } else if view.owner_bypasses_rls {
                format!(
                    "reads as its owner {}, who bypasses RLS: set security_invoker = true",
                    view.owner
                )
            } else if !view.owned_tables.is_empty() {
                format!(
                    "reads as its owner {}, who owns {}: set security_invoker = true",
                    view.owner,
                    view.owned_tables.join(", ")
                )
            } else {
                continue;
            };
            let object = format!("{}.{}", view.schema, view.name);
            let mut finding = Finding::new(Rule::Rls007, object, hint);
            finding.view = Some(Name {
                schema: view.schema.clone(),
                name: view.name.clone(),
            });
            findings.push(cx.for_role(finding, role));
        }
    }
    findings
}
