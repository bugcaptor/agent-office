// src-tauri/src/webdash/lan_tls.rs
//
// LAN 주소용 자체 서명 HTTPS. 공인 인증서를 받을 이름이 없으므로 앱이 이 컴퓨터만의 CA를 한 번 만들어
// 두고(10년), 그 CA로 지금 LAN IP들을 담은 서버 인증서를 발급한다(397일 — Apple·Chrome 상한 아래).
// 기기는 처음 열 때 경고를 한 번 수락하거나, `/ca.crt`로 받은 CA를 설치해 경고 없이 쓴다. 설정 화면에
// CA 지문을 보여 바꿔치기를 가릴 수 있게 한다.
//
// IP가 바뀌면 서버 인증서만 다시 발급한다(CA·지문은 그대로). 파일은 데이터 폴더 `web-dash-tls/`에 둔다.
use std::net::{IpAddr, Ipv4Addr};
use std::path::Path;
use std::sync::Arc;

use rcgen::{
    BasicConstraints, CertificateParams, DistinguishedName, DnType, ExtendedKeyUsagePurpose, IsCa, Issuer, KeyPair,
    KeyUsagePurpose, SanType,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use time::{Duration, OffsetDateTime};

use super::tls;

const CA_CERT: &str = "lan-ca.pem";
const CA_KEY: &str = "lan-ca-key.pem";
const LEAF: &str = "lan-leaf.json";
const CA_DAYS: i64 = 3650;
const LEAF_DAYS: i64 = 397;
/// 만료가 이만큼 남으면 서버 인증서를 새로 발급한다.
const RENEW_BEFORE_DAYS: i64 = 30;

/// 준비된 LAN 인증서.
pub struct LanCert {
    pub key: Arc<rustls::sign::CertifiedKey>,
    /// CA 인증서(PEM). `/ca.crt`로 내보낸다.
    pub ca_pem: String,
    /// CA 인증서 SHA-256 지문(`AB:CD:…`).
    pub fingerprint: String,
}

#[derive(Serialize, Deserialize)]
struct StoredLeaf {
    ips: Vec<Ipv4Addr>,
    /// 만료(유닉스 초).
    not_after: i64,
    cert_pem: String,
    key_pem: String,
}

/// CA의 이름·용도. 서명자를 다시 만들 때도 같은 값을 쓴다(발급자 이름이 CA 인증서와 맞아야 한다).
fn ca_params() -> CertificateParams {
    let mut params = CertificateParams::default();
    let mut dn = DistinguishedName::new();
    dn.push(DnType::OrganizationName, "Agent Office");
    dn.push(DnType::CommonName, "Agent Office LAN CA");
    params.distinguished_name = dn;
    params.is_ca = IsCa::Ca(BasicConstraints::Constrained(0));
    params.key_usages = vec![KeyUsagePurpose::KeyCertSign, KeyUsagePurpose::CrlSign, KeyUsagePurpose::DigitalSignature];
    params
}

fn write_private(path: &Path, text: &str) -> Result<(), String> {
    std::fs::write(path, text).map_err(|e| e.to_string())?;
    crate::httpapi::set_owner_only(path);
    Ok(())
}

/// CA를 읽거나(없으면 만들어) 서명자와 PEM을 돌려준다.
fn load_or_create_ca(dir: &Path, now: OffsetDateTime) -> Result<(KeyPair, String), String> {
    let (cert_path, key_path) = (dir.join(CA_CERT), dir.join(CA_KEY));
    if let (Ok(cert), Ok(key)) = (std::fs::read_to_string(&cert_path), std::fs::read_to_string(&key_path)) {
        if let Ok(key) = KeyPair::from_pem(&key) {
            return Ok((key, cert));
        }
    }
    let key = KeyPair::generate().map_err(|e| e.to_string())?;
    let mut params = ca_params();
    params.not_before = now - Duration::days(1);
    params.not_after = now + Duration::days(CA_DAYS);
    let cert = params.self_signed(&key).map_err(|e| e.to_string())?;
    std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    write_private(&key_path, &key.serialize_pem())?;
    std::fs::write(&cert_path, cert.pem()).map_err(|e| e.to_string())?;
    // CA가 바뀌면 옛 CA가 서명한 서버 인증서는 버린다.
    let _ = std::fs::remove_file(dir.join(LEAF));
    Ok((key, cert.pem()))
}

fn issue_leaf(ca_key: &KeyPair, ips: &[Ipv4Addr], now: OffsetDateTime) -> Result<StoredLeaf, String> {
    let key = KeyPair::generate().map_err(|e| e.to_string())?;
    let mut params = CertificateParams::default();
    let mut dn = DistinguishedName::new();
    dn.push(DnType::OrganizationName, "Agent Office");
    dn.push(DnType::CommonName, "Agent Office web dashboard");
    params.distinguished_name = dn;
    params.subject_alt_names = ips.iter().map(|ip| SanType::IpAddress(IpAddr::V4(*ip))).collect();
    params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];
    params.key_usages = vec![KeyUsagePurpose::DigitalSignature];
    params.use_authority_key_identifier_extension = true;
    params.not_before = now - Duration::days(1);
    params.not_after = now + Duration::days(LEAF_DAYS);
    let issuer = Issuer::new(ca_params(), ca_key);
    let cert = params.signed_by(&key, &issuer).map_err(|e| e.to_string())?;
    Ok(StoredLeaf {
        ips: ips.to_vec(),
        not_after: params.not_after.unix_timestamp(),
        cert_pem: cert.pem(),
        key_pem: key.serialize_pem(),
    })
}

