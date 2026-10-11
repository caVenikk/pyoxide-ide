//! TLS check for diagnostics: can a TLS session be established, which
//! version, and is the certificate trusted for the expected name.
//! PostgreSQL negotiates TLS with an SSLRequest message first; Oracle TCPS
//! starts TLS right after the TCP connect.

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::{CryptoProvider, verify_tls12_signature, verify_tls13_signature};
use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{
    CertificateError, ClientConfig, DigitallySignedStruct, RootCertStore, SignatureScheme,
};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio_rustls::TlsConnector;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// PostgreSQL answered the SSLRequest with "no".
    NotOffered,
    /// No TLS session at all.
    Failed(String),
    /// Session established and the certificate verified for the name.
    Verified { version: String },
    /// Session established only without verification.
    Untrusted {
        version: String,
        reason: String,
        /// The chain is trusted; only the host name does not match.
        name_mismatch_only: bool,
    },
}

fn provider() -> Arc<CryptoProvider> {
    Arc::new(rustls::crypto::ring::default_provider())
}

/// OS trust store plus extra CA certificates from a PEM file and from an
/// Oracle wallet's `ewallet.pem`.
fn roots(ca_file: Option<&Path>, wallet_dir: Option<&Path>) -> RootCertStore {
    let mut store = RootCertStore::empty();
    for cert in rustls_native_certs::load_native_certs().certs {
        store.add(cert).ok();
    }
    let extra = ca_file
        .map(Path::to_path_buf)
        .into_iter()
        .chain(wallet_dir.map(|d| d.join("ewallet.pem")));
    for path in extra {
        if let Ok(iter) = CertificateDer::pem_file_iter(&path) {
            for cert in iter.flatten() {
                store.add(cert).ok();
            }
        }
    }
    store
}

/// Accepts any certificate; used only to tell "no TLS" from "untrusted TLS".
#[derive(Debug)]
struct AcceptAny(Arc<CryptoProvider>);

