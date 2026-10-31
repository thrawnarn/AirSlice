//! TLS for talking to the printer.
//!
//! Bambu printers present a certificate signed by Bambu's own CA with the
//! serial number as common name. They are reached by LAN IP, so normal
//! verification cannot succeed; like Bambu Studio in LAN mode and
//! ha-bambulab, we accept the printer's certificate. Traffic never leaves
//! the LAN and the access code still authenticates us to the printer.
//!
//! The printer's certificate is X.509 v1, which webpki refuses to parse
//! (`UnsupportedCertVersion`), so rustls' `verify_tls12_signature` cannot be
//! used. We read the public key out of the certificate ourselves and check
//! the handshake signature with the provider's algorithms.

use rustls::{
    CertificateError, ClientConfig, DigitallySignedStruct, Error, PeerMisbehaved, SignatureScheme,
    client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier},
    crypto::{CryptoProvider, WebPkiSupportedAlgorithms},
    pki_types::{CertificateDer, ServerName, UnixTime},
};
use std::sync::Arc;

#[derive(Debug)]
struct AcceptPrinterCert(Arc<CryptoProvider>);

impl ServerCertVerifier for AcceptPrinterCert {
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

    // Signatures are still checked, so the handshake is a real TLS handshake
    // with the certificate the printer presented.
    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        verify_signature(message, cert, dss.scheme, dss.signature(), &self.0.signature_verification_algorithms)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        verify_signature(message, cert, dss.scheme, dss.signature(), &self.0.signature_verification_algorithms)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.0.signature_verification_algorithms.supported_schemes()
    }
}

/// Checks `signature` over `message` with the key in `cert`, trying every
/// algorithm the provider has for `scheme` whose key type matches the
/// certificate's (as webpki does).
fn verify_signature(
    message: &[u8],
    cert: &CertificateDer<'_>,
    scheme: SignatureScheme,
    signature: &[u8],
    algs: &WebPkiSupportedAlgorithms,
) -> Result<HandshakeSignatureValid, Error> {
    let (key_alg, key) = public_key(cert).ok_or(Error::InvalidCertificate(CertificateError::BadEncoding))?;
    let candidates = algs
        .mapping
        .iter()
        .find(|(s, _)| *s == scheme)
        .map(|(_, a)| *a)
        .ok_or(Error::PeerMisbehaved(PeerMisbehaved::SignedHandshakeWithUnadvertisedSigScheme))?;
    candidates
        .iter()
        .filter(|a| a.public_key_alg_id().as_ref() == key_alg)
        .any(|a| a.verify_signature(key, message, signature).is_ok())
        .then(HandshakeSignatureValid::assertion)
        .ok_or(Error::InvalidCertificate(CertificateError::BadSignature))
}

/// The SubjectPublicKeyInfo of a DER certificate of any X.509 version, as
/// (algorithm identifier contents, public key bits).
fn public_key(cert: &[u8]) -> Option<(&[u8], &[u8])> {
    let (cert, _) = der(cert, 0x30)?;
    let (mut tbs, _) = der(cert, 0x30)?;
    // [0] version is absent in v1 certificates.
    if tbs.first() == Some(&0xa0) {
        tbs = der(tbs, 0xa0)?.1;
    }
    // serial, signature algorithm, issuer, validity, subject
    for tag in [0x02, 0x30, 0x30, 0x30, 0x30] {
        tbs = der(tbs, tag)?.1;
    }
    let (spki, _) = der(tbs, 0x30)?;
    let (alg, rest) = der(spki, 0x30)?;
    let (bits, _) = der(rest, 0x03)?;
    // The first byte of a BIT STRING counts unused bits; keys have none.
    match bits.split_first()? {
        (0, key) => Some((alg, key)),
        _ => None,
    }
}

/// Splits one DER element with the given tag off `input`: (contents, rest).
fn der(input: &[u8], tag: u8) -> Option<(&[u8], &[u8])> {
    let (&t, rest) = input.split_first()?;
    if t != tag {
        return None;
    }
    let (&first, mut rest) = rest.split_first()?;
    let len = if first < 0x80 {
        first as usize
    } else {
        let n = (first & 0x7f) as usize;
        if n == 0 || n > 4 || rest.len() < n {
            return None;
        }
        let len = rest[..n].iter().fold(0usize, |acc, &b| acc << 8 | b as usize);
        rest = &rest[n..];
        len
    };
    (rest.len() >= len).then(|| rest.split_at(len))
}

/// A fresh client config. Each config carries its own in-memory session
/// cache, which FTPS relies on: the data connection must resume the TLS
/// session of the control connection.
pub fn printer_client_config() -> ClientConfig {
    let provider = CryptoProvider::get_default()
        .cloned()
        .unwrap_or_else(|| Arc::new(rustls::crypto::aws_lc_rs::default_provider()));
    let mut config = ClientConfig::builder_with_provider(provider.clone())
        .with_safe_default_protocol_versions()
        .expect("default TLS versions")
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(AcceptPrinterCert(provider)))
        .with_no_client_auth();
    // vsftpd ties data-channel TLS to the control session (session-id
    // resumption); tickets would not be accepted as "the same session".
    config.resumption = rustls::client::Resumption::in_memory_sessions(32)
        .tls12_resumption(rustls::client::Tls12Resumption::SessionIdOrTickets);
    config
}

pub fn install_default_provider() {
    let _ = rustls::crypto::aws_lc_rs::default_provider().install_default();
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustls::pki_types::{PrivateKeyDer, pem::PemObject};

    // X.509 v1, like the printer's; the fake printer serves the same one.
    const CERT: &str = include_str!("../../tools/fake-printer/fixtures/printer-cert.pem");
    const KEY: &str = include_str!("../../tools/fake-printer/fixtures/printer-key.pem");

    fn sign(scheme: SignatureScheme, message: &[u8]) -> Vec<u8> {
        let provider = rustls::crypto::aws_lc_rs::default_provider();
        let key = provider
            .key_provider
            .load_private_key(PrivateKeyDer::from_pem_slice(KEY.as_bytes()).unwrap())
            .unwrap();
        key.choose_scheme(&[scheme]).unwrap().sign(message).unwrap()
    }

    #[test]
    fn verifies_signatures_from_v1_certificates() {
        let cert = CertificateDer::from_pem_slice(CERT.as_bytes()).unwrap();
        let algs = rustls::crypto::aws_lc_rs::default_provider().signature_verification_algorithms;
        for scheme in [SignatureScheme::RSA_PSS_SHA256, SignatureScheme::RSA_PKCS1_SHA256] {
            let sig = sign(scheme, b"handshake");
            assert!(verify_signature(b"handshake", &cert, scheme, &sig, &algs).is_ok(), "{scheme:?}");
            assert_eq!(
                verify_signature(b"tampered", &cert, scheme, &sig, &algs).unwrap_err(),
                Error::InvalidCertificate(CertificateError::BadSignature)
            );
        }
    }

    #[test]
    fn rejects_keys_that_do_not_match_the_scheme() {
        let cert = CertificateDer::from_pem_slice(CERT.as_bytes()).unwrap();
        let algs = rustls::crypto::aws_lc_rs::default_provider().signature_verification_algorithms;
        assert_eq!(
            verify_signature(b"m", &cert, SignatureScheme::ECDSA_NISTP256_SHA256, &[0; 64], &algs).unwrap_err(),
            Error::InvalidCertificate(CertificateError::BadSignature)
        );
        assert!(public_key(&cert[..cert.len() - 1]).is_none());
    }
}
