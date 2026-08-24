// src-tauri/src/ipc/commands/tailscale.rs
//
// tailscale serve 대행(docs/web-remote-design.md §M3, 결정 H·I). 웹 원격은
// tailnet 평문 http로 붙는 것이 기본이고, 여기에 tailscaled의 serve 프록시로
// HTTPS를 씌우는 경로를 앱이 **대행**한다.
//
// 원칙 세 가지가 설계에 못 박혀 있다:
//
// 1. **상태 정본은 tailscaled다.** `serve --bg`는 기계 전역·재부팅 영속이라
//    앱 설정과 수명이 어긋난다. 그래서 앱 설정에 저장하지 않고 매번 CLI에
//    물어본다. 앱을 꺼도 매핑은 남는다(UI가 그 사실을 명시한다).
// 2. **`serve reset`·`funnel`은 쓰지 않는다.** reset은 사용자의 다른 매핑까지
//    지우고(이 기계에도 8443→4173 프록시가 살아 있다), funnel은 공개 인터넷
//    노출이다. 등록/해제는 **포트 지정형**(`--https=<포트> …` / `… off`)만 쓴다.
// 3. **CLI 실행은 이 커맨드 핸들러에서만.** 서버 기동 경로는 tailscaled를
//    건드리지 않는다.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use tauri::State;

use crate::state::AppState;

/// serve로 열 전용 HTTPS 포트. 443 루트는 이미 남의 서비스가 점유할 수 있어
/// (실측: 이 기계의 443은 다른 매핑) 충돌 없는 전용 포트를 쓴다. 다른 기계에서
/// 이 포트가 겹치면 여기를 바꾸면 된다 — 인증서는 포트와 무관하다.
pub const WEB_REMOTE_HTTPS_PORT: u16 = 47443;

/// CLI 한 번 호출의 상한. tailscaled가 응답하지 않을 때 UI가 영영 매달리지
/// 않게 한다.
const CLI_TIMEOUT: Duration = Duration::from_secs(10);

/// 설정 UI의 "HTTPS (tailscale serve)" 블록이 한 번에 읽는 상태. 전부
/// tailscaled에서 방금 읽어 온 사실이다(앱은 아무것도 기억하지 않는다).
#[derive(Debug, Clone, Default, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TailscaleServeStatus {
    /// CLI를 찾았는가. false면 UI는 안내문만 띄운다(버튼 없음).
    pub cli_found: bool,
    pub cli_path: Option<String>,
    /// `tailscale status`가 `BackendState: "Running"`을 보고했는가.
    pub backend_running: bool,
    /// 이 노드의 MagicDNS 이름(끝 점 제거). https 주소의 호스트부다.
    pub dns_name: Option<String>,
    pub https_port: u16,
    /// 우리 웹 원격으로 가는 프록시가 그 포트에 걸려 있는가.
    pub registered: bool,
    /// 그 포트에 실제로 걸린 업스트림(있으면). 남의 매핑일 수도 있다.
    pub upstream: Option<String>,
    /// 지금 켠다면 등록할 업스트림. 서버가 안 떠 있으면 None(켜기 불가).
    pub expected_upstream: Option<String>,
    /// 그 포트를 **다른 업스트림**이 점유 중이다 — 덮어쓰지 않고 UI가 알린다.
    pub conflict: bool,
    /// 브라우저에 불러 줄 https 주소.
    pub https_url: Option<String>,
    /// 조회 중 생긴 문제 요약(사람이 읽을 한 줄). CLI 미탐지는 여기가 아니라
    /// `cli_found`로 표현한다.
    pub error: Option<String>,
}

// ── 순수 파서 ────────────────────────────────────────────────────────

/// `tailscale serve status --json`에서 한 포트의 매핑만 추린 결과.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ServeMapping {
    /// TCP 맵에 이 포트의 HTTPS 종단이 있는가.
    pub https_listener: bool,
    /// 그 포트 Web 맵의 `/` 핸들러가 프록시하는 업스트림.
    pub upstream: Option<String>,
    /// 등록된 `host:port` 키(예 `zm4mini.tailc90d0d.ts.net:47443`).
    pub host: Option<String>,
}

