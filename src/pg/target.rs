use std::fmt;
use std::iter::Peekable;
use std::path::PathBuf;
use std::str::CharIndices;

/// libpq's `sslmode`, all five modes that can be honoured exactly. `allow` can't (the driver never
/// retries with TLS after a plain connection fails), so it's refused rather than approximated.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SslMode {
    Disable,
    Prefer,
    Require,
    VerifyCa,
    VerifyFull,
}

impl SslMode {
    fn from_name(name: &str) -> Option<Self> {
        match name {
            "disable" => Some(Self::Disable),
            "prefer" => Some(Self::Prefer),
            "require" => Some(Self::Require),
            "verify-ca" => Some(Self::VerifyCa),
            "verify-full" => Some(Self::VerifyFull),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Disable => "disable",
            Self::Prefer => "prefer",
            Self::Require => "require",
            Self::VerifyCa => "verify-ca",
            Self::VerifyFull => "verify-full",
        }
    }

    /// Whether a connection in this mode can only ever be encrypted.
    pub fn requires_tls(self) -> bool {
        !matches!(self, Self::Disable | Self::Prefer)
    }

    pub fn verifies(self) -> bool {
        matches!(self, Self::VerifyCa | Self::VerifyFull)
    }
}

impl fmt::Display for SslMode {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, thiserror::Error)]
pub enum TargetError {
    #[error("`database.url` is not a valid connection string")]
    InvalidUrl(#[source] postgres::Error),
    #[error(
        "`sslmode={0}` is not supported; use disable, prefer, require, verify-ca or verify-full"
    )]
    SslMode(String),
    #[error("`sslrootcert` only applies to sslmode=verify-ca or verify-full, not {0}")]
    RootCertWithoutVerify(SslMode),
}

/// Where to connect: the driver's config, plus the TLS settings it can't parse itself.
#[derive(Debug, Clone)]
pub struct Target {
    pub config: postgres::Config,
    pub ssl_mode: SslMode,
    /// `sslrootcert`: the CA certificates (PEM) that replace the default roots; `system` keeps them.
    pub root_cert: Option<PathBuf>,
}

impl Target {
    pub fn parse(url: &str) -> Result<Self, TargetError> {
        let (rest, params) = if is_url(url) {
            split_url(url)
        } else {
            split_key_value(url)
        };
        let mut ssl_mode = SslMode::Prefer;
        let mut root_cert = None;
        // Like libpq, a key given twice takes its last value.
        for (key, value) in params {
            match key.as_str() {
                "sslmode" => {
                    ssl_mode = SslMode::from_name(&value).ok_or(TargetError::SslMode(value))?;
                }
                _ => root_cert = Some(value),
            }
        }
        if root_cert.is_some() && !ssl_mode.verifies() {
            return Err(TargetError::RootCertWithoutVerify(ssl_mode));
        }
        let root_cert = root_cert.filter(|path| path != "system").map(PathBuf::from);
        let mut config: postgres::Config = rest.parse().map_err(TargetError::InvalidUrl)?;
        config.ssl_mode(match ssl_mode {
            SslMode::Disable => postgres::config::SslMode::Disable,
            SslMode::Prefer => postgres::config::SslMode::Prefer,
            SslMode::Require | SslMode::VerifyCa | SslMode::VerifyFull => {
                postgres::config::SslMode::Require
            }
        });
        Ok(Self {
            config,
            ssl_mode,
            root_cert,
        })
    }
}

const TLS_KEYS: [&str; 2] = ["sslmode", "sslrootcert"];

fn is_url(url: &str) -> bool {
    url.starts_with("postgres://") || url.starts_with("postgresql://")
}

/// The URL without its `sslmode`/`sslrootcert` query parameters, and those parameters decoded.
fn split_url(url: &str) -> (String, Vec<(String, String)>) {
    let Some((base, query)) = url.split_once('?') else {
        return (url.to_owned(), Vec::new());
    };
    let mut kept = Vec::new();
    let mut taken = Vec::new();
    for pair in query.split('&') {
        let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
        let key = percent_decode(key);
        if TLS_KEYS.contains(&key.as_str()) {
            taken.push((key, percent_decode(value)));
        } else {
            kept.push(pair);
        }
    }
    let rest = if kept.is_empty() {
        base.to_owned()
    } else {
        format!("{base}?{}", kept.join("&"))
    };
    (rest, taken)
}

