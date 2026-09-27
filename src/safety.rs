use std::net::IpAddr;

use postgres::config::Host;

use crate::pg::{SslMode, Target};

#[derive(Debug, thiserror::Error)]
pub enum SafetyError {
    #[error(
        "refusing to connect to non-local host `{0}`; list it under `safety.allowed_hosts` or pass --allow-remote"
    )]
    RemoteHost(String),
    #[error(
        "refusing to connect to non-local host `{0}` without TLS (sslmode=disable); use sslmode=require or stronger, or pass --allow-insecure"
    )]
    Insecure(String),
}

/// What the user explicitly allowed on the command line.
#[derive(Debug, Clone, Copy, Default)]
pub struct Allow {
    pub remote: bool,
    pub insecure: bool,
}

/// Checks where `target` goes. A local host is accepted as is; any other host must be listed in
/// `allowed_hosts` (or `--allow-remote`) and use TLS: `prefer` becomes `require`, `disable` is
/// refused, unless `--allow-insecure`. Returns the target to connect to and one-line warnings for
/// every override actually used.
pub fn check(
    mut target: Target,
    allowed_hosts: &[String],
    allow: Allow,
) -> Result<(Target, Vec<String>), SafetyError> {
    let mut warnings = Vec::new();
    let remote = remote_hosts(&target.config);
    if let Some(host) = remote
        .iter()
        .find(|host| !allowed_hosts.iter().any(|a| a.eq_ignore_ascii_case(host)))
    {
        if !allow.remote {
            return Err(SafetyError::RemoteHost(host.clone()));
        }
        warnings.push(format!(
            "connecting to non-local host `{host}` (--allow-remote)"
        ));
    }
    let Some(host) = remote.first() else {
        return Ok((target, warnings));
    };
    match (target.ssl_mode, allow.insecure) {
        (SslMode::Disable, false) => return Err(SafetyError::Insecure(host.clone())),
        (SslMode::Disable, true) => warnings.push(format!(
            "connecting to non-local host `{host}` without TLS (--allow-insecure)"
        )),
        (SslMode::Prefer, false) => target.require_tls(),
        (SslMode::Prefer, true) => warnings.push(format!(
            "connecting to non-local host `{host}` without TLS if it doesn't offer it (sslmode=prefer, --allow-insecure)"
        )),
        (SslMode::Require | SslMode::VerifyCa | SslMode::VerifyFull, _) => {}
    }
    Ok((target, warnings))
}

/// The hosts that aren't local, in order: `host`, then `hostaddr`, which overrides where the
/// connection actually goes, whatever `host` says.
fn remote_hosts(config: &postgres::Config) -> Vec<String> {
    let hosts = config.get_hosts().iter().filter_map(|host| match host {
        Host::Tcp(name) => Some(name.clone()),
        _ => None,
    });
    let addrs = config.get_hostaddrs().iter().map(IpAddr::to_string);
    let mut remote: Vec<String> = hosts.chain(addrs).filter(|h| !is_local(h)).collect();
    remote.dedup();
    remote
}

