// src-tauri/src/webdash/tls.rs
//
// tailnet 주소는 HTTPS로 연다. 인증서는 Tailscale이 MagicDNS 이름(`<기기>.<tailnet>.ts.net`)으로
// 발급해 주는 Let's Encrypt 인증서를 `tailscale cert`로 받는다(tailnet 관리 화면에서 HTTPS
// 인증서를 켜 둬야 한다). 못 받으면 예전처럼 HTTP로 연다.
//
// 한 포트에서 TLS와 평문을 함께 받는다. 첫 바이트가 TLS 핸드셰이크(0x16)면 TLS로, 아니면
// 같은 경로의 `https://<이름>:<포트>`로 넘긴다 — 예전 `http://100.x.y.z` 주소로 열어도 따라온다
// (주소의 `#t=` 토큰은 브라우저가 넘김 뒤에도 그대로 둔다).
//
// 인증서는 바꿔 끼울 수 있게 들고 있어(`SwapCert`) 갱신할 때 리스너를 다시 열지 않는다.
use std::net::{Ipv4Addr, SocketAddr};
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};
use std::time::Duration;

use axum::extract::{ConnectInfo, Request};
use axum::http::{header, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Router;
use hyper_util::rt::{TokioExecutor, TokioIo};
use hyper_util::server::conn::auto::Builder;
use hyper_util::service::TowerToHyperService;
use rustls::pki_types::pem::PemObject;
use rustls::pki_types::{CertificateDer, PrivateKeyDer};
use rustls::server::{ClientHello, ResolvesServerCert};
use rustls::sign::CertifiedKey;
use tokio::sync::{oneshot, watch};

/// 인증서를 받아 둘 폴더(데이터 폴더 아래).
pub const DIR: &str = "web-dash-tls";
/// `tailscale` 호출 한 번의 시간 상한(인증서를 새로 발급받을 때는 몇 초 걸린다).
const CLI_TIMEOUT: Duration = Duration::from_secs(60);
/// 첫 바이트를 기다리는 시간. 연결만 열고 말이 없는 쪽을 오래 붙잡지 않는다.
const FIRST_BYTE_TIMEOUT: Duration = Duration::from_secs(15);

/// 받아 온 인증서 한 벌.
pub struct Fetched {
    pub domain: String,
    pub key: Arc<CertifiedKey>,
}

/// 바꿔 끼울 수 있는 서버 인증서.
#[derive(Debug)]
pub struct SwapCert(RwLock<Arc<CertifiedKey>>);

impl SwapCert {
    pub fn new(key: Arc<CertifiedKey>) -> Self {
        Self(RwLock::new(key))
    }
    pub fn set(&self, key: Arc<CertifiedKey>) {
        *self.0.write().unwrap() = key;
    }
}

impl ResolvesServerCert for SwapCert {
    fn resolve(&self, _hello: ClientHello<'_>) -> Option<Arc<CertifiedKey>> {
        Some(self.0.read().unwrap().clone())
    }
}

fn provider() -> Arc<rustls::crypto::CryptoProvider> {
    Arc::new(rustls::crypto::ring::default_provider())
}

/// PEM 인증서 사슬과 개인 키로 rustls가 쓰는 인증서를 만든다.
pub fn certified_key(cert_pem: &[u8], key_pem: &[u8]) -> Result<Arc<CertifiedKey>, String> {
    let chain = CertificateDer::pem_slice_iter(cert_pem)
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| format!("cert: {e}"))?;
    if chain.is_empty() {
        return Err("cert: empty".into());
    }
    let key = PrivateKeyDer::from_pem_slice(key_pem).map_err(|e| format!("key: {e}"))?;
    let key = CertifiedKey::from_der(chain, key, &provider()).map_err(|e| format!("key: {e}"))?;
    Ok(Arc::new(key))
}

pub fn server_config(cert: Arc<SwapCert>) -> Arc<rustls::ServerConfig> {
    let mut config = rustls::ServerConfig::builder_with_provider(provider())
        .with_safe_default_protocol_versions()
        .expect("ring provider supports the default protocol versions")
        .with_no_client_auth()
        .with_cert_resolver(cert);
    config.alpn_protocols = vec![b"h2".to_vec(), b"http/1.1".to_vec()];
    Arc::new(config)
}

// ── Tailscale CLI ───────────────────────────────────────────────────

