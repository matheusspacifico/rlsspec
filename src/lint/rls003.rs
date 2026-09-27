use super::{Context, Finding, Rule};

/// An identity role that no policy applies to. One finding per role.
pub fn check(cx: &Context) -> Vec<Finding> {
    cx.facts
        .roles
        .iter()
        .filter(|role| role.bypasses_rls())
        .map(|role| {
            let attribute = if role.superuser {
                "superuser"
            } else {
                "BYPASSRLS"
            };
            let hint = format!("{attribute}: it bypasses every policy; use a role without it");
            let mut finding = Finding::new(Rule::Rls003, role.name.clone(), hint);
            finding.identities = cx.identities_of(&role.name);
            finding
        })
        .collect()
}
