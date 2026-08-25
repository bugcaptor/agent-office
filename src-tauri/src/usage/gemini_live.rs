// src-tauri/src/usage/gemini_live.rs
//
// Gemini CLI 사용량 실시간 조회(kbm #2j4, docs/usage-design.md §12).
//
// **누구를 위한 경로인가**: 라이선스가 있는 Gemini Code Assist 사용자
// (기업/Standard·Enterprise 티어). 개인 무료 티어는 2026-08 Antigravity로
// 이관되면서 gemini CLI의 OAuth 클라이언트가 자격을 잃었고(`loadCodeAssist`
// → `UNSUPPORTED_CLIENT`, `retrieveUserQuota` → 403 `SUBSCRIPTION_REQUIRED`,
// 실측), 그 경우는 `Ineligible`로 분류돼 표시에서 조용히 빠진다(§10 숨김 규칙).
// 그러니 이 모듈이 값을 내놓지 않는 것은 **정상 동작**일 수 있다.
//
// 구조는 claude_live와 같은 결 — CLI에 물어볼 수단이 없어(gemini CLI는
// print 모드에서 슬래시 명령을 확장하지 않고 사용량 서브커맨드도 없다)
// **우리가 자격증명을 읽어 비공식 엔드포인트를 직접 친다.**
//
// 프로토콜(gemini-cli 0.42.0 번들 실측):
//   POST {CODE_ASSIST_ENDPOINT}/v1internal:loadCodeAssist
//        {"cloudaicompanionProject": <projectId?>, "metadata": {...}}
//     → {"cloudaicompanionProject": "...", "currentTier": {...}, "paidTier": {...}}
//   POST {CODE_ASSIST_ENDPOINT}/v1internal:retrieveUserQuota
//        {"project": <projectId>}
//     → {"buckets": [{"modelId": "...", "remainingFraction": 0.42,
//                     "remainingAmount": "120", "resetTime": "..."}]}
//
// 주의할 값 규약: `remainingFraction`은 **잔여**다(Antigravity와 같다).
// 응답이 창 길이를 주지 않으므로 창 종류는 `Unknown` + 라벨=모델 ID다 —
// "며칠 창인지 모른다"를 지어내지 않고 그대로 표시한다.
//
// projectId: `GOOGLE_CLOUD_PROJECT` → `GOOGLE_CLOUD_PROJECT_ID` env가 우선이고
// (CLI와 같은 순서), 없으면 `loadCodeAssist` 응답의 `cloudaicompanionProject`.
// 둘 다 없으면 `ProjectRequired` — 기업 계정에서 흔한 설정 누락이라 별도
// 사유로 세워 UI가 "env를 설정하라"고 말할 수 있게 한다.
//
// 토큰: gemini CLI는 (1) OS Keychain(`gemini-cli-oauth`/`main-account`),
// (2) 암호화 파일 `~/.gemini/gemini-credentials.json`, (3) 레거시 평문
// `~/.gemini/oauth_creds.json` 순으로 자격증명을 둔다. 우리는 (1)과 (3)만
// 읽는다 — (2)는 hostname+username에서 scrypt로 유도한 키의 AES-256-GCM이라
// 크립토 의존을 두 개 더 들여야 하고, 실증 없이 넣을 코드가 아니다. 그 상태의
// 사용자는 `NoCredentials`로 보이며, 이 공백은 설계 문서 §12.4에 적어 뒀다.
//
// 만료 토큰은 **메모리 안에서만** 갱신한다(`oauth2.googleapis.com/token`,
// refresh_token grant). CLI의 자격증명 파일·Keychain에는 쓰지 않는다 — 남의
// 저장소를 앱이 고쳐 쓰면 CLI 쪽 상태와 어긋날 수 있고, 우리가 얻을 것도 없다.
// 토큰 문자열은 로그·진단 어디에도 넣지 않는다(claude_live와 같은 규율).
//
// 갱신에 필요한 OAuth 클라이언트(id/secret)는 **저장소에 하드코딩하지 않고
// 설치된 gemini-cli 번들에서 읽는다.** 설치형 앱 클라이언트라 secret이 기밀은
// 아니지만(공개 npm 번들에 평문으로 실려 있다), 우리 저장소에 박아 두면 (1)
// 비밀 스캐너가 잡는 패턴이라 미러 푸시가 막히거나 발급자에게 경보가 가고,
// (2) 로테이션되는 순간 우리 코드가 조용히 깨진다. 설치본에서 읽으면 둘 다
// 사라진다 — 사용자가 쓰는 CLI와 항상 같은 값을 쓰게 된다.
// `GEMINI_OAUTH_CLIENT_ID`/`GEMINI_OAUTH_CLIENT_SECRET` env로 덮어쓸 수 있다
// (자체 클라이언트를 쓰는 기업 환경 대비). 어느 쪽으로도 못 얻으면 갱신만
// 불가능하고(`RefreshFailed`), 아직 안 만료된 액세스 토큰으로는 그대로 돈다.

use std::path::{Path, PathBuf};
use std::time::Duration;

use parking_lot::Mutex;
use serde::Serialize;
use serde_json::{json, Value};

use super::claude_live::should_fetch;
use super::{parse_iso8601_ms, Provider, ProviderUsage, UsageWindow, UsageWindowKind};

/// Code Assist 내부 API 기본 주소. gemini CLI와 같은 env 오버라이드를 받는다
/// (기업 환경에서 프록시/스테이징을 가리키는 경우가 있다).
const DEFAULT_ENDPOINT: &str = "https://cloudcode-pa.googleapis.com";
const DEFAULT_API_VERSION: &str = "v1internal";

/// 토큰 갱신 엔드포인트(구글 표준 OAuth).
const TOKEN_ENDPOINT: &str = "https://oauth2.googleapis.com/token";

/// 번들에서 OAuth 클라이언트를 찾을 때 쓰는 표지. gemini-cli는 이 두 상수를
/// 번들 청크에 그대로 남긴다(설치형 앱 클라이언트라 감출 수 없다).
const CLIENT_ID_MARKER: &str = "OAUTH_CLIENT_ID = \"";
const CLIENT_SECRET_MARKER: &str = "OAUTH_CLIENT_SECRET = \"";

/// 번들 청크 한 개의 스캔 상한. gemini-cli 0.42의 최대 청크가 14MB이라
/// 여유를 두되, 엉뚱한 큰 파일을 통째로 읽지는 않게 한다.
const MAX_BUNDLE_SCAN_BYTES: u64 = 64 * 1024 * 1024;

