use postgres::Transaction;

use super::Catalog;

/// Security invoker views (the only way a view applies its reader's policies) exist from PostgreSQL 15.
pub const SECURITY_INVOKER_VERSION: i32 = 150000;

/// What the lint rules read from the catalog: the tables in scope, and what each identity role can do.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LintCatalog {
    pub server_version: i32,
    pub tables: Vec<RlsTable>,
    pub roles: Vec<RoleFacts>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RlsTable {
    pub oid: u32,
    pub rls_enabled: bool,
    pub rls_forced: bool,
    pub policies: i64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RoleFacts {
    pub name: String,
    pub superuser: bool,
    pub bypassrls: bool,
    /// Tables in scope whose owner's privileges the role has.
    pub owns: Vec<u32>,
    /// Tables in scope the role holds any table or column privilege on.
    pub privileged: Vec<u32>,
    pub true_policies: Vec<TruePolicy>,
    pub functions: Vec<Function>,
    /// Empty before PostgreSQL 15.
    pub views: Vec<View>,
}

impl RoleFacts {
    pub fn bypasses_rls(&self) -> bool {
        self.superuser || self.bypassrls
    }
}

/// A permissive write policy that applies to the role and whose `USING` or `WITH CHECK` is `true`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TruePolicy {
    pub table: u32,
    pub name: String,
    /// `pg_policy.polcmd`: `a` insert, `w` update, `d` delete, `*` all.
    pub command: String,
    pub using_true: bool,
    pub check_true: bool,
}

/// A `SECURITY DEFINER` function without `search_path` in its settings that the role can execute,
/// and that doesn't belong to an extension.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Function {
    pub schema: String,
    pub name: String,
    pub arguments: String,
}

/// A view or materialized view in scope, without `security_invoker`, that the role can read and
/// that reads at least one table with RLS enabled.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct View {
    pub schema: String,
    pub name: String,
    pub materialized: bool,
    pub owner: String,
    pub owner_bypasses_rls: bool,
    /// The RLS tables it reads whose owner's privileges the view's owner has.
    pub owned_tables: Vec<String>,
}

// A policy applies to the role `$2` when it targets PUBLIC (oid 0) or a role whose privileges `$2` has.
macro_rules! policy_applies_to_role {
    () => {
        "(0 = ANY(p.polroles)
          OR EXISTS (SELECT 1 FROM unnest(p.polroles) AS pr(oid)
                     WHERE pr.oid <> 0 AND pg_has_role($2::name, pr.oid, 'USAGE')))"
    };
}
pub(super) use policy_applies_to_role;

const TABLES: &str = "
SELECT c.oid, c.relrowsecurity, c.relforcerowsecurity,
       (SELECT count(*) FROM pg_policy p WHERE p.polrelid = c.oid)
FROM pg_class c
WHERE c.oid = ANY($1)";

const ROLE: &str = "SELECT rolsuper, rolbypassrls FROM pg_roles WHERE rolname = $1::name";

const OWNS: &str = "
SELECT c.oid FROM pg_class c
WHERE c.oid = ANY($1) AND pg_has_role($2::name, c.relowner, 'USAGE')";

const PRIVILEGED: &str = "
SELECT c.oid FROM pg_class c
WHERE c.oid = ANY($1)
  AND (has_table_privilege($2::name, c.oid, 'SELECT, INSERT, UPDATE, DELETE, TRUNCATE, REFERENCES, TRIGGER')
       OR has_any_column_privilege($2::name, c.oid, 'SELECT, INSERT, UPDATE, REFERENCES'))";

const TRUE_POLICIES: &str = concat!(
    "
SELECT p.polrelid, p.polname::text, p.polcmd::text,
       coalesce(pg_get_expr(p.polqual, p.polrelid) = 'true', false),
       coalesce(pg_get_expr(p.polwithcheck, p.polrelid) = 'true', false)
FROM pg_policy p
WHERE p.polrelid = ANY($1) AND p.polpermissive AND p.polcmd IN ('a', 'w', 'd', '*')
  AND (pg_get_expr(p.polqual, p.polrelid) = 'true' OR pg_get_expr(p.polwithcheck, p.polrelid) = 'true')
  AND ",
    policy_applies_to_role!(),
    "
ORDER BY p.polrelid, p.polname"
);

const FUNCTIONS: &str = "
SELECT n.nspname::text, p.proname::text, pg_get_function_identity_arguments(p.oid)
FROM pg_proc p
JOIN pg_namespace n ON n.oid = p.pronamespace
WHERE p.prosecdef
  AND n.nspname NOT IN ('pg_catalog', 'information_schema')
  AND (n.nspname = ANY($1) OR has_schema_privilege($2::name, n.oid, 'USAGE'))
  AND has_function_privilege($2::name, p.oid, 'EXECUTE')
  AND NOT EXISTS (SELECT 1 FROM unnest(p.proconfig) AS s(setting) WHERE s.setting LIKE 'search_path=%')
  AND NOT EXISTS (SELECT 1 FROM pg_depend d
                  WHERE d.classid = 'pg_proc'::regclass AND d.objid = p.oid AND d.deptype = 'e')
