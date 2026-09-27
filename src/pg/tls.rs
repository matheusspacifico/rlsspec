use std::path::{Path, PathBuf};
use std::sync::Arc;

use rustls::client::WebPkiServerVerifier;
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::{self, CryptoProvider};
use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{
    CertificateError, ClientConfig, DigitallySignedStruct, RootCertStore, SignatureScheme,
};
use tokio_postgres_rustls::MakeRustlsConnect;

use super::target::{SslMode, Target};

#[derive(Debug, thiserror::Error)]
pub enum TlsError {
    #[error("cannot read sslrootcert {}: {detail}", path.display())]
    RootCert { path: PathBuf, detail: String },
    #[error("no certificate found in sslrootcert {}", .0.display())]
    NoRootCert(PathBuf),
    #[error("no trusted root certificate to verify the server with")]
    NoRoots,
    #[error("cannot set up TLS: {0}")]
    Setup(#[from] rustls::Error),
    #[error("cannot set up TLS: {0}")]
    Verifier(#[from] rustls::client::VerifierBuilderError),
}

/// The connector for `target`. The driver only uses it when `sslmode` isn't `disable`.
pub fn connector(target: &Target) -> Result<MakeRustlsConnect, TlsError> {
    let provider = Arc::new(crypto::ring::default_provider());
    let builder = ClientConfig::builder_with_provider(provider.clone())
        .with_safe_default_protocol_versions()?;
    let config = match target.ssl_mode {
        SslMode::Disable | SslMode::Prefer | SslMode::Require => builder
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(Unverified(provider)))
            .with_no_client_auth(),
        SslMode::VerifyCa => {
            let roots = Arc::new(roots(target.root_cert.as_deref())?);
            let chain = WebPkiServerVerifier::builder_with_provider(roots, provider).build()?;
            builder
                .dangerous()
                .with_custom_certificate_verifier(Arc::new(AnyName(chain)))
                .with_no_client_auth()
        }
        SslMode::VerifyFull => builder
            .with_root_certificates(roots(target.root_cert.as_deref())?)
            .with_no_client_auth(),
    };
    Ok(MakeRustlsConnect::new(config))
}

/// `sslrootcert` when given, else Mozilla's roots plus the system's (a corporate CA lives there).
fn roots(root_cert: Option<&Path>) -> Result<RootCertStore, TlsError> {
    let mut roots = RootCertStore::empty();
    match root_cert {
        Some(path) => {
            let failed = |err: rustls::pki_types::pem::Error| TlsError::RootCert {
                path: path.to_path_buf(),
                detail: err.to_string(),
            };
            let certs = CertificateDer::pem_file_iter(path)
                .map_err(failed)?
                .collect::<Result<Vec<_>, _>>()
                .map_err(failed)?;
            if certs.is_empty() {
                return Err(TlsError::NoRootCert(path.to_path_buf()));
            }
            for cert in certs {
                roots.add(cert).map_err(|err| TlsError::RootCert {
                    path: path.to_path_buf(),
                    detail: err.to_string(),
                })?;
            }
        }
        None => {
            roots.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
            // A store that can't be read only leaves Mozilla's roots, like a system without one.
            let native = rustls_native_certs::load_native_certs();
            roots.add_parsable_certificates(native.certs);
        }
    }
    if roots.is_empty() {
        return Err(TlsError::NoRoots);
    }
    Ok(roots)
}

/// `prefer` and `require`: encrypted, but, like libpq, any certificate is accepted. The handshake
/// signatures are still checked, so the server must hold the certificate's key.
#[derive(Debug)]
struct Unverified(Arc<CryptoProvider>);

impl ServerCertVerifier for Unverified {
    fn verify_server_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        crypto::verify_tls12_signature(
            message,
            cert,
            dss,
            &self.0.signature_verification_algorithms,
        )
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        crypto::verify_tls13_signature(
            message,
            cert,
            dss,
            &self.0.signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.0.signature_verification_algorithms.supported_schemes()
    }
}

/// `verify-ca`: the chain must lead to a trusted root, whatever host name the certificate is for.
#[derive(Debug)]
struct AnyName(Arc<WebPkiServerVerifier>);

impl ServerCertVerifier for AnyName {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        server_name: &ServerName<'_>,
        ocsp_response: &[u8],
        now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        // The chain (and revocation) is checked before the name, so a name error means the chain
        // itself was valid.
        match self
            .0
            .verify_server_cert(end_entity, intermediates, server_name, ocsp_response, now)
        {
            Err(rustls::Error::InvalidCertificate(
                CertificateError::NotValidForName | CertificateError::NotValidForNameContext { .. },
            )) => Ok(ServerCertVerified::assertion()),
            other => other,
        }
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        self.0.verify_tls12_signature(message, cert, dss)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        self.0.verify_tls13_signature(message, cert, dss)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.0.supported_verify_schemes()
    }
}