const FETCH_TIMEOUT: Duration = Duration::from_secs(10);

/// Keychain 자식 프로세스(`security`) 대기 상한. claude_live와 같은 이유 —
/// 잠긴 Keychain에 폴링이 매달리면 다른 provider 응답까지 늦어진다.
#[cfg(target_os = "macos")]
const KEYCHAIN_TIMEOUT: Duration = Duration::from_secs(5);

/// gemini CLI의 Keychain 서비스명·계정명(번들 상수 `KEYCHAIN_SERVICE_NAME`·
/// `MAIN_ACCOUNT_KEY`).
#[cfg(target_os = "macos")]
const KEYCHAIN_SERVICE: &str = "gemini-cli-oauth";
#[cfg(target_os = "macos")]
const KEYCHAIN_ACCOUNT: &str = "main-account";

/// 만료 판정에 두는 여유. 조회 도중 만료되는 일이 없게 미리 갱신한다.
const EXPIRY_SKEW_MS: i64 = 60 * 1000;

/// 진단 detail에 싣는 문자열 상한.
const DETAIL_MAX_CHARS: usize = 120;

/// 실시간 조회의 마지막 시도 결과. TS `GeminiLiveOutcome` 미러(serde
/// snake_case). Claude와 같은 HTTP 어휘를 쓰되, 이 API에만 있는 두 갈래를
/// 더 세웠다 — 둘 다 "고칠 방법이 서로 다른" 실패라 뭉뚱그리면 안내가 안 된다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum GeminiLiveOutcome {
    /// 아직 한 번도 시도하지 않음(부팅 직후 첫 폴링 전).
    #[default]
    NeverAttempted,
    Ok,
    /// 자격증명을 어느 출처에서도 읽지 못함(미로그인·암호화 파일 저장소).
    NoCredentials,
    /// 토큰 갱신이 거부됨(refresh_token 폐기·재로그인 필요).
    RefreshFailed,
    /// 서버가 토큰을 거부(401/403 중 인증 문제).
    Unauthorized,
    /// 이 계정에 Code Assist 라이선스가 없다. 개인 무료 티어가 Antigravity로
    /// 이관된 뒤의 기본 상태이기도 하다 — 오류가 아니라 "여기 볼 것이 없음".
    Ineligible,
    /// projectId를 env에서도 loadCodeAssist에서도 얻지 못함(기업 계정 설정 누락).
    ProjectRequired,
    /// 그 외 비2xx 응답.
    HttpError,
    /// 요청 자체 실패(타임아웃·연결 실패 등).
    NetworkError,
    /// 2xx인데 본문이 아는 모양이 아님(비공식 API 계약 변화).
    UnexpectedResponse,
}

/// 실시간 조회 진단 스냅샷. TS `GeminiLiveStatus` 미러(camelCase).
/// Codex·Antigravity와 같은 네 필드다 — 토큰 출처를 진단에 싣지 않는 것은
/// 의도적이다(Claude와 달리 갈래가 둘뿐이라 detail 한 줄로 충분하다).
#[derive(Debug, Clone, PartialEq, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct GeminiLiveStatus {
    pub outcome: GeminiLiveOutcome,
    /// 사람이 읽을 진단 보조(예: "HTTP 401", "시간 초과"). **토큰·자격증명
    /// 문자열은 절대 넣지 않는다.**
    pub detail: Option<String>,
    /// 마지막 시도 시각(epoch ms). 스로틀에 막혀 건너뛴 폴링은 시도가 아니다.
    pub last_attempt_ms: Option<i64>,
    /// 마지막 성공 시각(epoch ms). 한 번도 성공한 적 없으면 null.
    pub last_success_ms: Option<i64>,
}

/// 실시간 조회 실패 하나(사유 + 사람이 읽을 보조 문자열).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct GeminiLiveFailure {
    pub outcome: GeminiLiveOutcome,
    pub detail: Option<String>,
}

impl GeminiLiveFailure {
    fn new(outcome: GeminiLiveOutcome, detail: impl Into<String>) -> Self {
        Self {
            outcome,
            detail: Some(truncate(detail.into())),
        }
    }

    fn bare(outcome: GeminiLiveOutcome) -> Self {
        Self {
            outcome,
            detail: None,
        }
    }
}

fn truncate(mut s: String) -> String {
    s = s.trim().replace(['\n', '\r'], " ");
    if s.chars().count() <= DETAIL_MAX_CHARS {
        return s;
    }
    let cut: String = s.chars().take(DETAIL_MAX_CHARS).collect();
    format!("{cut}…")
}

// ── 설정(경로·env) ───────────────────────────────────────────────────────

/// 이 provider가 읽는 환경 설정 묶음. 전역 `std::env::var` 접근을 호출 지점
/// 하나로 모아 두어(ipc/commands/usage.rs의 `resolve_usage_roots`와 같은
/// 규율) 파싱·조립 로직을 순수하게 유지한다.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GeminiEnv {
    /// `~/.gemini`(또는 그에 준하는 자격증명 디렉터리).
    pub config_dir: PathBuf,
    /// `GOOGLE_CLOUD_PROJECT` → `GOOGLE_CLOUD_PROJECT_ID` 순으로 고른 값.
    pub project_id: Option<String>,
    /// `CODE_ASSIST_ENDPOINT` 오버라이드(미설정이면 기본 주소).
    pub endpoint: Option<String>,
    /// `CODE_ASSIST_API_VERSION` 오버라이드.
    pub api_version: Option<String>,
    /// `GEMINI_OAUTH_CLIENT_ID`/`GEMINI_OAUTH_CLIENT_SECRET` 오버라이드.
    /// 미설정이면 설치된 gemini-cli 번들에서 읽는다(헤더 주석 참고).
    pub oauth_client: Option<OAuthClient>,
}

/// 토큰 갱신에 쓰는 OAuth 클라이언트 한 쌍.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OAuthClient {
    pub id: String,
    pub secret: String,
}