/// serve 설정 JSON에서 **우리 포트**의 매핑을 찾는다. 실측 출력은
///
/// ```json
/// { "TCP": { "8443": { "HTTPS": true } },
///   "Web": { "zm4mini.tailc90d0d.ts.net:8443": {
///              "Handlers": { "/": { "Proxy": "http://127.0.0.1:4173" } } } } }
/// ```
///
/// 형태다. 설정이 비어 있으면 `{}`(또는 `null`)이라 전부 옵셔널로 다룬다.
/// 다른 포트의 매핑은 **읽지도 건드리지도 않는다** — 남의 설정이다.
pub fn parse_serve_status(json: &str, https_port: u16) -> ServeMapping {
    let Ok(root) = serde_json::from_str::<serde_json::Value>(json) else {
        return ServeMapping::default();
    };
    let port_key = https_port.to_string();
    let suffix = format!(":{https_port}");

    let https_listener = root
        .get("TCP")
        .and_then(|t| t.get(&port_key))
        .and_then(|e| e.get("HTTPS"))
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    // Web 키는 `host:port`다. 호스트는 노드마다 달라서 포트부로만 고른다.
    let mut host = None;
    let mut upstream = None;
    if let Some(web) = root.get("Web").and_then(|w| w.as_object()) {
        for (key, entry) in web {
            if !key.ends_with(&suffix) {
                continue;
            }
            host = Some(key.clone());
            upstream = entry
                .get("Handlers")
                .and_then(|h| h.get("/"))
                .and_then(|h| h.get("Proxy"))
                .and_then(|v| v.as_str())
                .map(|s| s.to_string());
            break;
        }
    }

    ServeMapping {
        https_listener,
        upstream,
        host,
    }
}

/// `tailscale status --json`의 노드 정보(순수 함수).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NodeStatus {
    /// `Self.DNSName`에서 **끝 점을 지운** 이름. FQDN의 루트 점은 URL에 쓰면
    /// 어색하고(`https://host.ts.net.:47443`) 인증서 이름과도 어긋난다.
    pub dns_name: Option<String>,
    pub backend_running: bool,
}

pub fn parse_node_status(json: &str) -> NodeStatus {
    let Ok(root) = serde_json::from_str::<serde_json::Value>(json) else {
        return NodeStatus::default();
    };
    let dns_name = root
        .get("Self")
        .and_then(|s| s.get("DNSName"))
        .and_then(|v| v.as_str())
        .map(|s| s.trim_end_matches('.').to_string())
        .filter(|s| !s.is_empty());
    let backend_running = root
        .get("BackendState")
        .and_then(|v| v.as_str())
        .map(|s| s == "Running")
        .unwrap_or(false);
    NodeStatus {
        dns_name,
        backend_running,
    }
}

// ── CLI 탐색·실행 ────────────────────────────────────────────────────

/// tailscale CLI 경로. PATH를 먼저 보고, 못 찾으면 알려진 설치 위치를 훑는다 —
/// Finder에서 띄운 앱의 PATH는 `/usr/bin:/bin:/usr/sbin:/sbin`뿐이라
/// `/usr/local/bin/tailscale`이 PATH에 없는 것이 정상이기 때문이다. macOS
/// 앱스토어판은 앱 번들 안에만 CLI가 있다.
fn find_cli() -> Option<PathBuf> {
    if let Some(path) = std::env::var_os("PATH") {
        for dir in std::env::split_paths(&path) {
            let candidate = dir.join("tailscale");
            if is_executable(&candidate) {
                return Some(candidate);
            }
        }
    }
    let fallbacks = [
        "/usr/local/bin/tailscale",
        "/opt/homebrew/bin/tailscale",
        "/Applications/Tailscale.app/Contents/MacOS/Tailscale",
    ];
    fallbacks
        .iter()
        .map(PathBuf::from)
        .find(|p| is_executable(p))
}

fn is_executable(path: &Path) -> bool {
    match std::fs::metadata(path) {
        Ok(meta) if meta.is_file() => {
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                meta.permissions().mode() & 0o111 != 0
            }
            #[cfg(not(unix))]
            {
                true
            }
        }
        _ => false,
    }
}

/// CLI 한 번 실행. 실패는 stderr 마지막 줄로 요약한다(경로·토큰 같은 민감
/// 정보가 없는 짧은 사유 문자열이다).
async fn run_cli(cli: &Path, args: &[&str]) -> Result<String, String> {
    let mut command = tokio::process::Command::new(cli);
    command
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }

    let output = tokio::time::timeout(CLI_TIMEOUT, command.output())
        .await
        .map_err(|_| "timeout".to_string())?
        .map_err(|e| format!("tailscale-spawn-failed: {e}"))?;

    if output.status.success() {
        return Ok(String::from_utf8_lossy(&output.stdout).to_string());
    }
    // stderr 원문(tailscale 자신의 영어 메시지)은 번역하지 않고 상세로 붙인다.
    Err(format!(
        "tailscale-cli-error: {}",
        summarize_stderr(&String::from_utf8_lossy(&output.stderr))
    ))
}

/// stderr에서 사람이 읽을 한 줄을 뽑는다(마지막 비어 있지 않은 줄, 200자 상한).
fn summarize_stderr(stderr: &str) -> String {
    let line = stderr
        .lines()
        .rev()
        .map(|l| l.trim())
        .find(|l| !l.is_empty())
        .unwrap_or("unknown error");
    line.chars().take(200).collect()
}

