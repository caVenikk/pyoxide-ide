//! TLS setup: rustls with the `ring` provider, no OpenSSL, no aws-lc.
//!
//! The modes follow `sslmode` of libpq: `Prefer` and `Require` encrypt without
//! checking the certificate, `VerifyCa` checks the chain but not the host
//! name, `VerifyFull` checks both (against `server_name` when it is set).

use std::path::Path;
use std::sync::{Arc, OnceLock};

use db_core::{Error, ErrorKind, Result, TlsMode, TlsParams};
use rustls::client::WebPkiServerVerifier;
use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::CryptoProvider;
use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, PrivateKeyDer, ServerName, UnixTime};
use rustls::{
    CertificateError, ClientConfig, DigitallySignedStruct, Error as TlsError, RootCertStore,
    SignatureScheme,
};
use tokio_postgres::config::SslMode;
use tokio_postgres_rustls::MakeRustlsConnect;

/// The tokio-postgres `sslmode` for a [`TlsMode`]. Every verifying mode
/// requires TLS.
pub(crate) fn ssl_mode(mode: TlsMode) -> SslMode {
    match mode {
        TlsMode::Disable => SslMode::Disable,
        TlsMode::Prefer => SslMode::Prefer,
        TlsMode::Require | TlsMode::VerifyCa | TlsMode::VerifyFull => SslMode::Require,
    }
}

/// Which parts of the server certificate are checked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Verification {
    None,
    Chain,
    ChainAndName,
}

pub(crate) fn verification(mode: TlsMode) -> Verification {
    match mode {
        TlsMode::Disable | TlsMode::Prefer | TlsMode::Require => Verification::None,
        TlsMode::VerifyCa => Verification::Chain,
        TlsMode::VerifyFull => Verification::ChainAndName,
    }
}

fn provider() -> Arc<CryptoProvider> {
    Arc::new(rustls::crypto::ring::default_provider())
}

/// Build the connector for a connection (also used for cancel requests).
pub(crate) async fn connector(params: &TlsParams) -> Result<MakeRustlsConnect> {
    if verification(params.mode) != Verification::None {
        // The first use reads the OS trust store, which can take over 100 ms;
        // keep that off the async worker threads.
        let _ = tokio::task::spawn_blocking(native_roots).await;
    }
    build(params)
}

fn build(params: &TlsParams) -> Result<MakeRustlsConnect> {
    let provider = provider();
    let verifier: Arc<dyn ServerCertVerifier> = match verification(params.mode) {
        Verification::None => Arc::new(NoVerify {
            provider: provider.clone(),
        }),
        level => {
            let roots = root_store(params.ca_file.as_deref())?;
            Arc::new(ChainVerifier::new(
                roots,
                provider.clone(),
                level == Verification::ChainAndName,
                params.server_name.as_deref(),
            )?)
        }
    };

    let builder = ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .map_err(|e| Error::new(ErrorKind::Tls, format!("TLS setup failed: {e}")))?
        .dangerous()
        .with_custom_certificate_verifier(verifier);

    let config = match (&params.client_cert, &params.client_key) {
        (Some(cert), Some(key)) => {
            let chain = read_certs(cert, "client certificate")?;
            let key = PrivateKeyDer::from_pem_file(key).map_err(|e| {
                Error::config(format!("cannot read the client key {}: {e}", key.display()))
            })?;
            builder
                .with_client_auth_cert(chain, key)
                .map_err(|e| Error::new(ErrorKind::Tls, format!("invalid client key pair: {e}")))?
        }
        (None, None) => builder.with_no_client_auth(),
        _ => {
            return Err(Error::config(
                "a client certificate needs both client_cert and client_key",
            ));
        }
    };
    Ok(MakeRustlsConnect::new(config))
}

fn read_certs(path: &Path, what: &str) -> Result<Vec<CertificateDer<'static>>> {
    let certs: Vec<CertificateDer<'static>> = CertificateDer::pem_file_iter(path)
        .map_err(|e| {
            Error::config(format!(
                "cannot read the {what} file {}: {e}",
                path.display()
            ))
        })?
        .collect::<std::result::Result<_, _>>()
        .map_err(|e| Error::config(format!("invalid {what} in {}: {e}", path.display())))?;
    if certs.is_empty() {
        return Err(Error::config(format!(
            "no certificate found in the {what} file {}",
            path.display()
        )));
    }
    Ok(certs)
}

/// The OS trust store, loaded once per process (it can take a while).
fn native_roots() -> &'static RootCertStore {
    static ROOTS: OnceLock<RootCertStore> = OnceLock::new();
    ROOTS.get_or_init(|| {
        let mut store = RootCertStore::empty();
        let loaded = rustls_native_certs::load_native_certs();
        store.add_parsable_certificates(loaded.certs);
        store
    })
}