impl GeminiEnv {
    /// 순수 조립 — 테스트가 프로세스 전역 env를 건드리지 않게 값으로 받는다.
    /// 빈 문자열 env는 미설정으로 취급한다(일부 런처가 unset 대신 빈 문자열을
    /// 넘긴다 — resolve_usage_roots와 같은 판단).
    pub(crate) fn resolve(
        home: &Path,
        project: Option<&str>,
        project_id: Option<&str>,
        endpoint: Option<&str>,
        api_version: Option<&str>,
        client_id: Option<&str>,
        client_secret: Option<&str>,
    ) -> Self {
        let clean = |v: Option<&str>| {
            v.map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
        };
        Self {
            config_dir: home.join(".gemini"),
            project_id: clean(project).or_else(|| clean(project_id)),
            endpoint: clean(endpoint),
            api_version: clean(api_version),
            // 한쪽만 준 설정은 쓰지 않는다 — 절반짜리 클라이언트로 갱신을
            // 시도해 봐야 invalid_client일 뿐이고, 번들 폴백을 가려 버린다.
            oauth_client: clean(client_id)
                .zip(clean(client_secret))
                .map(|(id, secret)| OAuthClient { id, secret }),
        }
    }

    /// 프로세스 env에서 읽는다. 전역 접근은 여기 한 곳뿐이다.
    pub(super) fn from_process_env(home: &Path) -> Self {
        let var = |k: &str| std::env::var(k).ok();
        Self::resolve(
            home,
            var("GOOGLE_CLOUD_PROJECT").as_deref(),
            var("GOOGLE_CLOUD_PROJECT_ID").as_deref(),
            var("CODE_ASSIST_ENDPOINT").as_deref(),
            var("CODE_ASSIST_API_VERSION").as_deref(),
            var("GEMINI_OAUTH_CLIENT_ID").as_deref(),
            var("GEMINI_OAUTH_CLIENT_SECRET").as_deref(),
        )
    }

    fn method_url(&self, method: &str) -> String {
        let base = self.endpoint.as_deref().unwrap_or(DEFAULT_ENDPOINT);
        let version = self.api_version.as_deref().unwrap_or(DEFAULT_API_VERSION);
        format!("{base}/{version}:{method}")
    }
}

// ── OAuth 클라이언트 찾기 ────────────────────────────────────────────────

/// 프로세스 안에서 한 번만 훑도록 캐시한다. 번들 스캔은 수십 MB를 읽는 일이라
/// 폴링마다 되풀이할 게 못 된다. `None`도 캐시한다 — gemini CLI가 없는 머신에서
/// 갱신이 필요할 때마다 PATH를 다시 뒤지지 않게.
static BUNDLE_CLIENT: Mutex<Option<Option<OAuthClient>>> = Mutex::new(None);

/// 이번 갱신에 쓸 OAuth 클라이언트. env 오버라이드가 우선이고, 없으면 설치된
/// gemini-cli 번들에서 읽는다(헤더 주석의 "저장소에 하드코딩하지 않는다").
fn oauth_client(env: &GeminiEnv) -> Option<OAuthClient> {
    if let Some(client) = env.oauth_client.clone() {
        return Some(client);
    }
    let mut guard = BUNDLE_CLIENT.lock();
    guard.get_or_insert_with(discover_bundle_client).clone()
}

/// PATH의 `gemini`를 따라가 번들 디렉터리를 찾고, 그 안의 청크에서 클라이언트
/// 한 쌍을 읽는다. 못 찾으면 None(갱신만 불가능해진다).
fn discover_bundle_client() -> Option<OAuthClient> {
    let dir = bundle_dir()?;
    for entry in std::fs::read_dir(dir).ok()?.flatten() {
        let path = entry.path();
        if path.extension().is_none_or(|e| e != "js") {
            continue;
        }
        // 큰 청크에만 들어 있지만, 엉뚱한 거대 파일을 통째로 읽지는 않는다.
        if entry.metadata().ok()?.len() > MAX_BUNDLE_SCAN_BYTES {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        if let Some(client) = parse_bundle_client(&text) {
            return Some(client);
        }
    }
    None
}

/// PATH에서 `gemini`를 찾아 심링크를 풀고 그 파일이 든 디렉터리를 돌려준다
/// (Homebrew·npm 전역 설치 모두 `.../bundle/gemini.js`를 가리킨다).
fn bundle_dir() -> Option<std::path::PathBuf> {
    let exe = if cfg!(windows) { "gemini.cmd" } else { "gemini" };
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|dir| dir.join(exe))
        .find(|candidate| candidate.is_file())
        .and_then(|candidate| std::fs::canonicalize(candidate).ok())
        .and_then(|resolved| resolved.parent().map(std::path::Path::to_path_buf))
}

/// 번들 청크 본문 → OAuth 클라이언트(순수). 두 값이 **모두** 있어야 한다 —
/// 한쪽만 찾아 절반짜리로 갱신을 시도할 이유가 없다.
pub(super) fn parse_bundle_client(text: &str) -> Option<OAuthClient> {
    let id = quoted_after(text, CLIENT_ID_MARKER)?;
    let secret = quoted_after(text, CLIENT_SECRET_MARKER)?;
    Some(OAuthClient { id, secret })
}

/// `<marker>` 바로 뒤의 따옴표 문자열을 읽는다(marker가 여는 따옴표까지 포함).
/// 이스케이프가 든 값은 다루지 않는다 — 이 두 상수엔 있을 수 없다.
fn quoted_after(text: &str, marker: &str) -> Option<String> {
    let start = text.find(marker)? + marker.len();
    let rest = text.get(start..)?;
    let end = rest.find('"')?;
    let value = &rest[..end];
    (!value.is_empty()).then(|| value.to_string())
}

// ── 자격증명 ─────────────────────────────────────────────────────────────

/// 자격증명에서 뽑아낸 최소 정보. 토큰 문자열은 이 구조체 밖으로 나가지 않는다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct GeminiCredentials {
    access_token: String,
    refresh_token: Option<String>,
    /// 액세스 토큰 만료 시각(epoch ms). 모르면 None(=만료로 취급하지 않음).
    expiry_ms: Option<i64>,
}

/// 레거시 평문 파일(`oauth_creds.json`) 모양 파싱. `{access_token,
/// refresh_token, expiry_date}`(구글 OAuth2 라이브러리 표준 모양).
pub(super) fn parse_legacy_credentials(json: &str) -> Option<GeminiCredentials> {
    let v: Value = serde_json::from_str(json).ok()?;
    let access_token = v
        .get("access_token")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())?;
    Some(GeminiCredentials {
        access_token: access_token.to_string(),
        refresh_token: v
            .get("refresh_token")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .map(str::to_string),
        expiry_ms: v.get("expiry_date").and_then(Value::as_i64),
    })
}

