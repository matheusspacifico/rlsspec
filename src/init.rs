use std::fmt::Write as _;
use std::fs::{File, OpenOptions};
use std::io::{self, Write as _};
use std::path::{Path, PathBuf};

use postgres::Transaction;

use crate::catalog;
use crate::config::Safety;
use crate::pg::{self, PgError, Session, Target};
use crate::preset::Preset;

#[derive(Debug, thiserror::Error)]
pub enum InitError {
    #[error(transparent)]
    Pg(#[from] PgError),
    #[error("no tables found in {}", schemas_phrase(.0))]
    NoTables(Vec<String>),
    #[error(
        "no role other than PUBLIC holds a privilege on the tables in {}; grant privileges to the application roles first",
        schemas_phrase(.0)
    )]
    NoRoles(Vec<String>),
    #[error(
        "the {} preset needs the roles {}, but the database has no role {}",
        .preset.name(), .needed.join(" and "), .missing.join(" or ")
    )]
    MissingRoles {
        preset: Preset,
        needed: Vec<&'static str>,
        missing: Vec<&'static str>,
    },
    #[error("{} already exists; pass --force to overwrite it", .0.display())]
    Exists(PathBuf),
    #[error("cannot write {}", path.display())]
    Write {
        path: PathBuf,
        #[source]
        source: io::Error,
    },
}

impl From<postgres::Error> for InitError {
    fn from(err: postgres::Error) -> Self {
        Self::Pg(PgError::Query(err))
    }
}

/// What `init` found in the database: the tables in scope and the identities to scaffold.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Scaffold {
    pub schemas: Vec<String>,
    pub tables: Vec<String>,
    pub preset: Option<Preset>,
    /// A comment line describing which identities were scaffolded and why.
    pub rationale: &'static str,
    pub identities: Vec<Identity>,
    pub left_out: Vec<(String, &'static str)>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Identity {
    pub name: String,
    pub role: String,
    /// A YAML flow mapping written as the identity's `claims`.
    pub claims: Option<String>,
    pub todo: Option<&'static str>,
}

/// A role holding a privilege (table or column grant) on a table in scope.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Grantee {
    pub role: String,
    pub superuser: bool,
    pub bypassrls: bool,
    pub owner_only: bool,
}

impl Grantee {
    /// Why no identity should be scaffolded for this role, if there is a reason.
    pub fn unfit(&self) -> Option<&'static str> {
        if self.superuser {
            Some("superuser, so no policy applies to it")
        } else if self.bypassrls {
            Some("BYPASSRLS, so no policy applies to it")
        } else if self.owner_only {
            Some("holds privileges only as the owner of the tables")
        } else {
            None
        }
    }
}

const GUCS_TODO: &str =
    "gucs, the session settings your policies read, e.g. gucs: { app.user_id: \"…\" }";

impl Scaffold {
    pub fn cells(&self) -> usize {
        self.tables.len() * self.identities.len() * 4
    }
}

// Table and column grants (column ones included: `GRANT UPDATE (name)` is a privilege on the table).
const ROLES: &str = "
WITH grants AS (
    SELECT c.relowner, a.grantee
    FROM pg_class c CROSS JOIN LATERAL aclexplode(c.relacl) a
    WHERE c.oid = ANY($1)
    UNION ALL
    SELECT c.relowner, a.grantee
    FROM pg_class c
    JOIN pg_attribute att ON att.attrelid = c.oid AND att.attnum > 0 AND NOT att.attisdropped
    CROSS JOIN LATERAL aclexplode(att.attacl) a
    WHERE c.oid = ANY($1)
)
SELECT r.rolname::text, r.rolsuper, r.rolbypassrls, bool_and(g.grantee = g.relowner)
FROM grants g
JOIN pg_roles r ON r.oid = g.grantee
WHERE g.grantee <> 0
GROUP BY r.rolname, r.rolsuper, r.rolbypassrls
ORDER BY 1";

/// Reads the tables in `schemas` and the roles holding privileges on them, in a rolled-back
/// transaction. With a preset, the preset decides which identities to scaffold.
pub fn introspect(
    target: &Target,
    schemas: &[String],
    preset: Option<Preset>,
) -> Result<Scaffold, InitError> {
    let mut client = pg::connect(target)?;
    let mut session = Session::begin(&mut client, &Safety::default())?;
    let scaffold = scaffold(session.tx(), schemas, preset)?;
    session.rollback()?;
    if scaffold.tables.is_empty() {
        return Err(InitError::NoTables(schemas.to_vec()));
    }
    if scaffold.identities.is_empty() {
        return Err(InitError::NoRoles(schemas.to_vec()));
    }
    Ok(scaffold)
}

