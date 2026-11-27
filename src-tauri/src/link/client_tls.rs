// src-tauri/src/link/client_tls.rs
//
// 클라이언트 쪽 TLS 검증. 호스트 LAN 주소는 호스트 앱이 만든 CA가 서명한 인증서를 내므로(webdash/lan_tls.rs)
// 페어링 때 그 CA를 받아 고정하고, 이후로는 그 CA 하나만 믿는다. 페어링 때는 아직 믿을 CA가 없으므로
// 서버가 보낸 사슬의 마지막 인증서(CA)로 서버 인증서를 검증해 보고 그 CA를 기록만 한다 — 진짜인지는
// 페어링 증명(protocol.rs)이 그 CA 지문까지 묶어 가린다.
use std::sync::{Arc, Mutex};

use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::client::WebPkiServerVerifier;
use rustls::crypto::CryptoProvider;
use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{DigitallySignedStruct, RootCertStore, SignatureScheme};
use sha2::{Digest, Sha256};

fn provider() -> Arc<CryptoProvider> {
    Arc::new(rustls::crypto::ring::default_provider())
}

/// DER 인증서의 SHA-256 지문(`AB:CD:…`, webdash/lan_tls.rs와 같은 모양).
pub fn fingerprint_der(der: &[u8]) -> String {
    Sha256::digest(der).iter().map(|b| format!("{b:02X}")).collect::<Vec<_>>().join(":")
}

pub fn der_from_pem(pem: &str) -> Result<CertificateDer<'static>, String> {
    CertificateDer::from_pem_slice(pem.as_bytes()).map_err(|e| format!("ca: {e}"))
}

pub fn pem_from_der(der: &[u8]) -> String {
    use base64::Engine;
    let b64 = base64::engine::general_purpose::STANDARD.encode(der);
    let mut out = String::from("-----BEGIN CERTIFICATE-----\n");
    for chunk in b64.as_bytes().chunks(64) {
        out.push_str(std::str::from_utf8(chunk).unwrap_or_default());
        out.push('\n');
    }
    out.push_str("-----END CERTIFICATE-----\n");
    out
}

fn ca_verifier(ca: CertificateDer<'static>) -> Result<Arc<WebPkiServerVerifier>, rustls::Error> {
    let mut roots = RootCertStore::empty();
    roots.add(ca)?;
    WebPkiServerVerifier::builder_with_provider(Arc::new(roots), provider())
        .build()
        .map_err(|e| rustls::Error::General(e.to_string()))
}

/// 페어링 때 쓰는 검증기: 사슬의 마지막 인증서로 서버 인증서를 검증하고 그 인증서를 기록한다.
#[derive(Debug)]
pub struct CaptureCa {
    provider: Arc<CryptoProvider>,
    seen: Mutex<Option<CertificateDer<'static>>>,
}

impl CaptureCa {
    pub fn new() -> Arc<Self> {
        Arc::new(Self { provider: provider(), seen: Mutex::new(None) })
    }
    pub fn seen(&self) -> Option<CertificateDer<'static>> {
        self.seen.lock().unwrap().clone()
    }
}

impl ServerCertVerifier for CaptureCa {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        server_name: &ServerName<'_>,
        ocsp_response: &[u8],
        now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        let ca = intermediates.last().unwrap_or(end_entity).clone().into_owned();
        ca_verifier(ca.clone())?.verify_server_cert(end_entity, intermediates, server_name, ocsp_response, now)?;
        *self.seen.lock().unwrap() = Some(ca);
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(message, cert, dss, &self.provider.signature_verification_algorithms)
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(message, cert, dss, &self.provider.signature_verification_algorithms)
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.provider.signature_verification_algorithms.supported_schemes()
    }
}

fn builder() -> rustls::ConfigBuilder<rustls::ClientConfig, rustls::WantsVerifier> {
    rustls::ClientConfig::builder_with_provider(provider())
        .with_safe_default_protocol_versions()
        .expect("ring provider supports the default protocol versions")
}

fn finish(mut config: rustls::ClientConfig) -> rustls::ClientConfig {
    config.alpn_protocols = vec![b"http/1.1".to_vec()];
    config
}

/// 페어링용 설정과 그 검증기(나중에 본 CA를 꺼낸다).
pub fn capture_config() -> (rustls::ClientConfig, Arc<CaptureCa>) {
    let capture = CaptureCa::new();
    let config = builder().dangerous().with_custom_certificate_verifier(capture.clone()).with_no_client_auth();
    (finish(config), capture)
}

/// 고정한 CA 하나만 믿는 설정.
pub fn pinned_config(ca_pem: &str) -> Result<rustls::ClientConfig, String> {
    let verifier = ca_verifier(der_from_pem(ca_pem)?).map_err(|e| e.to_string())?;
    Ok(finish(builder().with_webpki_verifier(verifier).with_no_client_auth()))
}

/// 공인 인증서(webpki 루트).
pub fn public_config() -> rustls::ClientConfig {
    let roots = RootCertStore { roots: webpki_roots::TLS_SERVER_ROOTS.to_vec() };
    finish(builder().with_root_certificates(roots).with_no_client_auth())
}