/// Keychain에 저장되는 모양 파싱. gemini-cli는 MCP 토큰 저장소를 재사용해
/// `{serverName, token: {accessToken, refreshToken, expiresAt, ...}}`로 넣는다.
pub(super) fn parse_keychain_credentials(json: &str) -> Option<GeminiCredentials> {
    let v: Value = serde_json::from_str(json).ok()?;
    let token = v.get("token")?;
    let access_token = token
        .get("accessToken")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())?;
    Some(GeminiCredentials {
        access_token: access_token.to_string(),
        refresh_token: token
            .get("refreshToken")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .map(str::to_string),
        expiry_ms: token.get("expiresAt").and_then(Value::as_i64),
    })
}

/// Keychain(있으면) → 레거시 평문 파일 순으로 자격증명을 읽는다. 어느 쪽도
/// 못 읽으면 None(암호화 파일 저장소는 지원 범위 밖 — 헤더 주석 참고).
async fn read_credentials(config_dir: &Path) -> Option<GeminiCredentials> {
    #[cfg(target_os = "macos")]
    {
        if let Some(creds) = read_keychain(KEYCHAIN_SERVICE, KEYCHAIN_ACCOUNT)
            .await
            .as_deref()
            .and_then(parse_keychain_credentials)
        {
            return Some(creds);
        }
    }
    let content = std::fs::read_to_string(config_dir.join("oauth_creds.json")).ok()?;
    parse_legacy_credentials(&content)
}

#[cfg(target_os = "macos")]
async fn read_keychain(service: &str, account: &str) -> Option<String> {
    let output = tokio::time::timeout(
        KEYCHAIN_TIMEOUT,
        tokio::process::Command::new("security")
            .args(["find-generic-password", "-s", service, "-a", account, "-w"])
            .stdin(std::process::Stdio::null())
            .kill_on_drop(true)
            .output(),
    )
    .await
    .ok()?
    .ok()?;
    if !output.status.success() {
        return None;
    }
    let value = String::from_utf8(output.stdout).ok()?;
    let trimmed = value.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

/// 액세스 토큰이 곧 만료되는지(순수). 만료 시각을 모르면 "쓸 수 있다"로 본다
/// — 실제로 만료됐다면 서버가 401로 알려 주고, 그때 `Unauthorized`로 분류된다.
fn is_expired(expiry_ms: Option<i64>, now_ms: i64) -> bool {
    expiry_ms.is_some_and(|exp| exp - EXPIRY_SKEW_MS <= now_ms)
}

// ── HTTP ─────────────────────────────────────────────────────────────────

fn http_client() -> Result<reqwest::Client, GeminiLiveFailure> {
    reqwest::Client::builder()
        .timeout(FETCH_TIMEOUT)
        // claude_live와 같은 이유로 번들 루트와 OS 스토어 루트를 모두 신뢰한다
        // (사내 프록시가 TLS를 가로채는 환경 대비).
        .tls_built_in_webpki_certs(true)
        .tls_built_in_native_certs(true)
        .build()
        .map_err(|_| {
            GeminiLiveFailure::new(GeminiLiveOutcome::NetworkError, "클라이언트 초기화 실패")
        })
}

/// reqwest 오류 → 고정 어휘(오류 문자열에 URL이 섞여 나오므로 그대로 싣지 않는다).
fn network_failure(e: &reqwest::Error) -> GeminiLiveFailure {
    let detail = if e.is_timeout() {
        "시간 초과"
    } else if e.is_connect() {
        "연결 실패"
    } else {
        "요청 실패"
    };
    GeminiLiveFailure::new(GeminiLiveOutcome::NetworkError, detail)
}

/// 만료된 액세스 토큰을 refresh_token으로 갱신한다. 결과는 메모리에만 남는다.
async fn refresh_access_token(
    client: &reqwest::Client,
    oauth: &OAuthClient,
    refresh_token: &str,
    now_ms: i64,
) -> Result<GeminiCredentials, GeminiLiveFailure> {
    let resp = client
        .post(TOKEN_ENDPOINT)
        .form(&[
            ("client_id", oauth.id.as_str()),
            ("client_secret", oauth.secret.as_str()),
            ("refresh_token", refresh_token),
            ("grant_type", "refresh_token"),
        ])
        .send()
        .await
        .map_err(|e| network_failure(&e))?;
    let status = resp.status();
    if !status.is_success() {
        return Err(GeminiLiveFailure::new(
            GeminiLiveOutcome::RefreshFailed,
            format!("HTTP {}", status.as_u16()),
        ));
    }
    let body: Value = resp.json().await.map_err(|_| {
        GeminiLiveFailure::new(GeminiLiveOutcome::RefreshFailed, "본문이 JSON이 아님")
    })?;
    let access_token = body
        .get("access_token")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .ok_or_else(|| {
            GeminiLiveFailure::new(GeminiLiveOutcome::RefreshFailed, "access_token 없음")
        })?;
    let expiry_ms = body
        .get("expires_in")
        .and_then(Value::as_i64)
        .map(|secs| now_ms + secs * 1000);
    Ok(GeminiCredentials {
        access_token: access_token.to_string(),
        // 갱신 응답은 refresh_token을 되돌려주지 않는다 — 기존 것이 계속 유효하다.
        refresh_token: Some(refresh_token.to_string()),
        expiry_ms,
    })
}

/// `v1internal:<method>` POST 한 번. 비2xx는 본문을 읽어 사유를 가른다.
async fn post(
    client: &reqwest::Client,
    env: &GeminiEnv,
    token: &str,
    method: &str,
    body: Value,
) -> Result<Value, GeminiLiveFailure> {
    let resp = client
        .post(env.method_url(method))
        .bearer_auth(token)
        .json(&body)
        .send()
        .await
        .map_err(|e| network_failure(&e))?;
    let status = resp.status();
    if !status.is_success() {
        let text = resp.text().await.unwrap_or_default();
        return Err(http_failure(status.as_u16(), &text));
    }
    resp.json().await.map_err(|_| {
        GeminiLiveFailure::new(GeminiLiveOutcome::UnexpectedResponse, "본문이 JSON이 아님")
    })
}

/// 비2xx → 실패 분류(순수). 401/403을 뭉뚱그리지 않는 것이 핵심이다:
/// 403 + `SUBSCRIPTION_REQUIRED`/`UNSUPPORTED_CLIENT`는 "재로그인하면 된다"가
/// **아니라** "이 계정엔 볼 한도가 없다"는 뜻이고, 안내가 정반대다.
pub(super) fn http_failure(status: u16, body: &str) -> GeminiLiveFailure {
    let lower = body.to_ascii_lowercase();
    let ineligible = lower.contains("subscription_required")
        || lower.contains("unsupported_client")
        || lower.contains("ineligible");
    let outcome = match status {
        403 if ineligible => GeminiLiveOutcome::Ineligible,
        401 | 403 => GeminiLiveOutcome::Unauthorized,
        _ => GeminiLiveOutcome::HttpError,
    };
    GeminiLiveFailure::new(outcome, format!("HTTP {status}"))
}

// ── 조회 ─────────────────────────────────────────────────────────────────

/// 실시간 조회 1회. 성공하면 `ProviderUsage`(fetched_at_ms = 조회 시각).
///
/// `cached_project`는 직전 성공에서 알아낸 projectId다 — 있으면
/// `loadCodeAssist` 왕복을 건너뛴다. 플랜 표시명도 그때 함께 알아낸 값을 쓴다.
pub(super) async fn fetch_live(
    env: &GeminiEnv,
    cached: Option<&ProjectInfo>,
    now_ms: i64,
) -> Result<(ProviderUsage, ProjectInfo), GeminiLiveFailure> {
    let creds = read_credentials(&env.config_dir)
        .await
        .ok_or_else(|| GeminiLiveFailure::bare(GeminiLiveOutcome::NoCredentials))?;
    let client = http_client()?;

    let creds = if is_expired(creds.expiry_ms, now_ms) {
        let refresh = creds.refresh_token.as_deref().ok_or_else(|| {
            GeminiLiveFailure::new(GeminiLiveOutcome::RefreshFailed, "refresh_token 없음")
        })?;
        // 클라이언트를 못 찾으면 갱신만 못 한다(만료된 토큰은 그대로 쓸 수 없다).
        let oauth = oauth_client(env).ok_or_else(|| {
            GeminiLiveFailure::new(
                GeminiLiveOutcome::RefreshFailed,
                "gemini CLI의 OAuth 클라이언트를 찾지 못함",
            )
        })?;
        refresh_access_token(&client, &oauth, refresh, now_ms).await?
    } else {
        creds
    };

    let project = match cached {
        Some(info) => info.clone(),
        None => discover_project(&client, env, &creds.access_token).await?,
    };

    let quota = post(
        &client,
        env,
        &creds.access_token,
        "retrieveUserQuota",
        json!({ "project": project.id }),
    )
    .await?;
    let usage = parse_quota(&quota, project.plan_label.as_deref(), now_ms)?;
    Ok((usage, project))
}

/// 직전 성공에서 알아낸 계정 정보(projectId + 플랜 표시명). 매 폴링마다
/// `loadCodeAssist`를 다시 치지 않으려고 상태에 남긴다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ProjectInfo {
    pub id: String,
    pub plan_label: Option<String>,
}

