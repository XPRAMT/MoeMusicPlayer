//! TLS trust for remote servers (ADR 0026): the default is a normal certificate check; a self-signed server is trusted
//! by **pinning the SHA-256 of its certificate** after the user has seen and accepted it once.

use std::io::Write;
use std::net::TcpStream;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::crypto::{verify_tls12_signature, verify_tls13_signature, CryptoProvider};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{ClientConfig, DigitallySignedStruct, SignatureScheme};
use sha2::{Digest, Sha256};

fn provider() -> Arc<CryptoProvider> {
    Arc::new(rustls::crypto::ring::default_provider())
}

pub fn sha256_hex(der: &[u8]) -> String {
    Sha256::digest(der)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

/// Accepts exactly one certificate, the one whose SHA-256 (hex) is `pin`. The handshake signatures are still checked
/// with the normal algorithms, so only the "is this the server I accepted" question is replaced.
#[derive(Debug)]
struct PinnedVerifier {
    pin: String,
    provider: Arc<CryptoProvider>,
}

impl ServerCertVerifier for PinnedVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _intermediates: &[CertificateDer<'_>],
        _server_name: &ServerName<'_>,
        _ocsp: &[u8],
        _now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        if sha256_hex(end_entity.as_ref()).eq_ignore_ascii_case(&self.pin) {
            Ok(ServerCertVerified::assertion())
        } else {
            Err(rustls::Error::General(
                "伺服器的憑證與你先前接受的不同".to_owned(),
            ))
        }
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
            &self.provider.signature_verification_algorithms,
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
            &self.provider.signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.provider
            .signature_verification_algorithms
            .supported_schemes()
    }
}

/// A client configuration that trusts only the pinned certificate (for `reqwest`'s `use_preconfigured_tls`).
pub fn pinned_client_config(pin_sha256_hex: &str) -> ClientConfig {
    let provider = provider();
    ClientConfig::builder_with_provider(Arc::clone(&provider))
        .with_safe_default_protocol_versions()
        .expect("the default protocol versions are valid")
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(PinnedVerifier {
            pin: pin_sha256_hex.to_owned(),
            provider,
        }))
        .with_no_client_auth()
}

/// Remembers the certificate the server presents and accepts anything: used only to *show* the user the fingerprint of a
/// self-signed server before they decide to pin it. No data is sent over such a connection.
#[derive(Debug)]
struct RecordingVerifier {
    seen: Mutex<Option<Vec<u8>>>,
    provider: Arc<CryptoProvider>,
}

impl ServerCertVerifier for RecordingVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        _: &[CertificateDer<'_>],
        _: &ServerName<'_>,
        _: &[u8],
        _: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        *self.seen.lock().unwrap() = Some(end_entity.as_ref().to_vec());
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
            &self.provider.signature_verification_algorithms,
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
            &self.provider.signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.provider
            .signature_verification_algorithms
            .supported_schemes()
    }
}

/// The SHA-256 (hex) of the certificate `host:port` presents, whether or not it is trusted.
pub fn probe_certificate_sha256(host: &str, port: u16) -> Result<String, String> {
    let provider = provider();
    let verifier = Arc::new(RecordingVerifier {
        seen: Mutex::new(None),
        provider: Arc::clone(&provider),
    });
    let config = ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .map_err(|error| error.to_string())?
        .dangerous()
        .with_custom_certificate_verifier(Arc::clone(&verifier) as Arc<dyn ServerCertVerifier>)
        .with_no_client_auth();
    let name = ServerName::try_from(host.to_owned()).map_err(|error| error.to_string())?;
    let mut connection =
        rustls::ClientConnection::new(Arc::new(config), name).map_err(|error| error.to_string())?;
    let address = std::net::ToSocketAddrs::to_socket_addrs(&(host, port))
        .map_err(|error| error.to_string())?
        .next()
        .ok_or_else(|| "找不到這個主機。".to_owned())?;
    let mut stream = TcpStream::connect_timeout(&address, Duration::from_secs(10))
        .map_err(|error| error.to_string())?;
    stream.set_read_timeout(Some(Duration::from_secs(10))).ok();
    stream.set_write_timeout(Some(Duration::from_secs(10))).ok();
    while connection.is_handshaking() {
        connection
            .complete_io(&mut stream)
            .map_err(|error| error.to_string())?;
    }
    connection.send_close_notify();
    let _ = connection.write_tls(&mut stream);
    let _ = stream.flush();
    let seen = verifier
        .seen
        .lock()
        .unwrap()
        .clone()
        .ok_or_else(|| "伺服器沒有提供憑證。".to_owned())?;
    Ok(sha256_hex(&seen))
}