fn is_local(host: &str) -> bool {
    if host.eq_ignore_ascii_case("localhost") || host.starts_with('/') {
        return true;
    }
    let bare = host.trim_start_matches('[').trim_end_matches(']');
    bare.parse::<IpAddr>().is_ok_and(|ip| ip.is_loopback())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn allowed(hosts: &[&str]) -> Vec<String> {
        hosts.iter().map(|s| s.to_string()).collect()
    }

    fn refused(url: &str, hosts: &[&str]) -> Option<String> {
        let target = Target::parse(url).unwrap();
        match check(target, &allowed(hosts), Allow::default()) {
            Ok(_) => None,
            Err(SafetyError::RemoteHost(host)) => Some(host),
            Err(err) => panic!("unexpected error: {err}"),
        }
    }

    fn checked(url: &str, hosts: &[&str], allow: Allow) -> Result<(SslMode, Vec<String>), String> {
        let target = Target::parse(url).unwrap();
        check(target, &allowed(hosts), allow)
            .map(|(target, warnings)| (target.ssl_mode, warnings))
            .map_err(|err| err.to_string())
    }

    #[test]
    fn local_hosts_are_accepted() {
        for url in [
            "postgres://u:p@localhost:5432/db",
            "postgres://LOCALHOST/db",
            "postgres://127.0.0.1/db",
            "postgres://127.0.0.2/db",
            "postgres://[::1]:5432/db",
            "postgres:///db?host=/var/run/postgresql",
            "host=/tmp dbname=db",
            "host=localhost,127.0.0.1 dbname=db",
            "postgres://localhost/db?hostaddr=127.0.0.1",
        ] {
            assert_eq!(refused(url, &[]), None, "{url}");
        }
    }

    #[test]
    fn remote_hosts_are_refused() {
        assert_eq!(
            refused("postgres://u:p@db.example.com/db", &[]),
            Some("db.example.com".into())
        );
        assert_eq!(
            refused("host=localhost,10.0.0.5 dbname=db", &[]),
            Some("10.0.0.5".into())
        );
        assert_eq!(
            refused("postgres://localhost/db?hostaddr=10.1.2.3", &[]),
            Some("10.1.2.3".into())
        );
    }

    #[test]
    fn allowed_hosts_are_accepted() {
        assert_eq!(refused("postgres://db.internal/db", &["DB.internal"]), None);
        assert_eq!(
            refused("postgres://localhost/db?hostaddr=10.1.2.3", &["10.1.2.3"]),
            None
        );
    }

    #[test]
    fn allow_remote_skips_the_host_check_with_a_warning() {
        let allow = Allow {
            remote: true,
            ..Allow::default()
        };
        assert_eq!(
            checked("postgres://db.example.com/db", &[], allow),
            Ok((
                SslMode::Require,
                vec!["connecting to non-local host `db.example.com` (--allow-remote)".into()]
            ))
        );
        let listed = checked("postgres://db.example.com/db", &["db.example.com"], allow);
        assert_eq!(listed, Ok((SslMode::Require, vec![])));
    }

    #[test]
    fn local_hosts_keep_their_sslmode() {
        for mode in ["disable", "prefer"] {
            let url = format!("postgres://localhost/db?sslmode={mode}");
            let (checked_mode, warnings) = checked(&url, &[], Allow::default()).unwrap();
            assert_eq!(checked_mode.as_str(), mode);
            assert_eq!(warnings, Vec::<String>::new());
        }
    }

    #[test]
    fn remote_hosts_need_tls() {
        let hosts = &["db.internal"];
        assert_eq!(
            checked("postgres://db.internal/db?sslmode=disable", hosts, Allow::default()),
            Err("refusing to connect to non-local host `db.internal` without TLS (sslmode=disable); use sslmode=require or stronger, or pass --allow-insecure".into())
        );
        assert_eq!(
            checked("postgres://db.internal/db", hosts, Allow::default()),
            Ok((SslMode::Require, vec![]))
        );
        for mode in ["require", "verify-ca", "verify-full"] {
            let url = format!("postgres://db.internal/db?sslmode={mode}");
            let (checked_mode, warnings) = checked(&url, hosts, Allow::default()).unwrap();
            assert_eq!(checked_mode.as_str(), mode);
            assert_eq!(warnings, Vec::<String>::new());
        }
        // A remote hostaddr decides, even behind a local host name.
        assert!(
            checked(
                "postgres://localhost/db?hostaddr=10.1.2.3&sslmode=disable",
                &["10.1.2.3"],
                Allow::default()
            )
            .is_err()
        );
    }

    #[test]
    fn allow_insecure_keeps_the_sslmode_with_a_warning() {
        let allow = Allow {
            insecure: true,
            ..Allow::default()
        };
        let hosts = &["db.internal"];
        assert_eq!(
            checked("postgres://db.internal/db?sslmode=disable", hosts, allow),
            Ok((
                SslMode::Disable,
                vec![
                    "connecting to non-local host `db.internal` without TLS (--allow-insecure)"
                        .into()
                ]
            ))
        );
        assert_eq!(
            checked("postgres://db.internal/db", hosts, allow),
            Ok((
                SslMode::Prefer,
                vec!["connecting to non-local host `db.internal` without TLS if it doesn't offer it (sslmode=prefer, --allow-insecure)".into()]
            ))
        );
        assert_eq!(
            checked("postgres://localhost/db?sslmode=disable", &[], allow),
            Ok((SslMode::Disable, vec![]))
        );
    }
}