/// projectId(와 플랜 표시명)를 정한다. env가 있으면 그것으로 확정하되
/// 플랜 이름을 알아내려 `loadCodeAssist`는 한 번 친다(실패하면 이름만 없다).
async fn discover_project(
    client: &reqwest::Client,
    env: &GeminiEnv,
    token: &str,
) -> Result<ProjectInfo, GeminiLiveFailure> {
    let body = json!({
        "cloudaicompanionProject": env.project_id,
        "metadata": {
            "ideType": "IDE_UNSPECIFIED",
            "platform": "PLATFORM_UNSPECIFIED",
            "pluginType": "GEMINI",
            "duetProject": env.project_id,
        }
    });
    match post(client, env, token, "loadCodeAssist", body).await {
        Ok(res) => parse_project(&res, env.project_id.as_deref()),
        // loadCodeAssist가 막혀도 env로 projectId를 알면 한도 조회는 시도해 본다
        // — 두 엔드포인트의 권한이 항상 같지는 않다. 이름만 못 붙일 뿐이다.
        Err(failure) => match env.project_id.clone() {
            Some(id) => Ok(ProjectInfo {
                id,
                plan_label: None,
            }),
            None => Err(failure),
        },
    }
}

/// `loadCodeAssist` 응답 → projectId + 플랜 표시명(순수). env 값이 있으면
/// 그것이 우선이고(CLI와 같은 순서), 없으면 `cloudaicompanionProject`.
///
/// projectId를 못 얻었을 때의 갈래는 CLI의 `throwIneligibleOrProjectIdError`를
/// 그대로 따른다: 응답에 `ineligibleTiers`가 있으면 **라이선스 문제**이고,
/// 없으면 프로젝트 env 설정 누락이다. 이 API는 자격 없음을 200 본문으로
/// 알려 주기도 해서(개인 계정 실측), 상태코드만 봐서는 구분되지 않는다.
pub(super) fn parse_project(
    res: &Value,
    env_project: Option<&str>,
) -> Result<ProjectInfo, GeminiLiveFailure> {
    let id = env_project
        .map(str::to_string)
        .or_else(|| {
            res.get("cloudaicompanionProject")
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
        })
        .ok_or_else(|| ineligible_or_project_required(res))?;
    // 유료 티어 이름이 있으면 그쪽이 사용자가 아는 이름이다.
    let plan_label = ["paidTier", "currentTier"]
        .iter()
        .find_map(|k| res.get(k))
        .and_then(|tier| tier.get("name").or_else(|| tier.get("id")))
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    Ok(ProjectInfo { id, plan_label })
}

/// projectId를 못 얻은 이유를 가른다(순수) — CLI의
/// `throwIneligibleOrProjectIdError`와 같은 판단. 자격 없음이면 서버가 준
/// 사람 대상 문구(`reasonMessage`)를 detail로 실어 준다.
fn ineligible_or_project_required(res: &Value) -> GeminiLiveFailure {
    let tiers = res.get("ineligibleTiers").and_then(Value::as_array);
    match tiers.filter(|t| !t.is_empty()) {
        Some(tiers) => {
            let reason = tiers
                .iter()
                .find_map(|t| t.get("reasonMessage").and_then(Value::as_str))
                .or_else(|| {
                    tiers
                        .iter()
                        .find_map(|t| t.get("reasonCode").and_then(Value::as_str))
                });
            match reason {
                Some(reason) => GeminiLiveFailure::new(GeminiLiveOutcome::Ineligible, reason),
                None => GeminiLiveFailure::bare(GeminiLiveOutcome::Ineligible),
            }
        }
        None => GeminiLiveFailure::bare(GeminiLiveOutcome::ProjectRequired),
    }
}