fn root_store(ca_file: Option<&Path>) -> Result<RootCertStore> {
    let mut store = native_roots().clone();
    if let Some(path) = ca_file {
        let certs = read_certs(path, "CA")?;
        let (_, rejected) = store.add_parsable_certificates(certs);
        if rejected > 0 {
            return Err(Error::config(format!(
                "{rejected} certificate(s) in {} could not be parsed",
                path.display()
            )));
        }
    }
    if store.is_empty() {
        return Err(Error::new(
            ErrorKind::Tls,
            "no trusted root certificates are available; set a CA file",
        ));
    }
    Ok(store)
}

/// Accepts any certificate (`Prefer` and `Require`), but still checks that
/// handshake signatures are valid.
#[derive(Debug)]
struct NoVerify {
    provider: Arc<CryptoProvider>,
}

impl ServerCertVerifier for NoVerify {
    fn verify_server_cert(
        &self,
        _end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        _now: UnixTime,
    ) -> std::result::Result<ServerCertVerified, TlsError> {
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, TlsError> {
        rustls::crypto::verify_tls12_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, TlsError> {
        rustls::crypto::verify_tls13_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.provider
            .signature_verification_algorithms
            .supported_schemes()
    }
}

/// Verifies the certificate chain against the trust roots. The host name is
/// checked only when `check_name` is set, and then against `name_override`
/// when there is one.
#[derive(Debug)]
pub(crate) struct ChainVerifier {
    inner: Arc<WebPkiServerVerifier>,
    check_name: bool,
    name_override: Option<ServerName<'static>>,
}

impl ChainVerifier {
    pub(crate) fn new(
        roots: RootCertStore,
        provider: Arc<CryptoProvider>,
        check_name: bool,
        name_override: Option<&str>,
    ) -> Result<Self> {
        let inner = WebPkiServerVerifier::builder_with_provider(Arc::new(roots), provider)
            .build()
            .map_err(|e| Error::new(ErrorKind::Tls, format!("TLS setup failed: {e}")))?;
        let name_override = name_override
            .map(|n| {
                ServerName::try_from(n.to_owned())
                    .map_err(|_| Error::config(format!("invalid TLS server name {n:?}")))
            })
            .transpose()?;
        Ok(Self {
            inner,
            check_name,
            name_override,
        })
    }
}

impl ServerCertVerifier for ChainVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        server_name: &ServerName<'_>,
        ocsp_response: &[u8],
        now: UnixTime,
    ) -> std::result::Result<ServerCertVerified, TlsError> {
        let name = self.name_override.as_ref().unwrap_or(server_name);
        match self
            .inner
            .verify_server_cert(end_entity, intermediates, name, ocsp_response, now)
        {
            // The chain is checked before the name, so this error means the
            // chain is fine and only the host name differs.
            Err(TlsError::InvalidCertificate(
                CertificateError::NotValidForName | CertificateError::NotValidForNameContext { .. },
            )) if !self.check_name => Ok(ServerCertVerified::assertion()),
            other => other,
        }
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, TlsError> {
        self.inner.verify_tls12_signature(message, cert, dss)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, TlsError> {
        self.inner.verify_tls13_signature(message, cert, dss)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.inner.supported_verify_schemes()
    }
}

#[cfg(test)]
mod tests {
    use rcgen::{
        BasicConstraints, CertificateParams, DnType, IsCa, KeyPair, KeyUsagePurpose, SanType,
    };

    use super::*;

    #[test]
    fn modes_map_to_tokio_postgres() {
        assert_eq!(ssl_mode(TlsMode::Disable), SslMode::Disable);
        assert_eq!(ssl_mode(TlsMode::Prefer), SslMode::Prefer);
        assert_eq!(ssl_mode(TlsMode::Require), SslMode::Require);
        assert_eq!(ssl_mode(TlsMode::VerifyCa), SslMode::Require);
        assert_eq!(ssl_mode(TlsMode::VerifyFull), SslMode::Require);
    }

    #[test]
    fn modes_map_to_verification_level() {
        assert_eq!(verification(TlsMode::Disable), Verification::None);
        assert_eq!(verification(TlsMode::Prefer), Verification::None);
        assert_eq!(verification(TlsMode::Require), Verification::None);
        assert_eq!(verification(TlsMode::VerifyCa), Verification::Chain);
        assert_eq!(
            verification(TlsMode::VerifyFull),
            Verification::ChainAndName
        );
    }

    struct Pki {
        ca_der: CertificateDer<'static>,
        ca_pem: String,
        leaf_der: CertificateDer<'static>,
    }

    fn make_pki(leaf_name: &str) -> Pki {
        let ca_key = KeyPair::generate().unwrap();
        let mut ca_params = CertificateParams::new(Vec::<String>::new()).unwrap();
        ca_params.is_ca = IsCa::Ca(BasicConstraints::Unconstrained);
        ca_params.key_usages = vec![KeyUsagePurpose::KeyCertSign];
        ca_params
            .distinguished_name
            .push(DnType::CommonName, "pyoxide test CA");
        let ca = ca_params.self_signed(&ca_key).unwrap();

        let leaf_key = KeyPair::generate().unwrap();
        let mut leaf_params = CertificateParams::new(Vec::<String>::new()).unwrap();
        leaf_params.subject_alt_names = vec![SanType::DnsName(leaf_name.try_into().unwrap())];
        let issuer = rcgen::Issuer::new(ca_params, ca_key);
        let leaf = leaf_params.signed_by(&leaf_key, &issuer).unwrap();

        Pki {
            ca_der: ca.der().clone(),
            ca_pem: ca.pem(),
            leaf_der: leaf.der().clone(),
        }
    }

    fn roots_with(ca: &CertificateDer<'static>) -> RootCertStore {
        let mut roots = RootCertStore::empty();
        roots.add(ca.clone()).unwrap();
        roots
    }

    fn name(n: &str) -> ServerName<'static> {
        ServerName::try_from(n.to_owned()).unwrap()
    }

    fn check(
        v: &ChainVerifier,
        pki: &Pki,
        host: &str,
    ) -> std::result::Result<ServerCertVerified, TlsError> {
        v.verify_server_cert(&pki.leaf_der, &[], &name(host), &[], UnixTime::now())
    }

    #[test]
    fn verify_full_checks_chain_and_name() {
        let pki = make_pki("db.example.test");
        let v = ChainVerifier::new(roots_with(&pki.ca_der), provider(), true, None).unwrap();
        assert!(check(&v, &pki, "db.example.test").is_ok());
        assert!(matches!(
            check(&v, &pki, "other.example.test"),
            Err(TlsError::InvalidCertificate(
                CertificateError::NotValidForName | CertificateError::NotValidForNameContext { .. }
            ))
        ));
    }

    #[test]
    fn verify_ca_ignores_the_host_name_only() {
        let pki = make_pki("db.example.test");
        let v = ChainVerifier::new(roots_with(&pki.ca_der), provider(), false, None).unwrap();
        assert!(check(&v, &pki, "db.example.test").is_ok());
        assert!(check(&v, &pki, "other.example.test").is_ok());

        // a certificate from another CA is still rejected, whatever the name
        let stranger = make_pki("db.example.test");
        for host in ["db.example.test", "other.example.test"] {
            let err = check(&v, &stranger, host).unwrap_err();
            assert!(
                matches!(err, TlsError::InvalidCertificate(_))
                    && !matches!(
                        err,
                        TlsError::InvalidCertificate(
                            CertificateError::NotValidForName
                                | CertificateError::NotValidForNameContext { .. }
                        )
                    ),
                "{host}: {err:?}"
            );
        }
    }

    #[test]
    fn server_name_override_replaces_the_expected_name() {
        let pki = make_pki("db.example.test");
        let v = ChainVerifier::new(
            roots_with(&pki.ca_der),
            provider(),
            true,
            Some("db.example.test"),
        )
        .unwrap();
        // the connection host is an address, the certificate names the service
        assert!(check(&v, &pki, "127.0.0.1").is_ok());

        let wrong = ChainVerifier::new(
            roots_with(&pki.ca_der),
            provider(),
            true,
            Some("elsewhere.example.test"),
        )
        .unwrap();
        assert!(check(&wrong, &pki, "db.example.test").is_err());
    }

    #[test]
    fn invalid_override_is_a_config_error() {
        let pki = make_pki("db.example.test");
        let e = ChainVerifier::new(
            roots_with(&pki.ca_der),
            provider(),
            true,
            Some("not a host!"),
        )
        .unwrap_err();
        assert_eq!(e.kind, ErrorKind::Config);
    }

    #[test]
    fn the_connector_builds_for_every_mode_and_reads_a_ca_file() {
        let pki = make_pki("db.example.test");
        let dir = std::env::temp_dir().join(format!("pxpg_tls_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let ca_path = dir.join("ca.pem");
        std::fs::write(&ca_path, &pki.ca_pem).unwrap();

        for mode in [
            TlsMode::Disable,
            TlsMode::Prefer,
            TlsMode::Require,
            TlsMode::VerifyCa,
            TlsMode::VerifyFull,
        ] {
            let params = TlsParams {
                mode,
                ca_file: Some(ca_path.clone()),
                ..TlsParams::default()
            };
            build(&params).unwrap_or_else(|e| panic!("{mode:?}: {e}"));
        }

        let missing = TlsParams {
            mode: TlsMode::VerifyFull,
            ca_file: Some(dir.join("missing.pem")),
            ..TlsParams::default()
        };
        assert_eq!(build(&missing).err().unwrap().kind, ErrorKind::Config);

        let empty = dir.join("empty.pem");
        std::fs::write(&empty, "not a certificate").unwrap();
        let params = TlsParams {
            mode: TlsMode::VerifyCa,
            ca_file: Some(empty),
            ..TlsParams::default()
        };
        assert_eq!(build(&params).err().unwrap().kind, ErrorKind::Config);

        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn client_cert_needs_both_files() {
        let params = TlsParams {
            mode: TlsMode::Require,
            client_cert: Some("client.pem".into()),
            ..TlsParams::default()
        };
        assert_eq!(build(&params).err().unwrap().kind, ErrorKind::Config);
    }
}
