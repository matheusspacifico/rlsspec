//! Presets: platform-specific sugar, applied as a config → config transform before anything runs.
//! After `apply`, identities are plain role + GUCs; the engine never sees a preset.

pub mod supabase;

use crate::config::{Config, Diagnostic, Extensions};
use crate::init::{Grantee, Identity};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Preset {
    Supabase,
}

impl Preset {
    pub const ALL: [Preset; 1] = [Preset::Supabase];

    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|preset| preset.name() == name)
    }

    pub fn name(self) -> &'static str {
        match self {
            Preset::Supabase => "supabase",
        }
    }

    /// What the preset does, for the comment next to `preset:` in a scaffolded spec.
    pub fn description(self) -> &'static str {
        match self {
            Preset::Supabase => "identities' `claims` become the JWT settings auth.uid() reads",
        }
    }

    /// The roles `init` scaffolds identities for; the database must have them all.
    pub fn roles(self) -> &'static [&'static str] {
        match self {
            Preset::Supabase => &supabase::ROLES,
        }
    }

    /// The identities `init` writes, with a comment saying why those, from the roles holding
    /// privileges on the tables in scope. Roles it leaves out go to `left_out` with the reason.
    pub fn scaffold(
        self,
        grantees: &[Grantee],
        left_out: &mut Vec<(String, &'static str)>,
    ) -> (&'static str, Vec<Identity>) {
        match self {
            Preset::Supabase => supabase::scaffold(grantees, left_out),
        }
    }
}

pub fn apply(config: Config, extensions: Extensions) -> Result<Config, Vec<Diagnostic>> {
    let preset = match extensions.preset {
        None => None,
        Some((name, span)) => match Preset::from_name(&name) {
            Some(preset) => Some(preset),
            None => {
                let known: Vec<_> = Preset::ALL
                    .iter()
                    .map(|p| format!("`{}`", p.name()))
                    .collect();
                return Err(vec![Diagnostic {
                    span,
                    message: format!("unknown preset `{name}`; expected {}", known.join(", ")),
                }]);
            }
        },
    };
    match preset {
        Some(Preset::Supabase) => supabase::expand(config, extensions.claims),
        None if extensions.claims.is_empty() => Ok(config),
        None => Err(extensions
            .claims
            .iter()
            .map(|claims| Diagnostic {
                span: claims.span,
                message: "`claims` needs a preset that reads them; add `preset: supabase` at the top level"
                    .into(),
            })
            .collect()),
    }
}
