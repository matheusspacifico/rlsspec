use std::net::IpAddr;

use postgres::config::Host;

#[derive(Debug, thiserror::Error)]
pub enum SafetyError {
    #[error("`database.url` is not a valid connection string")]
    InvalidUrl(#[source] postgres::Error),
    #[error(
        "refusing to connect to non-local host `{0}`; list it under `safety.allowed_hosts` or pass --allow-remote"
    )]
    RemoteHost(String),
}

pub fn check(url: &str, allowed_hosts: &[String], allow_remote: bool) -> Result<(), SafetyError> {
    let config: postgres::Config = url.parse().map_err(SafetyError::InvalidUrl)?;
    if allow_remote {
        return Ok(());
    }
    let allowed =
        |host: &str| is_local(host) || allowed_hosts.iter().any(|a| a.eq_ignore_ascii_case(host));
    for host in config.get_hosts() {
        match host {
            Host::Tcp(name) if !allowed(name) => {
                return Err(SafetyError::RemoteHost(name.clone()));
            }
            _ => {}
        }
    }
    // `hostaddr` overrides where the connection actually goes, whatever `host` says.
    for addr in config.get_hostaddrs() {
        if !allowed(&addr.to_string()) {
            return Err(SafetyError::RemoteHost(addr.to_string()));
        }
    }
    Ok(())
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

    fn refused(url: &str, allowed: &[&str]) -> Option<String> {
        let allowed: Vec<String> = allowed.iter().map(|s| s.to_string()).collect();
        match check(url, &allowed, false) {
            Ok(()) => None,
            Err(SafetyError::RemoteHost(host)) => Some(host),
            Err(err) => panic!("unexpected error: {err}"),
        }
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
    fn allow_remote_skips_the_host_check() {
        assert!(check("postgres://db.example.com/db", &[], true).is_ok());
    }

    #[test]
    fn invalid_urls_are_rejected_even_with_allow_remote() {
        assert!(matches!(
            check("postgres://localhost:notaport/db", &[], true),
            Err(SafetyError::InvalidUrl(_))
        ));
    }
}