impl ServerCertVerifier for AcceptAny {
    fn verify_server_cert(
        &self,
        _: &CertificateDer<'_>,
        _: &[CertificateDer<'_>],
        _: &ServerName<'_>,
        _: &[u8],
        _: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        verify_tls12_signature(
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
        verify_tls13_signature(
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

fn config(verify: bool, roots: RootCertStore) -> Result<ClientConfig, String> {
    let builder = ClientConfig::builder_with_provider(provider())
        .with_safe_default_protocol_versions()
        .map_err(|e| e.to_string())?;
    Ok(if verify {
        builder.with_root_certificates(roots).with_no_client_auth()
    } else {
        builder
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(AcceptAny(provider())))
            .with_no_client_auth()
    })
}

/// PostgreSQL SSLRequest: length 8, code 80877103. The answer is one byte.
async fn pg_ssl_request(stream: &mut TcpStream) -> std::io::Result<bool> {
    let mut msg = [0u8; 8];
    msg[..4].copy_from_slice(&8u32.to_be_bytes());
    msg[4..].copy_from_slice(&80_877_103u32.to_be_bytes());
    stream.write_all(&msg).await?;
    let mut answer = [0u8; 1];
    stream.read_exact(&mut answer).await?;
    Ok(answer[0] == b'S')
}

async fn handshake(
    mut stream: TcpStream,
    postgres: bool,
    name: &ServerName<'static>,
    cfg: ClientConfig,
) -> Result<Result<String, rustls::Error>, Outcome> {
    if postgres {
        match pg_ssl_request(&mut stream).await {
            Ok(true) => {}
            Ok(false) => return Err(Outcome::NotOffered),
            Err(e) => return Err(Outcome::Failed(format!("SSLRequest: {e}"))),
        }
    }
    let connector = TlsConnector::from(Arc::new(cfg));
    match connector.connect(name.clone(), stream).await {
        Ok(tls) => {
            let version = tls
                .get_ref()
                .1
                .protocol_version()
                .map(|v| format!("{v:?}").replace('_', "."))
                .unwrap_or_else(|| "TLS".to_owned());
            Ok(Ok(version))
        }
        Err(e) => match e.get_ref().and_then(|i| i.downcast_ref::<rustls::Error>()) {
            Some(tls_err) => Ok(Err(tls_err.clone())),
            None => Err(Outcome::Failed(e.to_string())),
        },
    }
}

/// Probe TLS on an already connected stream. A second connection to the
/// same address is opened when the first handshake fails verification.
pub async fn probe(
    stream: TcpStream,
    postgres: bool,
    server_name: &str,
    ca_file: Option<&Path>,
    wallet_dir: Option<&Path>,
    timeout: Duration,
) -> Outcome {
    let addr = match stream.peer_addr() {
        Ok(a) => a,
        Err(e) => return Outcome::Failed(e.to_string()),
    };
    let name = match ServerName::try_from(server_name.to_owned()) {
        Ok(n) => n,
        Err(e) => return Outcome::Failed(format!("invalid server name {server_name:?}: {e}")),
    };
    let verified = match config(true, roots(ca_file, wallet_dir)) {
        Ok(c) => c,
        Err(e) => return Outcome::Failed(e),
    };
    let first = tokio::time::timeout(timeout, handshake(stream, postgres, &name, verified)).await;
    let reason = match first {
        Err(_) => return Outcome::Failed(format!("no TLS answer within {} s", timeout.as_secs())),
        Ok(Err(outcome)) => return outcome,
        Ok(Ok(Ok(version))) => return Outcome::Verified { version },
        Ok(Ok(Err(e))) => e,
    };
    let name_mismatch_only = matches!(
        reason,
        rustls::Error::InvalidCertificate(CertificateError::NotValidForName)
            | rustls::Error::InvalidCertificate(CertificateError::NotValidForNameContext { .. })
    );
    if !matches!(reason, rustls::Error::InvalidCertificate(_)) {
        return Outcome::Failed(reason.to_string());
    }
    let retry = async {
        let stream = TcpStream::connect(addr)
            .await
            .map_err(|e| Outcome::Failed(e.to_string()))?;
        let cfg = config(false, RootCertStore::empty()).map_err(Outcome::Failed)?;
        handshake(stream, postgres, &name, cfg).await
    };
    match tokio::time::timeout(timeout, retry).await {
        Ok(Ok(Ok(version))) => Outcome::Untrusted {
            version,
            reason: reason.to_string(),
            name_mismatch_only,
        },
        Ok(Ok(Err(e))) => Outcome::Failed(e.to_string()),
        Ok(Err(outcome)) => outcome,
        Err(_) => Outcome::Failed(format!("no TLS answer within {} s", timeout.as_secs())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn plain_server_fails_tls() {
        // A listener that answers garbage instead of a TLS handshake.
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            if let Ok((mut s, _)) = listener.accept().await {
                let _ = s.write_all(b"not tls at all\r\n").await;
            }
        });
        let stream = TcpStream::connect(addr).await.unwrap();
        let out = probe(
            stream,
            false,
            "localhost",
            None,
            None,
            Duration::from_secs(2),
        )
        .await;
        assert!(matches!(out, Outcome::Failed(_)), "{out:?}");
    }

    #[tokio::test]
    async fn postgres_without_tls_is_not_offered() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            if let Ok((mut s, _)) = listener.accept().await {
                let mut req = [0u8; 8];
                let _ = s.read_exact(&mut req).await;
                let _ = s.write_all(b"N").await;
            }
        });
        let stream = TcpStream::connect(addr).await.unwrap();
        let out = probe(
            stream,
            true,
            "localhost",
            None,
            None,
            Duration::from_secs(2),
        )
        .await;
        assert_eq!(out, Outcome::NotOffered);
    }
}