/// `retrieveUserQuota` 응답 → `ProviderUsage`(순수).
///
/// 버킷은 모델별이고 **창 길이를 주지 않는다**. 그래서 종류는 `Unknown`이고
/// 라벨에 모델 ID를 싣는다 — 5시간인지 하루인지 지어내지 않는다. 리셋 시각은
/// `resetTime`(RFC3339)에서 온다.
pub(super) fn parse_quota(
    res: &Value,
    plan_label: Option<&str>,
    now_ms: i64,
) -> Result<ProviderUsage, GeminiLiveFailure> {
    let buckets = res
        .get("buckets")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            GeminiLiveFailure::new(GeminiLiveOutcome::UnexpectedResponse, "buckets 없음")
        })?;
    let windows: Vec<UsageWindow> = buckets.iter().filter_map(parse_bucket).collect();
    if windows.is_empty() {
        return Err(GeminiLiveFailure::new(
            GeminiLiveOutcome::UnexpectedResponse,
            "아는 한도 버킷이 없음",
        ));
    }
    Ok(ProviderUsage {
        provider: Provider::Gemini,
        fetched_at_ms: now_ms,
        plan_label: plan_label.map(str::to_string),
        windows,
    })
}

/// 버킷 하나 → 윈도. `remainingFraction`은 **잔여**다(Antigravity와 같다).
fn parse_bucket(bucket: &Value) -> Option<UsageWindow> {
    let remaining = bucket.get("remainingFraction").and_then(Value::as_f64)?;
    let label = bucket
        .get("modelId")
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|s| !s.is_empty())?;
    Some(UsageWindow {
        kind: UsageWindowKind::Unknown,
        label: Some(label.to_string()),
        used_percent: ((1.0 - remaining) * 100.0).clamp(0.0, 100.0),
        resets_at_ms: bucket
            .get("resetTime")
            .and_then(Value::as_str)
            .and_then(parse_iso8601_ms),
        // 응답이 창 길이를 주지 않는다. 모르는 것은 null로 둔다.
        window_minutes: None,
        is_active: None,
    })
}

// ── 스로틀 상태 ──────────────────────────────────────────────────────────

/// 실시간 조회 메모리 상태. 다른 provider와 같은 규율: 판단·기록만 락 안에서
/// 하고 네트워크 왕복은 락 밖에서 한다.
#[derive(Default)]
pub struct GeminiLiveState {
    inner: Mutex<GeminiLiveInner>,
}

#[derive(Default)]
struct GeminiLiveInner {
    last_success: Option<ProviderUsage>,
    last_attempt_ms: Option<i64>,
    outcome: GeminiLiveOutcome,
    detail: Option<String>,
    /// 직전 성공에서 알아낸 계정 정보(loadCodeAssist 왕복 절약).
    project: Option<ProjectInfo>,
}

impl GeminiLiveState {
    pub(crate) fn begin_attempt_if_due(&self, now_ms: i64) -> bool {
        let mut guard = self.inner.lock();
        let due = should_fetch(guard.last_success.as_ref(), guard.last_attempt_ms, now_ms);
        if due {
            guard.last_attempt_ms = Some(now_ms);
        }
        due
    }

    pub(super) fn cached_project(&self) -> Option<ProjectInfo> {
        self.inner.lock().project.clone()
    }

    pub(super) fn record_success(&self, usage: ProviderUsage, project: ProjectInfo) {
        let mut guard = self.inner.lock();
        guard.last_success = Some(usage);
        guard.project = Some(project);
        guard.outcome = GeminiLiveOutcome::Ok;
        guard.detail = None;
    }

    /// 실패 사유를 기록한다. `last_success`는 건드리지 않는다(파일 캐시가 없어
    /// 이것이 유일한 표시값이다). 다만 **계정 정보 캐시는 버린다** — 계정이
    /// 바뀌었거나 프로젝트 권한이 사라진 경우 낡은 projectId로 계속 물으면
    /// 영영 복구되지 않는다.
    pub(super) fn record_failure(&self, failure: GeminiLiveFailure) {
        let mut guard = self.inner.lock();
        guard.outcome = failure.outcome;
        guard.detail = failure.detail;
        guard.project = None;
    }

    pub(super) fn status(&self) -> GeminiLiveStatus {
        let guard = self.inner.lock();
        GeminiLiveStatus {
            outcome: guard.outcome,
            detail: guard.detail.clone(),
            last_attempt_ms: guard.last_attempt_ms,
            last_success_ms: guard.last_success.as_ref().map(|u| u.fetched_at_ms),
        }
    }

