use postgres::Transaction;

use super::Outcome;
use crate::catalog::{self, Table};
use crate::config::Identity;

/// `insert: deny` is checked statically: nothing is inserted, the catalog must prove no insert can succeed.
pub fn check(
    tx: &mut Transaction,
    table: &Table,
    name: &str,
    identity: &Identity,
) -> Result<Outcome, postgres::Error> {
    let access = catalog::insert_access(tx, table, &identity.role)?;
    let role = &identity.role;
    if !access.privilege {
        return Ok(Outcome::Pass("deny (no privilege)".into()));
    }
    let reason = if access.bypasses_rls {
        "bypasses RLS (superuser or BYPASSRLS)".to_owned()
    } else if !access.rls_enabled {
        format!("RLS is not enabled on {name}")
    } else if access.owner && !access.rls_forced {
        format!("owns {name} and RLS is not forced")
    } else {
        match access.permissive_policies.as_slice() {
            [] => return Ok(Outcome::Pass("deny (no policy)".into())),
            [policy] => format!("permissive policy `{policy}` applies to it"),
            policies => format!(
                "permissive policies {} apply to it",
                policies
                    .iter()
                    .map(|p| format!("`{p}`"))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        }
    };
    Ok(Outcome::Fail(format!(
        "`{role}` has INSERT privilege and {reason}; write `insert` cases with `values` to check which rows it accepts"
    )))
}
