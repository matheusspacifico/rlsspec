//! `preset: supabase`: an identity's `claims` become the GUCs PostgREST sets from the JWT, so
//! `auth.uid()`, `auth.role()` and `auth.jwt()` work in policies.

use serde_json::Value;

use crate::config::{Claims, Config, Diagnostic, Guc};

const CLAIMS_GUC: &str = "request.jwt.claims";
const CLAIM_GUC_PREFIX: &str = "request.jwt.claim.";

/// Each identity's `claims` become `request.jwt.claims` (the whole object as JSON) and, for every
/// top-level string claim, `request.jwt.claim.<name>`, which older `auth.uid()` versions read.
/// Without a `role` claim, the identity's role is used, as PostgREST's JWT would carry it.
pub fn expand(mut config: Config, claims: Vec<Claims>) -> Result<Config, Vec<Diagnostic>> {
    let mut diagnostics = Vec::new();
    for Claims {
        identity,
        value,
        span,
    } in claims
    {
        let Value::Object(mut map) = value else {
            diagnostics.push(Diagnostic {
                span,
                message:
                    "`claims` must be a mapping of claim names to values, e.g. `{ sub: \"…\" }`"
                        .into(),
            });
            continue;
        };
        // `resolve` makes one identity per name and `Claims` only for those identities.
        let Some(identity) = config.identities.iter_mut().find(|i| i.name == identity) else {
            continue;
        };
        map.entry("role")
            .or_insert_with(|| Value::String(identity.role.clone()));
        let mut gucs = vec![(
            CLAIMS_GUC.to_owned(),
            Value::Object(map.clone()).to_string(),
        )];
        for (name, value) in &map {
            if let Value::String(text) = value {
                gucs.push((format!("{CLAIM_GUC_PREFIX}{name}"), text.clone()));
            }
        }
        for (name, value) in gucs {
            if let Some(by_hand) = identity.gucs.iter().find(|g| g.name == name) {
                diagnostics.push(Diagnostic {
                    span: by_hand.span,
                    message: format!(
                        "`{name}` is set from `claims` by the supabase preset; remove it from `gucs`"
                    ),
                });
                continue;
            }
            identity.gucs.push(Guc { name, value, span });
        }
    }
    if diagnostics.is_empty() {
        Ok(config)
    } else {
        diagnostics.sort_by_key(|d| d.span);
        Err(diagnostics)
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use crate::config::{self, Config, ConfigError, Extensions};
    use crate::preset;

    const HEAD: &str = "version: 1\ndatabase: { url: postgres://localhost/db }\n";

    fn env(name: &str) -> Option<String> {
        (name == "USER_EMAIL").then(|| "ana@example.com".into())
    }

    fn unexpanded(body: &str) -> (Config, Extensions) {
        let text = format!("{HEAD}{body}");
        config::parse_unexpanded(&text, Path::new("rlsspec.yaml"), &env).unwrap()
    }

    fn parse(body: &str) -> Result<Config, ConfigError> {
        config::parse(&format!("{HEAD}{body}"), Path::new("rlsspec.yaml"), &env)
    }

    fn gucs(config: &Config, identity: &str) -> Vec<(String, String)> {
        let identity = config
            .identities
            .iter()
            .find(|i| i.name == identity)
            .unwrap();
        identity
            .gucs
            .iter()
            .map(|g| (g.name.clone(), g.value.clone()))
            .collect()
    }

    fn pairs(expected: &[(&str, &str)]) -> Vec<(String, String)> {
        expected
            .iter()
            .map(|(name, value)| (name.to_string(), value.to_string()))
            .collect()
    }

    #[test]
    fn claims_become_the_json_claims_and_one_guc_per_top_level_string() {
        let config = parse(
            r#"preset: supabase
vars:
  ana: 0e5e0000-0000-4000-8000-000000000a0a
identities:
  ana:
    role: authenticated
    gucs: { app.locale: fr }
    claims:
      sub: "${ana}"
      email: "${env:USER_EMAIL}"
      exp: 1700000000
      is_anonymous: false
      amr: [{ method: password, at: "${ana}" }]
      app_metadata: { plan: pro, "${ana}": "kept as a key" }
"#,
        )
        .unwrap();
        let json = concat!(
            r#"{"amr":[{"at":"0e5e0000-0000-4000-8000-000000000a0a","method":"password"}],"#,
            r#""app_metadata":{"${ana}":"kept as a key","plan":"pro"},"#,
            r#""email":"ana@example.com","exp":1700000000,"is_anonymous":false,"#,
            r#""role":"authenticated","sub":"0e5e0000-0000-4000-8000-000000000a0a"}"#
        );
        assert_eq!(
            gucs(&config, "ana"),
            pairs(&[
                ("app.locale", "fr"),
                ("request.jwt.claims", json),
                ("request.jwt.claim.email", "ana@example.com"),
                ("request.jwt.claim.role", "authenticated"),
                (
                    "request.jwt.claim.sub",
                    "0e5e0000-0000-4000-8000-000000000a0a"
                ),
            ])
        );
    }

    #[test]
    fn the_role_claim_defaults_to_the_identity_role_and_can_be_overridden() {
        let config = parse(
            "preset: supabase
identities:
  anon: { role: anon, claims: {} }
  service: { role: authenticated, claims: { role: service_role } }
  plain: { role: authenticated }
",
        )
        .unwrap();
        assert_eq!(
            gucs(&config, "anon"),
            pairs(&[
                ("request.jwt.claims", r#"{"role":"anon"}"#),
                ("request.jwt.claim.role", "anon"),
            ])
        );
        assert_eq!(
            gucs(&config, "service"),
            pairs(&[
                ("request.jwt.claims", r#"{"role":"service_role"}"#),
                ("request.jwt.claim.role", "service_role"),
            ])
        );
        assert_eq!(gucs(&config, "plain"), []);
    }

    #[test]
    fn a_config_without_the_preset_is_unchanged() {
        let (config, extensions) = unexpanded(
            "identities:
  ana: { role: app, gucs: { request.jwt.claim.sub: x, app.org: \"${env:USER_EMAIL}\" } }
",
        );
        assert_eq!(extensions, Extensions::default());
        assert_eq!(preset::apply(config.clone(), extensions).unwrap(), config);
    }

    #[test]
    fn the_preset_line_alone_changes_nothing() {
        let (plain, _) = unexpanded("identities:\n  ana: { role: app }\n");
        let (config, extensions) =
            unexpanded("identities:\n  ana: { role: app }\npreset: supabase\n");
        assert_eq!(preset::apply(config, extensions).unwrap(), plain);
    }

    #[test]
    fn claims_without_the_preset_are_located() {
        insta::assert_snapshot!(
            parse(
                "identities:
  ana:
    role: authenticated
    claims: { sub: x }
"
            )
            .unwrap_err()
        );
    }

    #[test]
    fn claims_that_are_not_a_mapping_are_located() {
        insta::assert_snapshot!(
            parse(
                "preset: supabase
identities:
  ana: { role: authenticated, claims: [sub, x] }
  ben: { role: authenticated, claims: sub }
"
            )
            .unwrap_err()
        );
    }

    #[test]
    fn claim_gucs_set_by_hand_are_located() {
        insta::assert_snapshot!(
            parse(
                "preset: supabase
identities:
  ana:
    role: authenticated
    gucs:
      request.jwt.claim.sub: x
      request.jwt.claims: '{}'
      request.jwt.claim.email: kept, not a claim here
    claims: { sub: x }
"
            )
            .unwrap_err()
        );
    }

    #[test]
    fn unknown_preset_is_located() {
        insta::assert_snapshot!(
            parse("preset: firebase\nidentities:\n  ana: { role: app }\n").unwrap_err()
        );
    }
}