fn scaffold(
    tx: &mut Transaction,
    schemas: &[String],
    preset: Option<Preset>,
) -> Result<Scaffold, InitError> {
    let catalog = catalog::load(tx, schemas)?;
    let mut left_out = Vec::new();
    let mut tables = Vec::new();
    for table in catalog.tables() {
        // A spec reads `a.b` as schema `a`, table `b`: such a name can't be written yet.
        if table.name.contains('.') {
            left_out.push((
                format!("table {}.{}", table.schema, table.name),
                "its name contains a dot, which a spec can't express yet",
            ));
        } else {
            tables.push(catalog.display_name(table));
        }
    }
    let oids: Vec<u32> = catalog.tables().iter().map(|t| t.oid).collect();
    let grantees: Vec<Grantee> = tx
        .query(ROLES, &[&oids])?
        .iter()
        .map(|row| Grantee {
            role: row.get(0),
            superuser: row.get(1),
            bypassrls: row.get(2),
            owner_only: row.get(3),
        })
        .collect();
    let (rationale, identities) = match preset {
        None => (
            "One identity per role holding a privilege on a table in scope (PUBLIC excluded).",
            plain(&grantees, &mut left_out),
        ),
        Some(preset) => {
            let needed = preset.roles();
            let existing: Vec<String> = tx
                .query(
                    "SELECT rolname::text FROM pg_roles WHERE rolname = ANY($1)",
                    &[&needed],
                )?
                .iter()
                .map(|row| row.get(0))
                .collect();
            let missing: Vec<_> = needed
                .iter()
                .copied()
                .filter(|role| !existing.iter().any(|e| e == role))
                .collect();
            if !missing.is_empty() {
                return Err(InitError::MissingRoles {
                    preset,
                    needed: needed.to_vec(),
                    missing,
                });
            }
            preset.scaffold(&grantees, &mut left_out)
        }
    };
    Ok(Scaffold {
        schemas: schemas.to_vec(),
        tables,
        preset,
        rationale,
        identities,
        left_out,
    })
}