/// serve가 프록시할 업스트림. 웹 원격 리스너는 tailnet IP에 **직접** 바인드하므로
/// `http://127.0.0.1:<포트>`가 아니다(M1 이탈 1 — 루프백은 그때 닫혀 있다).
/// 전 인터페이스(`0.0.0.0`) 바인드일 때만 사람에게 불러 줄 주소로 되돌아간다.
fn expected_upstream(app_state: &AppState) -> Option<String> {
    let bound = app_state.web_remote_server.current_bound()?;
    let host = if bound.ip.is_unspecified() {
        crate::webremote::local_addr_hint()?
    } else {
        bound.ip.to_string()
    };
    Some(format!("http://{host}:{}", bound.port))
}

/// 상태 한 벌을 CLI 두 번(serve status + status)으로 만든다.
async fn collect_status(cli: &Path, expected: Option<String>) -> TailscaleServeStatus {
    let mut status = TailscaleServeStatus {
        cli_found: true,
        cli_path: Some(cli.display().to_string()),
        https_port: WEB_REMOTE_HTTPS_PORT,
        expected_upstream: expected.clone(),
        ..Default::default()
    };

    match run_cli(cli, &["status", "--json", "--peers=false"]).await {
        Ok(raw) => {
            let node = parse_node_status(&raw);
            status.backend_running = node.backend_running;
            status.dns_name = node.dns_name;
        }
        Err(e) => status.error = Some(e),
    }

    match run_cli(cli, &["serve", "status", "--json"]).await {
        Ok(raw) => {
            let mapping = parse_serve_status(&raw, WEB_REMOTE_HTTPS_PORT);
            // 업스트림이 우리 것과 다르면 남의 매핑이다 — 덮어쓰지 않는다.
            // 서버가 안 떠 있어(expected=None) 비교할 수 없으면 등록 사실만 보고한다.
            match (&mapping.upstream, &expected) {
                (Some(found), Some(want)) if found == want => status.registered = true,
                (Some(_), Some(_)) => status.conflict = true,
                (Some(_), None) => status.registered = true,
                (None, _) => {}
            }
            status.upstream = mapping.upstream;
            if status.dns_name.is_none() {
                // status가 실패했더라도 등록된 키에서 호스트를 건질 수 있다.
                status.dns_name = mapping
                    .host
                    .and_then(|h| h.rsplit_once(':').map(|(host, _)| host.to_string()));
            }
        }
        Err(e) => {
            if status.error.is_none() {
                status.error = Some(e);
            }
        }
    }

    if let Some(dns) = &status.dns_name {
        status.https_url = Some(format!(
            "https://{dns}:{}/web/",
            WEB_REMOTE_HTTPS_PORT
        ));
    }
    status
}

// ── 커맨드 ───────────────────────────────────────────────────────────

#[tauri::command(rename_all = "camelCase")]
pub async fn tailscale_serve_status(
    app_state: State<'_, AppState>,
) -> Result<TailscaleServeStatus, String> {
    let expected = expected_upstream(&app_state);
    let Some(cli) = find_cli() else {
        return Ok(TailscaleServeStatus {
            cli_found: false,
            https_port: WEB_REMOTE_HTTPS_PORT,
            expected_upstream: expected,
            ..Default::default()
        });
    };
    Ok(collect_status(&cli, expected).await)
}

/// `tailscale serve --bg --https=47443 http://<tailnet IP>:<웹 원격 포트>`.
///
/// 포트 지정형이라 사용자의 다른 serve 매핑(예: 8443→4173)은 건드리지 않는다.
#[tauri::command(rename_all = "camelCase")]
pub async fn tailscale_serve_enable(app_state: State<'_, AppState>) -> Result<(), String> {
    let Some(upstream) = expected_upstream(&app_state) else {
        return Err("web-remote-down".into());
    };
    let Some(cli) = find_cli() else {
        return Err("tailscale-cli-not-found".into());
    };

    // 남의 매핑을 덮어쓰지 않는다(자동 포트 순회도 하지 않는다 — 설계 §10.2).
    let before = collect_status(&cli, Some(upstream.clone())).await;
    if before.conflict {
        let found = before.upstream.unwrap_or_default();
        return Err(format!(
            "serve-port-conflict: {WEB_REMOTE_HTTPS_PORT} -> {found}"
        ));
    }

    let https_flag = format!("--https={WEB_REMOTE_HTTPS_PORT}");
    run_cli(&cli, &["serve", "--bg", &https_flag, &upstream])
        .await
        .map(|_| ())
}