    pub(super) fn last_success(&self) -> Option<ProviderUsage> {
        self.inner.lock().last_success.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env_for(home: &str) -> GeminiEnv {
        GeminiEnv::resolve(Path::new(home), None, None, None, None, None, None)
    }

    #[test]
    fn resolve_prefers_google_cloud_project_over_project_id() {
        let env = GeminiEnv::resolve(
            Path::new("/home/u"),
            Some("primary"),
            Some("secondary"),
            None,
            None,
            None,
            None,
        );
        assert_eq!(env.config_dir, Path::new("/home/u/.gemini"));
        assert_eq!(env.project_id.as_deref(), Some("primary"));
    }

    #[test]
    fn resolve_treats_blank_env_as_unset() {
        let env = GeminiEnv::resolve(
            Path::new("/home/u"),
            Some("  "),
            Some("fallback"),
            Some(""),
            None,
            None,
            None,
        );
        assert_eq!(env.project_id.as_deref(), Some("fallback"));
        assert_eq!(env.endpoint, None);
    }

    #[test]
    fn oauth_override_needs_both_halves() {
        // 한쪽만 준 설정은 무시하고 번들 폴백에 맡긴다 — 절반짜리 클라이언트로
        // 갱신해 봐야 invalid_client일 뿐이다.
        let only_id =
            GeminiEnv::resolve(Path::new("/h"), None, None, None, None, Some("id"), None);
        assert_eq!(only_id.oauth_client, None);
        let both = GeminiEnv::resolve(
            Path::new("/h"),
            None,
            None,
            None,
            None,
            Some("id"),
            Some("sec"),
        );
        assert_eq!(
            both.oauth_client,
            Some(OAuthClient {
                id: "id".into(),
                secret: "sec".into()
            })
        );
    }

    #[test]
    fn method_url_honours_endpoint_override() {
        let mut env = env_for("/home/u");
        assert_eq!(
            env.method_url("retrieveUserQuota"),
            "https://cloudcode-pa.googleapis.com/v1internal:retrieveUserQuota"
        );
        env.endpoint = Some("https://staging-cloudcode-pa.googleapis.com".into());
        env.api_version = Some("v1beta".into());
        assert_eq!(
            env.method_url("retrieveUserQuota"),
            "https://staging-cloudcode-pa.googleapis.com/v1beta:retrieveUserQuota"
        );
    }

    /// 번들 청크에서 클라이언트를 읽어내는 모양(gemini-cli 0.42 실측 발췌).
    /// 값 자체는 저장소에 두지 않으므로 테스트도 가짜 값으로만 모양을 굳힌다.
    #[test]
    fn parses_oauth_client_out_of_a_bundle_chunk() {
        let text = r#"var OAUTH_CLIENT_ID = "123-abc.apps.googleusercontent.com";
var OAUTH_CLIENT_SECRET = "SECRET-VALUE";
var OAUTH_SCOPE = ["..."];"#;
        let client = parse_bundle_client(text).unwrap();
        assert_eq!(client.id, "123-abc.apps.googleusercontent.com");
        assert_eq!(client.secret, "SECRET-VALUE");
    }

    #[test]
    fn bundle_client_needs_both_constants() {
        // 한쪽만 있는 청크(대부분의 청크가 그렇다)는 건너뛰어야 한다.
        assert_eq!(parse_bundle_client(r#"var OAUTH_CLIENT_ID = "only";"#), None);
        assert_eq!(parse_bundle_client("무관한 청크"), None);
        // 빈 값도 못 쓴다.
        assert_eq!(
            parse_bundle_client(r#"OAUTH_CLIENT_ID = "";OAUTH_CLIENT_SECRET = "s";"#),
            None
        );
    }

    #[test]
    fn parses_legacy_credentials_file() {
        let json = r#"{"access_token":"at","refresh_token":"rt","expiry_date":1785393014484}"#;
        let creds = parse_legacy_credentials(json).unwrap();
        assert_eq!(creds.access_token, "at");
        assert_eq!(creds.refresh_token.as_deref(), Some("rt"));
        assert_eq!(creds.expiry_ms, Some(1_785_393_014_484));
    }

    #[test]
    fn parses_keychain_credentials() {
        let json = r#"{"serverName":"main-account","token":{"accessToken":"at","refreshToken":"rt","tokenType":"Bearer","expiresAt":1785393014484},"updatedAt":1}"#;
        let creds = parse_keychain_credentials(json).unwrap();
        assert_eq!(creds.access_token, "at");
        assert_eq!(creds.expiry_ms, Some(1_785_393_014_484));
    }

    #[test]
    fn credentials_without_access_token_are_rejected() {
        assert_eq!(parse_legacy_credentials(r#"{"refresh_token":"rt"}"#), None);
        assert_eq!(parse_legacy_credentials(r#"{"access_token":""}"#), None);
        assert_eq!(parse_keychain_credentials(r#"{"token":{}}"#), None);
        assert_eq!(parse_keychain_credentials("not json"), None);
    }

    #[test]
    fn expiry_uses_a_skew_so_a_token_never_dies_mid_flight() {
        assert!(!is_expired(Some(10_000_000), 9_000_000));
        // 만료 1분 전이면 이미 갱신 대상이다.
        assert!(is_expired(Some(10_000_000), 10_000_000 - EXPIRY_SKEW_MS));
        assert!(is_expired(Some(10_000_000), 10_000_001));
        // 만료 시각을 모르면 그대로 써 본다(401이 오면 그때 분류한다).
        assert!(!is_expired(None, i64::MAX));
    }

    /// 개인 계정에서 실제로 돌아온 응답(2026-08-25). 라이선스 없음은 인증
    /// 문제가 아니므로 `Unauthorized`가 아니라 `Ineligible`이어야 한다.
    #[test]
    fn subscription_required_is_ineligible_not_unauthorized() {
        let body = r#"{"error":{"code":403,"status":"PERMISSION_DENIED","details":[{"reason":"SUBSCRIPTION_REQUIRED"}]}}"#;
        assert_eq!(
            http_failure(403, body).outcome,
            GeminiLiveOutcome::Ineligible
        );
    }

    #[test]
    fn plain_401_is_unauthorized() {
        assert_eq!(
            http_failure(401, r#"{"error":{"message":"invalid credentials"}}"#).outcome,
            GeminiLiveOutcome::Unauthorized
        );
        assert_eq!(
            http_failure(403, r#"{"error":{"message":"forbidden"}}"#).outcome,
            GeminiLiveOutcome::Unauthorized
        );
        assert_eq!(http_failure(500, "").outcome, GeminiLiveOutcome::HttpError);
    }

    #[test]
    fn parses_project_from_response_and_env() {
        let res: Value = serde_json::from_str(
            r#"{"cloudaicompanionProject":"discovered","paidTier":{"id":"standard-tier","name":"Gemini Code Assist Standard"}}"#,
        )
        .unwrap();
        let from_res = parse_project(&res, None).unwrap();
        assert_eq!(from_res.id, "discovered");
        assert_eq!(
            from_res.plan_label.as_deref(),
            Some("Gemini Code Assist Standard")
        );
        // env가 있으면 그것이 우선(CLI와 같은 순서).
        assert_eq!(parse_project(&res, Some("from-env")).unwrap().id, "from-env");
    }

    #[test]
    fn project_falls_back_to_current_tier_name_and_can_be_absent() {
        let res: Value =
            serde_json::from_str(r#"{"cloudaicompanionProject":"p","currentTier":{"id":"free-tier"}}"#)
                .unwrap();
        assert_eq!(parse_project(&res, None).unwrap().plan_label.as_deref(), Some("free-tier"));
        // projectId를 어디서도 못 얻고 자격 정보도 없으면 env 설정 누락이다.
        let empty: Value = serde_json::from_str("{}").unwrap();
        assert_eq!(
            parse_project(&empty, None).unwrap_err().outcome,
            GeminiLiveOutcome::ProjectRequired
        );
    }

    /// 개인 계정 실측(2026-08-25): 서버가 **200**으로 자격 없음을 알려 준다.
    /// 상태코드만 보면 ProjectRequired로 잘못 잡히는 자리다.
    #[test]
    fn ineligible_tiers_in_a_200_body_beat_project_required() {
        let res: Value = serde_json::from_str(
            r#"{"ineligibleTiers":[{"reasonCode":"UNSUPPORTED_CLIENT",
                 "reasonMessage":"This client is no longer supported for Gemini Code Assist for individuals.",
                 "tierId":"free-tier","tierName":"Gemini Code Assist for individuals"}]}"#,
        )
        .unwrap();
        let failure = parse_project(&res, None).unwrap_err();
        assert_eq!(failure.outcome, GeminiLiveOutcome::Ineligible);
        assert!(failure.detail.unwrap().contains("no longer supported"));
    }

    #[test]
    fn env_project_wins_even_when_the_account_looks_ineligible() {
        // 기업 계정에서 일부 티어만 ineligible로 오는 응답이 있을 수 있다 —
        // projectId를 알면 한도 조회는 그대로 시도한다.
        let res: Value =
            serde_json::from_str(r#"{"ineligibleTiers":[{"reasonCode":"X"}]}"#).unwrap();
        assert_eq!(parse_project(&res, Some("p")).unwrap().id, "p");
    }

    /// 번들에서 읽어낸 응답 계약(gemini-cli 0.42.0 `refreshUserQuota`).
    fn quota_sample() -> Value {
        serde_json::from_str(
            r#"{"buckets":[
                 {"modelId":"gemini-3-pro","remainingFraction":0.25,
                  "remainingAmount":"60","resetTime":"2026-08-26T00:00:00Z"},
                 {"modelId":"gemini-3-flash","remainingFraction":1,
                  "resetTime":"2026-08-26T00:00:00Z"}
               ]}"#,
        )
        .unwrap()
    }

    #[test]
    fn parses_quota_buckets_into_model_labelled_windows() {
        let usage = parse_quota(&quota_sample(), Some("Standard"), 7_000).unwrap();
        assert_eq!(usage.provider, Provider::Gemini);
        assert_eq!(usage.fetched_at_ms, 7_000);
        assert_eq!(usage.plan_label.as_deref(), Some("Standard"));
        assert_eq!(usage.windows.len(), 2);

        let pro = &usage.windows[0];
        // 창 길이를 주지 않는 응답이라 종류는 Unknown, 뜻은 라벨이 진다.
        assert_eq!(pro.kind, UsageWindowKind::Unknown);
        assert_eq!(pro.label.as_deref(), Some("gemini-3-pro"));
        assert_eq!(pro.window_minutes, None);
        // remainingFraction은 잔여 — 25% 남았으면 75% 쓴 것이다.
        assert_eq!(pro.used_percent, 75.0);
        assert_eq!(pro.resets_at_ms, parse_iso8601_ms("2026-08-26T00:00:00Z"));

        assert_eq!(usage.windows[1].used_percent, 0.0);
    }

    #[test]
    fn quota_without_buckets_is_unexpected_response() {
        let empty: Value = serde_json::from_str("{}").unwrap();
        assert_eq!(
            parse_quota(&empty, None, 0).unwrap_err().outcome,
            GeminiLiveOutcome::UnexpectedResponse
        );
        // modelId·remainingFraction이 없는 버킷만 오면 쓸 창이 하나도 없다.
        let junk: Value = serde_json::from_str(r#"{"buckets":[{"modelId":"m"}]}"#).unwrap();
        assert_eq!(
            parse_quota(&junk, None, 0).unwrap_err().outcome,
            GeminiLiveOutcome::UnexpectedResponse
        );
    }

    #[test]
    fn state_throttles_and_keeps_last_success_on_failure() {
        let state = GeminiLiveState::default();
        assert!(state.begin_attempt_if_due(1_000_000));
        assert!(!state.begin_attempt_if_due(1_060_000));

        let usage = parse_quota(&quota_sample(), None, 5_000).unwrap();
        let project = ProjectInfo {
            id: "p".into(),
            plan_label: None,
        };
        state.record_success(usage.clone(), project.clone());
        assert_eq!(state.cached_project(), Some(project));

        state.record_failure(GeminiLiveFailure::bare(GeminiLiveOutcome::Ineligible));
        assert_eq!(state.last_success(), Some(usage));
        assert_eq!(state.status().outcome, GeminiLiveOutcome::Ineligible);
        // 계정 정보 캐시는 버려야 계정·권한 변경에서 복구된다.
        assert_eq!(state.cached_project(), None);
    }

    /// 실제 자격증명으로 왕복한다. 라이선스 없는 계정에서는 `Ineligible`이
    /// 정상이므로 성공을 단언하지 않고 **분류가 서는지**만 본다.
    /// `cargo test -p agent-office --lib -- --ignored gemini_live`.
    /// 설치된 gemini-cli에서 클라이언트를 실제로 읽어내는지. 값은 출력하지
    /// 않고 **모양만** 본다 — 저장소에도 로그에도 남기지 않는다.
    #[test]
    #[ignore = "실제 gemini CLI 설치가 필요하다"]
    fn discovers_oauth_client_from_the_installed_bundle() {
        let client = discover_bundle_client().expect("gemini 번들에서 클라이언트를 찾지 못함");
        assert!(client.id.ends_with(".apps.googleusercontent.com"));
        assert!(!client.secret.is_empty());
    }

    #[test]
    #[ignore = "실제 gemini 자격증명이 필요하다"]
    fn live_fetch_against_real_credentials() {
        let home = PathBuf::from(std::env::var("HOME").unwrap());
        let env = GeminiEnv::from_process_env(&home);
        let rt = tokio::runtime::Runtime::new().unwrap();
        match rt.block_on(fetch_live(&env, None, 1_000)) {
            Ok((usage, project)) => {
                assert!(!usage.windows.is_empty());
                assert!(!project.id.is_empty());
            }
            Err(failure) => {
                eprintln!("gemini live: {:?} {:?}", failure.outcome, failure.detail);
                assert_ne!(failure.outcome, GeminiLiveOutcome::NeverAttempted);
            }
        }
    }
}