fn plain(grantees: &[Grantee], left_out: &mut Vec<(String, &'static str)>) -> Vec<Identity> {
    let mut identities = Vec::new();
    for grantee in grantees {
        match grantee.unfit() {
            Some(reason) => left_out.push((format!("role {}", grantee.role), reason)),
            None => identities.push(Identity {
                name: grantee.role.clone(),
                role: grantee.role.clone(),
                claims: None,
                todo: Some(GUCS_TODO),
            }),
        }
    }
    identities
}

/// The spec file: valid as is, with every identity × table × operation cell `todo`.
pub fn render(scaffold: &Scaffold) -> String {
    let schemas = scaffold
        .schemas
        .iter()
        .map(|s| scalar(s))
        .collect::<Vec<_>>()
        .join(", ");
    let mut out = format!(
        "# Generated by `rlsspec init` from {}.
# Every identity × table × operation is `todo`: replace each one with the access you intend, e.g.
# `select: deny`, `select: {{ rows: \"published\" }}` or `update: [{{ where: \"true\", expect: deny }}]`.
# `rlsspec cover` shows what is left; `rlsspec test` runs what is specified.
version: 1
{preset}
database:
  url: ${{env:DATABASE_URL}}
  schemas: [{schemas}]

# setup:                          # SQL files run first as the privileged role, always rolled back
#   - seed.sql                    # (the rows your checks need)

# {rationale}
",
        schemas_phrase(&scaffold.schemas),
        preset = scaffold.preset.map(preset_line).unwrap_or_default(),
        rationale = scaffold.rationale,
    );

    if !scaffold.left_out.is_empty() {
        out.push_str("# Left out:\n");
        for (what, reason) in &scaffold.left_out {
            let _ = writeln!(out, "#   {what}: {reason}");
        }
    }
    out.push_str("identities:\n");
    for identity in &scaffold.identities {
        let _ = writeln!(
            out,
            "  {}:\n    role: {}",
            scalar(&identity.name),
            scalar(&identity.role)
        );
        if let Some(claims) = &identity.claims {
            let _ = writeln!(out, "    claims: {claims}");
        }
        if let Some(todo) = identity.todo {
            let _ = writeln!(out, "    # TODO: {todo}");
        }
    }

    out.push_str(
        "
# Cells without a case are listed as unspecified. Use `fail` in CI once the spec is complete.
unspecified: warn

expect:
",
    );
    for table in &scaffold.tables {
        let _ = writeln!(out, "  {}:", scalar(table));
        for identity in &scaffold.identities {
            let _ = writeln!(
                out,
                "    {}: {{ select: todo, insert: todo, update: todo, delete: todo }}",
                scalar(&identity.name)
            );
        }
    }
    out
}

/// Writes `text` to `path`, never replacing an existing file unless `force`.
pub fn write(path: &Path, text: &str, force: bool) -> Result<(), InitError> {
    let file = if force {
        File::create(path)
    } else {
        OpenOptions::new().write(true).create_new(true).open(path)
    };
    let failed = |source: io::Error| match source.kind() {
        io::ErrorKind::AlreadyExists => InitError::Exists(path.to_path_buf()),
        _ => InitError::Write {
            path: path.to_path_buf(),
            source,
        },
    };
    file.and_then(|mut f| f.write_all(text.as_bytes()))
        .map_err(failed)
}

fn preset_line(preset: Preset) -> String {
    format!(
        "preset: {}                  # {}\n",
        preset.name(),
        preset.description()
    )
}

fn schemas_phrase(schemas: &[String]) -> String {
    let noun = if schemas.len() == 1 {
        "schema"
    } else {
        "schemas"
    };
    format!("{noun} {}", schemas.join(", "))
}

/// A YAML scalar for a name: plain when that can't be misread, double-quoted otherwise.
fn scalar(name: &str) -> String {
    const RESERVED: [&str; 9] = ["true", "false", "yes", "no", "on", "off", "y", "n", "null"];
    let mut chars = name.chars();
    let plain = chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'))
        && !RESERVED.contains(&name.to_ascii_lowercase().as_str());
    if plain {
        return name.to_owned();
    }
    let mut out = String::from("\"");
    for c in name.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            c if c.is_control() => {
                let _ = write!(out, "\\u{:04x}", u32::from(c));
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;
    use crate::config::{self, Spec};

    fn example() -> Scaffold {
        let grantee = |role: &str| Grantee {
            role: role.into(),
            superuser: false,
            bypassrls: false,
            owner_only: false,
        };
        let mut left_out = Vec::new();
        let identities = plain(
            &[grantee("app"), grantee("on"), grantee("we\"ird")],
            &mut left_out,
        );
        Scaffold {
            schemas: vec!["public".into()],
            tables: vec!["notes".into(), "Weird Table".into()],
            preset: None,
            rationale: "One identity per role.",
            identities,
            left_out: vec![(
                "role postgres".into(),
                "superuser, so no policy applies to it",
            )],
        }
    }

    #[test]
    fn names_are_quoted_when_needed() {
        assert_eq!(scalar("app_user"), "app_user");
        assert_eq!(scalar("app.notes"), "app.notes");
        assert_eq!(scalar("Off"), "\"Off\"");
        assert_eq!(scalar("9lives"), "\"9lives\"");
        assert_eq!(scalar("a b"), "\"a b\"");
        assert_eq!(scalar("we\"ird\\"), "\"we\\\"ird\\\\\"");
    }

    #[test]
    fn rendered_spec_round_trips_with_every_cell_todo() {
        let text = render(&example());
        let env = |name: &str| (name == "DATABASE_URL").then(|| "postgres://localhost/db".into());
        let config = config::parse(&text, Path::new("rlsspec.yaml"), &env).unwrap();
        let identities: Vec<_> = config.identities.iter().map(|i| i.role.as_str()).collect();
        assert_eq!(identities, ["app", "on", "we\"ird"]);
        assert_eq!(config.expect.len(), 6);
        for block in &config.expect {
            let ops = &block.ops;
            assert_eq!(ops.select, Some(Spec::Todo));
            assert_eq!(ops.insert, Some(Spec::Todo));
            assert_eq!(ops.update, Some(Spec::Todo));
            assert_eq!(ops.delete, Some(Spec::Todo));
        }
    }

    #[test]
    fn supabase_spec_round_trips_with_the_claims_expanded() {
        let grantee = |role: &str, bypassrls| Grantee {
            role: role.into(),
            superuser: false,
            bypassrls,
            owner_only: false,
        };
        let grantees = [
            grantee("anon", false),
            grantee("authenticated", false),
            grantee("reporting", false),
            grantee("service_role", true),
        ];
        let mut left_out = Vec::new();
        let (rationale, identities) = Preset::Supabase.scaffold(&grantees, &mut left_out);
        let names: Vec<_> = left_out.iter().map(|(what, _)| what.as_str()).collect();
        assert_eq!(names, ["role reporting", "role service_role"]);
        let text = render(&Scaffold {
            schemas: vec!["public".into()],
            tables: vec!["todos".into()],
            preset: Some(Preset::Supabase),
            rationale,
            identities,
            left_out,
        });
        let env = |name: &str| (name == "DATABASE_URL").then(|| "postgres://localhost/db".into());
        let config = config::parse(&text, Path::new("rlsspec.yaml"), &env).unwrap();
        let gucs: Vec<_> = config
            .identities
            .iter()
            .map(|i| {
                (
                    i.role.as_str(),
                    i.gucs[0].name.as_str(),
                    i.gucs[0].value.as_str(),
                )
            })
            .collect();
        assert_eq!(
            gucs,
            [
                ("anon", "request.jwt.claims", r#"{"role":"anon"}"#),
                (
                    "authenticated",
                    "request.jwt.claims",
                    r#"{"role":"authenticated"}"#
                ),
            ]
        );
    }
}
