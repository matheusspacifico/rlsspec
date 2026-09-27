use postgres::Transaction;

use crate::config::Identity;
use crate::pg;

/// Becomes `identity` until the enclosing savepoint is rolled back.
pub fn apply(tx: &mut Transaction, identity: &Identity) -> Result<(), postgres::Error> {
    let role = pg::quote_ident(&identity.role);
    tx.execute(&format!("SET LOCAL ROLE {role}"), &[])?;
    for guc in &identity.gucs {
        pg::set_local(tx, &guc.name, &guc.value)?;
    }
    Ok(())
}