/// `tailscale serve --https=47443 off`. **`serve reset`은 쓰지 않는다** —
/// 사용자의 다른 매핑까지 지우기 때문이다.
#[tauri::command(rename_all = "camelCase")]
pub async fn tailscale_serve_disable(_app_state: State<'_, AppState>) -> Result<(), String> {
    let Some(cli) = find_cli() else {
        return Err("tailscale-cli-not-found".into());
    };
    let https_flag = format!("--https={WEB_REMOTE_HTTPS_PORT}");
    run_cli(&cli, &["serve", &https_flag, "off"])
        .await
        .map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 실기계 `tailscale serve status --json` 출력(2026-08-21, v1.102.3).
    /// 8443에 남의 매핑이 살아 있는 상태 — 우리 포트를 찾을 때 이것을 건드리면
    /// 안 된다는 사실이 픽스처 자체에 박혀 있다.
    const REAL_OTHER_PORT: &str = r#"{
  "TCP": { "8443": { "HTTPS": true } },
  "Web": {
    "zm4mini.tailc90d0d.ts.net:8443": {
      "Handlers": { "/": { "Proxy": "http://127.0.0.1:4173" } }
    }
  }
}"#;

    /// 같은 형태에 우리 포트가 함께 등록된 모습.
    const OURS_AND_OTHER: &str = r#"{
  "TCP": { "8443": { "HTTPS": true }, "47443": { "HTTPS": true } },
  "Web": {
    "zm4mini.tailc90d0d.ts.net:8443": {
      "Handlers": { "/": { "Proxy": "http://127.0.0.1:4173" } }
    },
    "zm4mini.tailc90d0d.ts.net:47443": {
      "Handlers": { "/": { "Proxy": "http://100.88.236.3:47800" } }
    }
  }
}"#;

    #[test]
    fn serve_status_ignores_other_ports() {
        let mapping = parse_serve_status(REAL_OTHER_PORT, WEB_REMOTE_HTTPS_PORT);
        assert_eq!(mapping, ServeMapping::default(), "남의 8443 매핑을 우리 것으로 읽으면 안 된다");
    }

    #[test]
    fn serve_status_finds_our_mapping() {
        let mapping = parse_serve_status(OURS_AND_OTHER, WEB_REMOTE_HTTPS_PORT);
        assert!(mapping.https_listener);
        assert_eq!(mapping.upstream.as_deref(), Some("http://100.88.236.3:47800"));
        assert_eq!(
            mapping.host.as_deref(),
            Some("zm4mini.tailc90d0d.ts.net:47443")
        );
        // 다른 포트를 물어보면 그 포트의 사실만 나온다.
        let other = parse_serve_status(OURS_AND_OTHER, 8443);
        assert_eq!(other.upstream.as_deref(), Some("http://127.0.0.1:4173"));
    }

    #[test]
    fn serve_status_tolerates_empty_config() {
        for raw in ["{}", "null", "", "not json"] {
            assert_eq!(
                parse_serve_status(raw, WEB_REMOTE_HTTPS_PORT),
                ServeMapping::default(),
                "입력: {raw:?}"
            );
        }
    }

    /// `serve status`가 TCP 종단만 있고 Web 핸들러가 없는(=TCP 포워드) 상태도
    /// "우리 프록시 등록됨"으로 읽히면 안 된다.
    #[test]
    fn serve_status_without_proxy_has_no_upstream() {
        let raw = r#"{ "TCP": { "47443": { "HTTPS": true } } }"#;
        let mapping = parse_serve_status(raw, WEB_REMOTE_HTTPS_PORT);
        assert!(mapping.https_listener);
        assert!(mapping.upstream.is_none());
    }

    #[test]
    fn node_status_strips_the_trailing_dot() {
        let raw = r#"{
  "BackendState": "Running",
  "Self": { "DNSName": "zm4mini.tailc90d0d.ts.net.", "HostName": "zm4mini" }
}"#;
        let node = parse_node_status(raw);
        assert_eq!(node.dns_name.as_deref(), Some("zm4mini.tailc90d0d.ts.net"));
        assert!(node.backend_running);
    }

    #[test]
    fn node_status_reports_stopped_backend() {
        let raw = r#"{ "BackendState": "Stopped", "Self": { "DNSName": "" } }"#;
        let node = parse_node_status(raw);
        assert!(!node.backend_running);
        assert!(node.dns_name.is_none(), "빈 DNSName은 주소를 만들 수 없다");
        assert_eq!(parse_node_status("{}"), NodeStatus::default());
    }

    #[test]
    fn stderr_summary_takes_the_last_line() {
        let raw = "usage: tailscale serve\n\nerror: invalid port\n\n";
        assert_eq!(summarize_stderr(raw), "error: invalid port");
        assert_eq!(summarize_stderr("   \n"), "unknown error");
        assert_eq!(summarize_stderr(&"x".repeat(500)).chars().count(), 200);
    }
}