fn percent_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let hex = bytes
            .get(i + 1..i + 3)
            .and_then(|h| std::str::from_utf8(h).ok())
            .and_then(|h| u8::from_str_radix(h, 16).ok());
        match (bytes[i], hex) {
            (b'%', Some(byte)) => {
                out.push(byte);
                i += 3;
            }
            (byte, _) => {
                out.push(byte);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// The `key=value` connection string without its `sslmode`/`sslrootcert` pairs, and their values.
/// A string that doesn't parse is returned whole, for the driver to report.
fn split_key_value(text: &str) -> (String, Vec<(String, String)>) {
    let Some(pairs) = pairs(text) else {
        return (text.to_owned(), Vec::new());
    };
    let mut rest = String::with_capacity(text.len());
    let mut taken = Vec::new();
    let mut from = 0;
    for pair in pairs {
        if TLS_KEYS.contains(&pair.key.as_str()) {
            rest.push_str(&text[from..pair.start]);
            from = pair.end;
            taken.push((pair.key, pair.value));
        }
    }
    rest.push_str(&text[from..]);
    (rest, taken)
}

struct Pair {
    key: String,
    value: String,
    start: usize,
    end: usize,
}

/// libpq's `key = value` syntax: values are bare or single-quoted, with backslash escapes.
fn pairs(text: &str) -> Option<Vec<Pair>> {
    fn skip_space(chars: &mut Peekable<CharIndices>) {
        while chars.next_if(|(_, c)| c.is_whitespace()).is_some() {}
    }
    let mut chars = text.char_indices().peekable();
    let mut pairs = Vec::new();
    loop {
        skip_space(&mut chars);
        let Some(&(start, _)) = chars.peek() else {
            return Some(pairs);
        };
        let mut key = String::new();
        while let Some((_, c)) = chars.next_if(|(_, c)| *c != '=' && !c.is_whitespace()) {
            key.push(c);
        }
        skip_space(&mut chars);
        chars.next_if(|(_, c)| *c == '=')?;
        skip_space(&mut chars);
        let mut value = String::new();
        let mut end = chars.peek().map_or(text.len(), |&(i, _)| i);
        let quoted = chars.next_if(|(_, c)| *c == '\'').is_some();
        loop {
            let next = if quoted {
                chars.next()
            } else {
                chars.next_if(|(_, c)| !c.is_whitespace())
            };
            let Some((i, c)) = next else {
                if quoted {
                    return None;
                }
                break;
            };
            end = i + c.len_utf8();
            match c {
                '\\' => {
                    let (j, escaped) = chars.next()?;
                    end = j + escaped.len_utf8();
                    value.push(escaped);
                }
                '\'' if quoted => break,
                c => value.push(c),
            }
        }
        pairs.push(Pair {
            key,
            value,
            start,
            end,
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parsed(url: &str) -> Target {
        Target::parse(url).unwrap()
    }

    #[test]
    fn sslmode_defaults_to_prefer() {
        let target = parsed("postgres://u:p@localhost/db");
        assert_eq!(target.ssl_mode, SslMode::Prefer);
        assert_eq!(target.root_cert, None);
    }

    #[test]
    fn every_libpq_mode_but_allow_is_read_from_a_url() {
        for mode in ["disable", "prefer", "require", "verify-ca", "verify-full"] {
            let target = parsed(&format!("postgres://u@localhost/db?sslmode={mode}"));
            assert_eq!(target.ssl_mode.as_str(), mode);
            assert_eq!(target.config.get_dbname(), Some("db"));
        }
        assert!(matches!(
            Target::parse("postgres://localhost/db?sslmode=allow"),
            Err(TargetError::SslMode(mode)) if mode == "allow"
        ));
        assert!(matches!(
            Target::parse("host=localhost sslmode=bogus"),
            Err(TargetError::SslMode(_))
        ));
    }

    #[test]
    fn verify_modes_ask_the_driver_for_tls() {
        let target = parsed("postgres://localhost/db?sslmode=verify-full");
        assert_eq!(
            target.config.get_ssl_mode(),
            postgres::config::SslMode::Require
        );
        let target = parsed("postgres://localhost/db?sslmode=disable");
        assert_eq!(
            target.config.get_ssl_mode(),
            postgres::config::SslMode::Disable
        );
    }

    #[test]
    fn tls_parameters_are_taken_out_of_the_url() {
        let target = parsed(
            "postgresql://u@db.example.com:6543/app?application_name=x&sslmode=verify-ca&sslrootcert=%2Fetc%2Fca.pem&connect_timeout=5",
        );
        assert_eq!(target.ssl_mode, SslMode::VerifyCa);
        assert_eq!(target.root_cert, Some(PathBuf::from("/etc/ca.pem")));
        assert_eq!(target.config.get_application_name(), Some("x"));
        assert_eq!(
            target.config.get_connect_timeout(),
            Some(&std::time::Duration::from_secs(5))
        );
    }

    #[test]
    fn tls_parameters_are_taken_out_of_key_value_strings() {
        let target = parsed(
            "host=db.example.com sslmode = 'verify-full' sslrootcert='/tmp/my ca.pem' dbname=app",
        );
        assert_eq!(target.ssl_mode, SslMode::VerifyFull);
        assert_eq!(target.root_cert, Some(PathBuf::from("/tmp/my ca.pem")));
        assert_eq!(target.config.get_dbname(), Some("app"));

        let (rest, taken) = split_key_value(r"password=a\ b sslmode=require user=u");
        assert_eq!(rest, r"password=a\ b  user=u");
        assert_eq!(taken, [("sslmode".to_owned(), "require".to_owned())]);
    }

    #[test]
    fn the_last_sslmode_wins() {
        let target = parsed("postgres://localhost/db?sslmode=disable&sslmode=require");
        assert_eq!(target.ssl_mode, SslMode::Require);
    }

    #[test]
    fn sslrootcert_needs_a_verify_mode() {
        assert!(matches!(
            Target::parse("postgres://localhost/db?sslmode=require&sslrootcert=ca.pem"),
            Err(TargetError::RootCertWithoutVerify(SslMode::Require))
        ));
        assert!(matches!(
            Target::parse("postgres://localhost/db?sslrootcert=ca.pem"),
            Err(TargetError::RootCertWithoutVerify(SslMode::Prefer))
        ));
        let target = parsed("postgres://localhost/db?sslmode=verify-full&sslrootcert=system");
        assert_eq!(target.root_cert, None);
    }

    #[test]
    fn invalid_strings_are_still_reported_by_the_driver() {
        assert!(matches!(
            Target::parse("postgres://localhost:notaport/db"),
            Err(TargetError::InvalidUrl(_))
        ));
        assert!(matches!(
            Target::parse("host='unterminated sslmode=require"),
            Err(TargetError::InvalidUrl(_))
        ));
    }
}