/// `tailscale` CLI 경로. macOS 앱판은 PATH에 없을 수 있어 앱 안 실행 파일도 본다.
fn tailscale_cli() -> Option<PathBuf> {
    let mut candidates: Vec<PathBuf> = Vec::new();
    #[cfg(target_os = "macos")]
    candidates.extend([
        "/Applications/Tailscale.app/Contents/MacOS/Tailscale".into(),
        "/opt/homebrew/bin/tailscale".into(),
        "/usr/local/bin/tailscale".into(),
    ]);
    #[cfg(windows)]
    {
        for var in ["ProgramFiles", "ProgramFiles(x86)"] {
            if let Some(dir) = std::env::var_os(var) {
                candidates.push(PathBuf::from(dir).join("Tailscale").join("tailscale.exe"));
            }
        }
    }
    #[cfg(target_os = "linux")]
    candidates.extend(["/usr/bin/tailscale".into(), "/usr/local/bin/tailscale".into()]);
    if let Some(found) = candidates.into_iter().find(|p| p.is_file()) {
        return Some(found);
    }
    // 그 밖은 PATH에 맡긴다.
    let name = if cfg!(windows) { "tailscale.exe" } else { "tailscale" };
    std::env::var_os("PATH")
        .into_iter()
        .flat_map(|p| std::env::split_paths(&p).collect::<Vec<_>>())
        .map(|dir| dir.join(name))
        .find(|p| p.is_file())
}

fn run_cli(cli: &Path, args: &[&str]) -> Result<Vec<u8>, String> {
    let program = cli.to_string_lossy();
    let run = crate::proc_runner::run(crate::proc_runner::ProcSpec {
        program: &program,
        args,
        cwd: None,
        envs: &[],
        timeout: CLI_TIMEOUT,
        max_stdout_bytes: Some(8 * 1024 * 1024),
        cancel: None,
    });
    match run.outcome {
        crate::proc_runner::ProcOutcome::Exited { success: true } => Ok(run.stdout),
        crate::proc_runner::ProcOutcome::Exited { success: false } => Err(format!("tailscale {} failed", args[0])),
        crate::proc_runner::ProcOutcome::SpawnFailed => Err("tailscale-missing".into()),
        _ => Err(format!("tailscale {} timed out", args[0])),
    }
}

/// `tailscale status --json`에서 인증서를 받을 수 있는 이름. 관리 화면에서 HTTPS를 켜야 생긴다.
pub fn cert_domain(status_json: &[u8]) -> Option<String> {
    let v: serde_json::Value = serde_json::from_slice(status_json).ok()?;
    v.get("CertDomains")?
        .as_array()?
        .iter()
        .filter_map(|d| d.as_str())
        .map(|d| d.trim_end_matches('.').to_string())
        .find(|d| !d.is_empty())
}

/// 이 기기의 tailnet 인증서를 받아 온다(블로킹 — 프로세스를 두 번 띄운다).
/// Tailscale이 갱신 시점을 스스로 판단하므로 다시 불러도 싸다.
pub fn fetch(data_dir: &Path) -> Result<Fetched, String> {
    let cli = tailscale_cli().ok_or("tailscale-missing")?;
    let domain = cert_domain(&run_cli(&cli, &["status", "--json"])?).ok_or("https-not-enabled")?;
    let dir = data_dir.join(DIR);
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let cert_path = dir.join("cert.pem");
    let key_path = dir.join("key.pem");
    let (cert_arg, key_arg) = (cert_path.to_string_lossy(), key_path.to_string_lossy());
    run_cli(&cli, &["cert", "--cert-file", &cert_arg, "--key-file", &key_arg, &domain])?;
    crate::httpapi::set_owner_only(&key_path);
    let cert = std::fs::read(&cert_path).map_err(|e| e.to_string())?;
    let key = std::fs::read(&key_path).map_err(|e| e.to_string())?;
    Ok(Fetched { domain, key: certified_key(&cert, &key)? })
}

// ── 리스너 ──────────────────────────────────────────────────────────

/// `ip:port`에서 TLS(와 평문 → HTTPS 넘김)를 받는다. 돌려준 송신기로 멈춘다.
pub async fn serve(
    ip: Ipv4Addr,
    port: u16,
    app: Router,
    domain: String,
    cert: Arc<SwapCert>,
) -> std::io::Result<oneshot::Sender<()>> {
    let listener = tokio::net::TcpListener::bind((ip, port)).await?;
    Ok(serve_listener(listener, app, domain, cert))
}

pub(crate) fn serve_listener(
    listener: tokio::net::TcpListener,
    app: Router,
    domain: String,
    cert: Arc<SwapCert>,
) -> oneshot::Sender<()> {
    let port = listener.local_addr().map(|a| a.port()).unwrap_or(0);
    let acceptor = tokio_rustls::TlsAcceptor::from(server_config(cert));
    let redirect = Router::new().fallback(move |req: Request| async move { redirect(&domain, port, &req) });
    let (tx, mut rx) = oneshot::channel::<()>();
    // 멈추면 열려 있던 연결도 끊는다(설정을 끄면 바로 닫혀야 한다).
    let (stop_tx, stop_rx) = watch::channel(false);
    tokio::spawn(async move {
        loop {
            let (stream, peer) = tokio::select! {
                _ = &mut rx => break,
                accepted = listener.accept() => match accepted {
                    Ok(a) => a,
                    Err(_) => {
                        tokio::time::sleep(Duration::from_millis(100)).await;
                        continue;
                    }
                },
            };
            let (app, redirect, acceptor, mut stop) = (app.clone(), redirect.clone(), acceptor.clone(), stop_rx.clone());
            tokio::spawn(async move {
                let conn = handle(stream, peer, app, redirect, acceptor);
                tokio::select! {
                    _ = conn => {}
                    _ = stop.changed() => {}
                }
            });
        }
        let _ = stop_tx.send(true);
    });
    tx
}