ORDER BY 1, 2, 3";

// The tables a view reads directly are the ones its `_RETURN` rule depends on.
const VIEWS: &str = "
WITH views AS (
    SELECT c.oid, n.nspname::text AS schema, c.relname::text AS name, c.relkind = 'm' AS materialized,
           c.relowner, o.rolname::text AS owner, o.rolsuper OR o.rolbypassrls AS owner_bypasses
    FROM pg_class c
    JOIN pg_namespace n ON n.oid = c.relnamespace
    JOIN pg_roles o ON o.oid = c.relowner
    WHERE c.relkind IN ('v', 'm') AND n.nspname = ANY($1)
      AND (has_table_privilege($2::name, c.oid, 'SELECT') OR has_any_column_privilege($2::name, c.oid, 'SELECT'))
      AND NOT coalesce((SELECT opt.option_value::bool FROM pg_options_to_table(c.reloptions) AS opt
                        WHERE opt.option_name = 'security_invoker'), false)
)
SELECT v.schema, v.name, v.materialized, v.owner, v.owner_bypasses,
       coalesce(array_agg(DISTINCT t.oid::regclass::text ORDER BY t.oid::regclass::text)
                FILTER (WHERE pg_has_role(v.relowner, t.relowner, 'USAGE')), '{}')
FROM views v
JOIN pg_rewrite r ON r.ev_class = v.oid
JOIN pg_depend d ON d.classid = 'pg_rewrite'::regclass AND d.objid = r.oid
                AND d.refclassid = 'pg_class'::regclass
JOIN pg_class t ON t.oid = d.refobjid AND t.oid <> v.oid AND t.relkind IN ('r', 'p') AND t.relrowsecurity
GROUP BY v.schema, v.name, v.materialized, v.owner, v.owner_bypasses
ORDER BY 1, 2";

const MISSING_ROLES: &str = "
SELECT r.name::text FROM unnest($1::text[]) AS r(name)
WHERE NOT EXISTS (SELECT 1 FROM pg_roles WHERE rolname = r.name)";

/// Which of `roles` don't exist.
pub fn missing_roles(tx: &mut Transaction, roles: &[&str]) -> Result<Vec<String>, postgres::Error> {
    let rows = tx.query(MISSING_ROLES, &[&roles])?;
    Ok(rows.iter().map(|row| row.get(0)).collect())
}

/// Reads the catalog for the tables in `catalog` and each of `roles`, which must all exist.
pub fn load(
    tx: &mut Transaction,
    catalog: &Catalog,
    schemas: &[String],
    roles: &[&str],
) -> Result<LintCatalog, postgres::Error> {
    let server_version: i32 = tx
        .query_one("SELECT current_setting('server_version_num')::int", &[])?
        .get(0);
    let oids: Vec<u32> = catalog.tables().iter().map(|t| t.oid).collect();
    let mut tables: Vec<RlsTable> = tx
        .query(TABLES, &[&oids])?
        .iter()
        .map(|row| RlsTable {
            oid: row.get(0),
            rls_enabled: row.get(1),
            rls_forced: row.get(2),
            policies: row.get(3),
        })
        .collect();
    tables.sort_by_key(|t| oids.iter().position(|&oid| oid == t.oid));

    let mut facts = Vec::new();
    for &role in roles {
        let row = tx.query_one(ROLE, &[&role])?;
        let oids_where = |tx: &mut Transaction, sql: &str| -> Result<Vec<u32>, postgres::Error> {
            let rows = tx.query(sql, &[&oids, &role])?;
            Ok(rows.iter().map(|row| row.get(0)).collect())
        };
        let owns = oids_where(tx, OWNS)?;
        let privileged = oids_where(tx, PRIVILEGED)?;
        let true_policies = tx
            .query(TRUE_POLICIES, &[&oids, &role])?
            .iter()
            .map(|row| TruePolicy {
                table: row.get(0),
                name: row.get(1),
                command: row.get(2),
                using_true: row.get(3),
                check_true: row.get(4),
            })
            .collect();
        let functions = tx
            .query(FUNCTIONS, &[&schemas, &role])?
            .iter()
            .map(|row| Function {
                schema: row.get(0),
                name: row.get(1),
                arguments: row.get(2),
            })
            .collect();
        let views = if server_version >= SECURITY_INVOKER_VERSION {
            tx.query(VIEWS, &[&schemas, &role])?
                .iter()
                .map(|row| View {
                    schema: row.get(0),
                    name: row.get(1),
                    materialized: row.get(2),
                    owner: row.get(3),
                    owner_bypasses_rls: row.get(4),
                    owned_tables: row.get(5),
                })
                .collect()
        } else {
            Vec::new()
        };
        facts.push(RoleFacts {
            name: role.to_owned(),
            superuser: row.get(0),
            bypassrls: row.get(1),
            owns,
            privileged,
            true_policies,
            functions,
            views,
        });
    }
    Ok(LintCatalog {
        server_version,
        tables,
        roles: facts,
    })
}