/// PEM 인증서의 SHA-256 지문.
pub fn fingerprint(cert_pem: &str) -> Option<String> {
    use rustls::pki_types::pem::PemObject;
    let der = rustls::pki_types::CertificateDer::from_pem_slice(cert_pem.as_bytes()).ok()?;
    let digest = Sha256::digest(der.as_ref());
    Some(digest.iter().map(|b| format!("{b:02X}")).collect::<Vec<_>>().join(":"))
}

/// `ips`를 담은 서버 인증서를 준비한다(블로킹 — 파일을 읽고 쓴다). 있던 것이 같은 IP를 담고 아직
/// 넉넉하면 그대로 쓴다.
pub fn prepare(data_dir: &Path, ips: &[Ipv4Addr]) -> Result<LanCert, String> {
    prepare_at(data_dir, ips, OffsetDateTime::now_utc())
}

fn prepare_at(data_dir: &Path, ips: &[Ipv4Addr], now: OffsetDateTime) -> Result<LanCert, String> {
    let dir = data_dir.join(tls::DIR);
    let (ca_key, ca_pem) = load_or_create_ca(&dir, now)?;
    let mut wanted = ips.to_vec();
    wanted.sort();
    wanted.dedup();
    let stored = std::fs::read(dir.join(LEAF)).ok().and_then(|b| serde_json::from_slice::<StoredLeaf>(&b).ok());
    let fresh = stored.filter(|l| {
        let mut have = l.ips.clone();
        have.sort();
        have == wanted && l.not_after - now.unix_timestamp() > RENEW_BEFORE_DAYS * 86_400
    });
    let leaf = match fresh {
        Some(leaf) => leaf,
        None => {
            let leaf = issue_leaf(&ca_key, &wanted, now)?;
            let json = serde_json::to_string(&leaf).map_err(|e| e.to_string())?;
            write_private(&dir.join(LEAF), &json)?;
            leaf
        }
    };
    // 브라우저가 사슬을 이을 수 있게 CA도 함께 보낸다.
    let chain = format!("{}{}", leaf.cert_pem, ca_pem);
    Ok(LanCert {
        key: tls::certified_key(chain.as_bytes(), leaf.key_pem.as_bytes())?,
        fingerprint: fingerprint(&ca_pem).ok_or("ca: bad pem")?,
        ca_pem,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp() -> std::path::PathBuf {
        std::env::temp_dir().join(format!("lan-tls-{}", uuid::Uuid::new_v4()))
    }

    #[test]
    fn keeps_the_ca_and_reissues_the_leaf_only_when_ips_change_or_expire() {
        let dir = temp();
        let ip1: Ipv4Addr = "192.168.1.10".parse().unwrap();
        let ip2: Ipv4Addr = "10.0.0.5".parse().unwrap();
        let now = OffsetDateTime::now_utc();
        let first = prepare_at(&dir, &[ip1], now).unwrap();
        let leaf = |d: &Path| std::fs::read_to_string(d.join(tls::DIR).join(LEAF)).unwrap();
        let leaf1 = leaf(&dir);
        let again = prepare_at(&dir, &[ip1], now).unwrap();
        assert_eq!(first.fingerprint, again.fingerprint);
        assert_eq!(leaf(&dir), leaf1, "같은 IP면 서버 인증서를 그대로 쓴다");
        let moved = prepare_at(&dir, &[ip1, ip2], now).unwrap();
        assert_eq!(moved.fingerprint, first.fingerprint, "IP가 바뀌어도 CA는 그대로");
        let leaf2 = leaf(&dir);
        assert_ne!(leaf2, leaf1);
        prepare_at(&dir, &[ip2, ip1], now + Duration::days(LEAF_DAYS - RENEW_BEFORE_DAYS + 1)).unwrap();
        assert_ne!(leaf(&dir), leaf2, "만료가 가까우면 새로 발급한다");
        assert_eq!(first.fingerprint.split(':').count(), 32);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn a_client_trusting_the_ca_connects_by_ip() {
        let dir = temp();
        let lan = prepare(&dir, &[Ipv4Addr::LOCALHOST]).unwrap();
        let listener = tokio::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let addr = listener.local_addr().unwrap();
        let app = axum::Router::new().route("/ok", axum::routing::get(|| async { "ok" }));
        let stop = tls::serve_listener(listener, app, "127.0.0.1".into(), Arc::new(tls::SwapCert::new(lan.key)));
        let client = reqwest::Client::builder()
            .add_root_certificate(reqwest::Certificate::from_pem(lan.ca_pem.as_bytes()).unwrap())
            .build()
            .unwrap();
        let body = client.get(format!("https://127.0.0.1:{}/ok", addr.port())).send().await.unwrap().text().await.unwrap();
        assert_eq!(body, "ok");
        let _ = stop.send(());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
