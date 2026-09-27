use super::{Context, Finding, Name, Rule};

/// A `SECURITY DEFINER` function an identity role can execute, without a pinned `search_path`.
pub fn check(cx: &Context) -> Vec<Finding> {
    let mut findings = Vec::new();
    for role in cx.subject_roles() {
        for function in &role.functions {
            let object = format!(
                "{}.{}({})",
                function.schema, function.name, function.arguments
            );
            let hint = "SECURITY DEFINER without a pinned search_path: add SET search_path = ''";
            let mut finding = Finding::new(Rule::Rls005, object, hint.into());
            finding.function = Some(Name {
                schema: function.schema.clone(),
                name: function.name.clone(),
            });
            findings.push(cx.for_role(finding, role));
        }
    }
    findings
}