async fn handle(
    stream: tokio::net::TcpStream,
    peer: SocketAddr,
    app: Router,
    redirect: Router,
    acceptor: tokio_rustls::TlsAcceptor,
) {
    let mut first = [0u8; 1];
    match tokio::time::timeout(FIRST_BYTE_TIMEOUT, stream.peek(&mut first)).await {
        Ok(Ok(1)) => {}
        _ => return,
    }
    let builder = Builder::new(TokioExecutor::new());
    if first[0] == 0x16 {
        let Ok(Ok(tls)) = tokio::time::timeout(FIRST_BYTE_TIMEOUT, acceptor.accept(stream)).await else {
            return;
        };
        let svc = TowerToHyperService::new(app.layer(axum::Extension(ConnectInfo(peer))));
        let _ = builder.serve_connection_with_upgrades(TokioIo::new(tls), svc).await;
    } else {
        let svc = TowerToHyperService::new(redirect);
        let _ = builder.serve_connection(TokioIo::new(stream), svc).await;
    }
}

/// 평문으로 들어온 요청을 같은 경로의 HTTPS 주소로 넘긴다.
fn redirect(domain: &str, port: u16, req: &Request) -> Response {
    let path = req.uri().path_and_query().map(|p| p.as_str()).unwrap_or("/");
    let location = format!("https://{domain}:{port}{path}");
    let mut res = StatusCode::PERMANENT_REDIRECT.into_response();
    if let Ok(v) = HeaderValue::from_str(&location) {
        res.headers_mut().insert(header::LOCATION, v);
    }
    res
}

#[cfg(test)]
mod tests {
    use super::*;

    // 시험용 CA와 그 CA가 서명한 서버 인증서(SAN `dash.test`, EC P-256, 100년). 이 시험에서만 쓴다.
    const TEST_CA: &str = include_str!("testdata/dash-test-ca.pem");
    const TEST_CERT: &str = include_str!("testdata/dash-test-cert.pem");
    const TEST_KEY: &str = include_str!("testdata/dash-test-key.pem");

    #[test]
    fn reads_cert_domain_from_status() {
        let status = br#"{"CertDomains":["zm.tail1.ts.net."],"Self":{"DNSName":"zm.tail1.ts.net."}}"#;
        assert_eq!(cert_domain(status).as_deref(), Some("zm.tail1.ts.net"));
        assert_eq!(cert_domain(br#"{"CertDomains":null}"#), None);
        assert_eq!(cert_domain(br#"{"CertDomains":[]}"#), None);
        assert_eq!(cert_domain(b"not json"), None);
    }

    #[test]
    fn builds_certified_key_from_pem() {
        assert!(certified_key(TEST_CERT.as_bytes(), TEST_KEY.as_bytes()).is_ok());
        assert!(certified_key(b"", TEST_KEY.as_bytes()).is_err());
        assert!(certified_key(TEST_CERT.as_bytes(), b"").is_err());
    }

    #[tokio::test]
    async fn serves_https_and_redirects_plain_http() {
        let key = certified_key(TEST_CERT.as_bytes(), TEST_KEY.as_bytes()).unwrap();
        let listener = tokio::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
        let addr = listener.local_addr().unwrap();
        let app = Router::new().route(
            "/hello",
            axum::routing::get(|ConnectInfo(peer): ConnectInfo<SocketAddr>| async move { format!("hi {}", peer.ip()) }),
        );
        let stop = serve_listener(listener, app, "dash.test".into(), Arc::new(SwapCert::new(key)));

        let client = reqwest::Client::builder()
            .add_root_certificate(reqwest::Certificate::from_pem(TEST_CA.as_bytes()).unwrap())
            .resolve("dash.test", addr)
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .unwrap();
        let body = client
            .get(format!("https://dash.test:{}/hello", addr.port()))
            .send()
            .await
            .unwrap()
            .text()
            .await
            .unwrap();
        assert_eq!(body, "hi 127.0.0.1");

        let plain = client.get(format!("http://127.0.0.1:{}/hello?x=1", addr.port())).send().await.unwrap();
        assert_eq!(plain.status(), StatusCode::PERMANENT_REDIRECT);
        assert_eq!(
            plain.headers().get(header::LOCATION).unwrap(),
            &format!("https://dash.test:{}/hello?x=1", addr.port())
        );
        let _ = stop.send(());
    }
}
