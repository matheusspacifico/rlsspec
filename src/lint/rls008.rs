use super::{Context, Finding, Rule};

/// An identity denied every operation on a table its role holds privileges on. Only reported when
/// every identity with that role is denied everything there too: otherwise the grant is needed.
pub fn check(cx: &Context) -> Vec<Finding> {
    let denies = |oid: u32, identity: &str| {
        cx.plan
            .iter()
            .find(|e| e.table.oid == oid && e.identity.name == identity)
            .is_some_and(|e| e.denies_everything())
    };
    let mut findings = Vec::new();
    for table in cx.catalog.tables() {
        for identity in &cx.config.identities {
            let Some(role) = cx.role(&identity.role) else {
                continue;
            };
            if role.bypasses_rls()
                || !role.privileged.contains(&table.oid)
                || !cx
                    .identities_of(&role.name)
                    .iter()
                    .all(|i| denies(table.oid, i))
            {
                continue;
            }
            let hint = format!(
                "`{}` is denied every operation, yet its role holds privileges here: revoke them",
                identity.name
            );
            let mut finding = cx.on_table(Rule::Rls008, table, hint);
            finding.role = Some(role.name.clone());
            finding.identities = vec![identity.name.clone()];
            findings.push(finding);
        }
    }
    findings
}
