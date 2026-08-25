// src-tauri/src/usage/mod.rs
//
// 구독 사용량(rate limit) 스냅샷 조립. 설계: docs/usage-limits-design.md.
// 기존 session-analytics 관례를 따라 백엔드는 정규화된 원시 스냅샷만 반환하고
// 집계·해석·표시는 프런트가 한다. 두 소스(claude/codex)는 서로 독립적으로
// 파싱되며, 실패한 소스는 해당 provider가 None일 뿐 조립 자체는 항상 성공한다
// (반환은 오류 없이 축소로만 나타남).
//
// 단위 정규화는 전부 여기서: resets_at → epoch ms(Claude ISO8601 파싱,
// Codex 유닉스 초→ms), 신선도(fetchedAtMs)도 epoch ms.

mod antigravity_live;
mod claude;
mod claude_live;
mod claude_live_fallback;
mod codex;
mod codex_live;
mod gemini_live;

use std::path::Path;

use serde::Serialize;

pub use antigravity_live::{AntigravityLiveOutcome, AntigravityLiveStatus};
pub use claude_live::LiveUsageState;
pub use codex_live::{CodexLiveOutcome, CodexLiveStatus};
pub use gemini_live::{GeminiLiveOutcome, GeminiLiveStatus};

/// Claude 실시간 조회의 마지막 시도 결과. TS `LiveFetchOutcome` 미러
/// (serde snake_case). 왜 표시값이 낡았는지를 UI가 사용자에게 설명하기 위한
/// 진단값이다 — 실패해도 스냅샷은 파일 캐시로 정상 반환되므로 이 값은
/// "표시 실패"가 아니라 "신선도의 이유"를 뜻한다(docs/usage-design.md §7).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum LiveFetchOutcome {
    /// 아직 한 번도 시도하지 않음(부팅 직후 첫 폴링 전).
    #[default]
    NeverAttempted,
    Ok,
    /// 토큰을 어느 출처에서도 읽지 못함(Keychain 거부/잠김·타임아웃 + 파일 부재).
    NoCredentials,
    /// 서버가 토큰을 거부(401/403). 만료됐거나 재로그인이 필요하다.
    Unauthorized,
    /// 그 외 비2xx 응답.
    HttpError,
    /// 요청 자체 실패(타임아웃·연결 실패 등).
    NetworkError,
    /// 2xx인데 본문이 아는 모양이 아님(비공식 API 계약 변경).
    UnexpectedResponse,
}

/// 토큰을 읽어낸 출처. TS `TokenSource` 미러. 진단에 필요하다 — 예컨대
/// `file` + `unauthorized`는 "Keychain 접근이 막혀 파일로 폴백했는데 그 파일의
/// 토큰이 낡음"이라는 흔한 실패 조합을 가리킨다. 토큰 값 자체는 절대 담지 않는다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TokenSource {
    KeychainScoped,
    KeychainLegacy,
    File,
}

/// 사용량 값을 실제로 얻어낸 전송 수단. TS `FetchTransport` 미러.
/// `direct`가 아니라는 것은 앱의 자체 HTTPS가 이 환경에서 막혀 있고(사내
/// MITM 프록시·self-signed 루트 등) 외부 프로세스로 우회 중이라는 뜻이다.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FetchTransport {
    /// 앱 안의 reqwest로 직접 조회(기본 경로).
    Direct,
    /// `curl` 자식 프로세스 경유(OS 인증서 스토어·프록시 설정을 그대로 씀).
    Curl,
    /// `claude -p /usage` 경유. 현재 CLI는 사용량을 돌려주지 않아 사실상
    /// 도달하지 않는 값이다(claude_live_fallback 헤더 주석 참조).
    ClaudeCli,
}

/// Claude 실시간 조회 진단 스냅샷. TS `ClaudeLiveStatus` 미러(camelCase).
/// 스냅샷마다 항상 존재한다(실패해도 null이 아니다 — "모름"은 `NeverAttempted`).
#[derive(Debug, Clone, PartialEq, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ClaudeLiveStatus {
    pub outcome: LiveFetchOutcome,
    /// 마지막 시도에서 토큰을 읽어낸 출처. 토큰을 못 읽었으면 null.
    pub token_source: Option<TokenSource>,
    /// 사람이 읽을 진단 보조(예: "HTTP 500", "시간 초과"). **토큰·자격증명
    /// 문자열은 절대 넣지 않는다**(설계 §6.2).
    pub detail: Option<String>,
    /// 마지막 시도 시각(epoch ms). 스로틀에 막혀 건너뛴 폴링은 시도가 아니다.
    pub last_attempt_ms: Option<i64>,
    /// 마지막 성공 시각(epoch ms). 한 번도 성공한 적 없으면 null.
    pub last_success_ms: Option<i64>,
    /// 마지막으로 값을 얻어낸 전송 수단. 한 번도 성공한 적 없으면 null.
    /// 실패는 이 값을 지우지 않는다 — "아까는 curl로 받아온 값"이라는 설명이
    /// 실패 중에도 성립해야 하기 때문이다.
    pub via: Option<FetchTransport>,
}

/// 한도 윈도 종류. TS `UsageWindowKind` 미러(serde snake_case).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum UsageWindowKind {
    Session,
    Weekly,
    /// 모델별 5시간 창(Codex `rateLimitsByLimitId`의 이름 붙은 버킷).
    /// `Session`과 구분해 두어야 뱃지의 "5시간" 자리를 모델별 창이 가로채지
    /// 않는다 — 라벨(모델 표시명)은 `UsageWindow::label`에 있다.
    SessionModel,
    WeeklyModel,
    Unknown,
}

/// CLI provider. TS `"claude" | "codex" | "antigravity" | "gemini"` 미러
/// (serde lowercase).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Provider {
    Claude,
    Codex,
    /// Antigravity(`agy` CLI). 파일 캐시 미러가 없어 실시간 조회에만 의존한다
    /// — antigravity_live.rs 헤더 주석 참고.
    Antigravity,
    /// Gemini CLI(Code Assist 라이선스 보유 계정). 역시 파일 캐시가 없고,
    /// 라이선스가 없는 계정에서는 값이 없는 것이 정상이다 — gemini_live.rs.
    Gemini,
}

/// 한도 윈도 1개. TS `UsageWindow` 미러(camelCase). nullable 필드는
/// skip 하지 않고 null로 직렬화한다(TS는 `T | null`이지 optional이 아님).
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageWindow {
    pub kind: UsageWindowKind,
    /// weekly_model일 때 모델 표시명 등. 없으면 null.
    pub label: Option<String>,
    pub used_percent: f64,
    /// epoch ms로 정규화. 파싱 불가/부재 시 null.
    pub resets_at_ms: Option<i64>,
    pub window_minutes: Option<i64>,
    /// "지금 구속 중인 윈도"인지(Claude `limits[]`에만 있음). **유효성이
    /// 아니다** — 실측(`~/.claude.json`)상 weekly_all/weekly_scoped도 살아
    /// 있는 한도인데 is_active:false로 온다. 걸러내는 용도로 쓰지 말 것,
    /// 표시용 보조 정보로만 쓴다. Codex와 Claude five_hour/seven_day 폴백
    /// 경로는 항상 null.
    pub is_active: Option<bool>,
}

/// provider별 사용량. TS `ProviderUsage` 미러.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProviderUsage {
    pub provider: Provider,
    /// 신선도 기준 시각(epoch ms).
    pub fetched_at_ms: i64,
    /// codex plan_type, claude organizationRateLimitTier 등. 없으면 null.
    pub plan_label: Option<String>,
    /// 가변 배열 — UI가 "5시간+주간 둘 다"를 하드코딩하지 않는다.
    pub windows: Vec<UsageWindow>,
}

/// 전체 스냅샷. TS `UsageSnapshot` 미러. 실패한 소스는 null.
#[derive(Debug, Clone, PartialEq, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct UsageSnapshot {
    pub claude: Option<ProviderUsage>,
    pub codex: Option<ProviderUsage>,
    /// Antigravity 사용량. **파일 캐시 미러가 없다** — 실시간 조회가 한 번도
    /// 성공하지 않았으면 항상 None이다(Claude·Codex와 다른 점).
    pub antigravity: Option<ProviderUsage>,
    /// Gemini CLI 사용량. Antigravity와 같은 성질(파일 캐시 없음)이고, 게다가
    /// Code Assist 라이선스가 없는 계정에서는 영영 None이다(정상).
    pub gemini: Option<ProviderUsage>,
    /// Claude 실시간 조회 진단(항상 존재). 파일 캐시만 읽는 동기 경로에서는
    /// `NeverAttempted`가 그대로 나간다.
    pub claude_live: ClaudeLiveStatus,
    /// Codex 실시간 조회 진단(항상 존재). 같은 규칙 — 동기 경로에서는
    /// `NeverAttempted`.
    pub codex_live: CodexLiveStatus,
    /// Antigravity 실시간 조회 진단(항상 존재). 이쪽은 강등할 파일 캐시가
    /// 없어서 진단이 곧 "왜 안 보이는지"의 유일한 설명이다.
    pub antigravity_live: AntigravityLiveStatus,
    /// Gemini 실시간 조회 진단(항상 존재). 같은 이유로 유일한 설명이다.
    pub gemini_live: GeminiLiveStatus,
}

/// `claude_root`(홈, `.claude.json`이 이 아래)와 `codex_root`(`~/.codex`,
/// `sessions/`가 이 아래)에서 각 provider 스냅샷을 읽어 조립한다. 각 소스
/// 파싱은 독립적이며 실패 시 해당 필드가 None이 된다.
pub fn load_usage_snapshot(claude_root: &Path, codex_root: &Path) -> UsageSnapshot {
    UsageSnapshot {
        claude: claude::load(claude_root),
        codex: codex::load(codex_root),
        // Antigravity·Gemini는 읽을 파일 캐시가 없다 — 동기 경로에서는 항상
        // 비어 있고 값은 아래 실시간 경로에서만 채워진다.
        antigravity: None,
        gemini: None,
        claude_live: ClaudeLiveStatus::default(),
        codex_live: CodexLiveStatus::default(),
        antigravity_live: AntigravityLiveStatus::default(),
        gemini_live: GeminiLiveStatus::default(),
    }
}

/// 실시간 조회를 얹은 조립(이슈 #33, docs/claude-usage-live-fetch-design.md
/// §3.2 · Codex는 kbm #2h8, docs/usage-design.md §8). 커맨드가 이것에
/// 위임한다. 동기 `load_usage_snapshot`(파일 캐시 미러)은 그대로 두고 그
/// 결과의 claude·codex 필드를 각각 실시간 값으로 보강한다. 두 provider의
/// 조회는 서로 독립이라 동시에 돌리고(join), 한쪽 실패가 다른 쪽을 막지
/// 않는다.
///
/// 흐름:
/// 1. 파일 캐시 스냅샷을 먼저 읽는다(항상 성공, 실패 소스는 None).
/// 2. 스로틀(§3.1)을 통과하면 토큰을 읽어 실시간 fetch를 시도한다. Mutex는
///    판단·기록의 짧은 임계구역에서만 잡고 fetch await는 락 밖에서 한다.
/// 3. claude 필드를 파일 캐시와 메모리 live 중 더 신선한 쪽으로 확정하되,
///    plan_label은 응답에 없으므로 파일 캐시 값을 접목한다.
/// 4. 시도 결과(성공/실패 사유)를 `claude_live`에 실어 보낸다 — 실패는 여전히
///    조용한 폴백이지만 "왜 값이 안 움직이는지"는 UI가 말할 수 있어야 한다
///    (docs/usage-design.md §7). 스로틀에 막혀 이번 폴링이 시도조차 안 했으면
///    직전 시도의 결과가 그대로 유지돼 나간다.
///
/// `claude_config_dir`은 자격증명(.credentials.json)·스코프 Keychain의 기준
/// 디렉터리로, `.claude.json`을 읽는 `claude_root`와 다를 수 있다
/// (CLAUDE_CONFIG_DIR 미설정 시 claude_root=홈, config_dir=~/.claude).
pub async fn load_usage_snapshot_with_live(
    live: &LiveUsageState,
    home: &Path,
    claude_root: &Path,
    claude_config_dir: &Path,
    codex_root: &Path,
    now_ms: i64,
) -> UsageSnapshot {
    let mut snapshot = load_usage_snapshot(claude_root, codex_root);

    // 두 provider의 실시간 조회는 자원도 실패 모드도 겹치지 않는다 —
    // 동시에 돌려 폴링 1회의 지연이 둘의 합이 되지 않게 한다.
    let gemini_env = gemini_live::GeminiEnv::from_process_env(home);
    tokio::join!(
        refresh_claude_live(live, claude_config_dir, now_ms),
        refresh_codex_live(&live.codex, now_ms),
        refresh_antigravity_live(&live.antigravity, now_ms),
        refresh_gemini_live(&live.gemini, &gemini_env, now_ms),
    );

    snapshot.claude = merge_provider(snapshot.claude.take(), live.last_success());
    snapshot.claude_live = live.status();
    snapshot.codex = merge_provider(snapshot.codex.take(), live.codex.last_success());
    snapshot.codex_live = live.codex.status();
    // 파일 쪽이 항상 None이라 merge는 형식적이지만, 세 provider가 같은 규칙을
    // 쓰는 편이 나중에 캐시 소스가 생겼을 때 손댈 곳이 줄어든다.
    snapshot.antigravity = merge_provider(
        snapshot.antigravity.take(),
        live.antigravity.last_success(),
    );
    snapshot.antigravity_live = live.antigravity.status();
    snapshot.gemini = merge_provider(snapshot.gemini.take(), live.gemini.last_success());
    snapshot.gemini_live = live.gemini.status();
    snapshot
}

/// Claude 실시간 조회 1회분(스로틀 통과 시에만 실제로 돈다).
async fn refresh_claude_live(live: &LiveUsageState, claude_config_dir: &Path, now_ms: i64) {
    // 락 안에서 스로틀 판단 + last_attempt 갱신(중복 fetch 차단) → 락 해제 후 fetch.
    if live.begin_attempt_if_due(now_ms) {
        match claude_live::read_access_token(claude_config_dir).await {
            Some((token, source)) => match claude_live::fetch_live(&token).await {
                Ok(windows) => live.record_success(
                    claude_live::live_provider_usage(windows, now_ms),
                    Some(source),
                    FetchTransport::Direct,
                ),
                Err(failure) => {
                    run_fallback_chain(live, Some((&token, source)), failure, now_ms).await
                }
            },
            // 토큰을 못 읽어도 폴백은 의미가 있다 — `claude` 갈래는 CLI 자신의
            // 자격증명을 쓰므로 우리 Keychain 접근이 막힌 것과 무관하게 돈다.
            None => {
                run_fallback_chain(live, None, claude_live::LiveFailure::no_credentials(), now_ms)
                    .await
            }
        }
    }

}

/// Codex 실시간 조회 1회분. 실패는 조용히 기록만 하고 끝난다 — 표시값은
/// rollout 스냅샷(codex::load) 또는 직전 성공 값이 그대로 쓰인다.
async fn refresh_codex_live(live: &codex_live::CodexLiveState, now_ms: i64) {
    if !live.begin_attempt_if_due(now_ms) {
        return;
    }
    match codex_live::fetch_live(now_ms).await {
        Ok(usage) => live.record_success(usage),
        Err(failure) => live.record_failure(failure),
    }
}

/// Antigravity 실시간 조회 1회분. 실패하면 직전 성공 값이 그대로 쓰이고,
/// 그마저 없으면 이 provider는 스냅샷에서 None이다 — 강등할 파일 캐시가 없다.
async fn refresh_antigravity_live(live: &antigravity_live::AntigravityLiveState, now_ms: i64) {
    if !live.begin_attempt_if_due(now_ms) {
        return;
    }
    match antigravity_live::fetch_live(now_ms).await {
        Ok(usage) => live.record_success(usage),
        Err(failure) => live.record_failure(failure),
    }
}

/// Gemini 실시간 조회 1회분. Antigravity와 같은 성질(파일 캐시 없음)이라
/// 실패하면 직전 성공 값만 남고, 라이선스가 없는 계정에서는 영영 값이 없다.
async fn refresh_gemini_live(
    live: &gemini_live::GeminiLiveState,
    env: &gemini_live::GeminiEnv,
    now_ms: i64,
) {
    if !live.begin_attempt_if_due(now_ms) {
        return;
    }
    // 계정 정보(projectId·플랜명) 캐시가 있으면 loadCodeAssist 왕복을 건너뛴다.
    match gemini_live::fetch_live(env, live.cached_project().as_ref(), now_ms).await {
        Ok((usage, project)) => live.record_success(usage, project),
        Err(failure) => live.record_failure(failure),
    }
}

/// 1차(앱 내 reqwest) 조회가 실패했을 때의 우회 체인. curl → claude CLI 순으로
/// 시도하고, 하나라도 성공하면 그 값과 수단을 기록한다. 설계 의도는
/// claude_live_fallback 헤더 주석에 있다.
///
/// 스로틀: 폴백은 1시간에 한 번만 시도한다. 막히면 1차 실패 사유를 그대로
/// 기록하고 끝낸다 — 60초 폴링마다 자식 프로세스를 띄우는 일은 없다.
///
/// 실패로 끝나면 **1차 실패의 outcome을 유지**한다. 사용자가 고쳐야 할 대상은
/// "앱이 왜 직접 못 가져오는가"이고, 폴백 갈래별 결과는 detail에 접두를 달아
/// 함께 실어 보낸다("연결 실패 · curl: HTTP 401 · claude: 사용량 미제공").
async fn run_fallback_chain(
    live: &LiveUsageState,
    credentials: Option<(&str, TokenSource)>,
    primary: claude_live::LiveFailure,
    now_ms: i64,
) {
    let token_source = credentials.map(|(_, source)| source);
    if !live.begin_fallback_if_due(now_ms) {
        live.record_failure(token_source, primary);
        return;
    }

    let mut notes: Vec<String> = primary.detail.clone().into_iter().collect();

    // 1) curl — 우리가 읽은 토큰이 있어야 한다(1차와 같은 요청을 그대로 재현).
    match credentials {
        Some((token, source)) => match claude_live_fallback::fetch_via_curl(token).await {
            Ok(windows) => {
                live.record_success(
                    claude_live::live_provider_usage(windows, now_ms),
                    Some(source),
                    FetchTransport::Curl,
                );
                return;
            }
            Err(failure) => notes.push(format!("curl: {}", detail_or(&failure))),
        },
        None => notes.push("curl: 토큰 없음".into()),
    }

    // 2) claude CLI — 과금이 감지돼 봉인된 경우가 아니면 시도한다.
    if let Some(reason) = live.cli_disabled_reason() {
        notes.push(format!("claude: 건너뜀({reason})"));
    } else {
        let probe = claude_live_fallback::fetch_via_claude_cli().await;
        if let Some(spend) = probe.token_spend {
            // 조회 성패와 무관하게 즉시 봉인한다. 이 경로는 공짜일 때만 존재 가치가 있다.
            live.disable_cli_fallback(spend.clone());
            notes.push(format!("claude: 비활성화({spend})"));
        }
        match probe.result {
            Ok(windows) => {
                live.record_success(
                    claude_live::live_provider_usage(windows, now_ms),
                    token_source,
                    FetchTransport::ClaudeCli,
                );
                return;
            }
            Err(failure) => notes.push(format!("claude: {}", detail_or(&failure))),
        }
    }

    live.record_failure(
        token_source,
        claude_live::LiveFailure {
            outcome: primary.outcome,
            detail: (!notes.is_empty()).then(|| notes.join(" · ")),
        },
    );
}

/// 실패의 사람이 읽을 사유. detail이 비어 있는 실패는 사실상 없지만, 진단
/// 문자열이 "claude: " 처럼 잘려 나가지 않도록 기본값을 둔다.
fn detail_or(failure: &claude_live::LiveFailure) -> String {
    failure.detail.clone().unwrap_or_else(|| "실패".into())
}

/// 파일 캐시와 실시간 결과 중 `fetched_at_ms`가 큰 쪽을 고른다(렌더러
/// fresherProvider와 같은 규칙 — Claude Code가 방금 캐시를 갱신했다면 그쪽이
/// 이길 수 있다). 동률·live 우선(이 기능의 취지). live가 이기면 plan_label을
/// 파일 캐시에서 접목한다(Claude live 응답엔 plan_label이 없고, Codex는
/// 있지만 없을 때를 대비해 같은 규칙을 쓴다). 설계 §3.2. 두 provider가
/// 같은 규칙을 쓰므로 이름은 provider 중립이다.
fn merge_provider(
    file: Option<ProviderUsage>,
    live: Option<ProviderUsage>,
) -> Option<ProviderUsage> {
    match (file, live) {
        (None, None) => None,
        (Some(f), None) => Some(f),
        (None, Some(l)) => Some(l),
        (Some(f), Some(mut l)) => {
            if l.fetched_at_ms >= f.fetched_at_ms {
                if l.plan_label.is_none() {
                    l.plan_label = f.plan_label;
                }
                Some(l)
            } else {
                Some(f)
            }
        }
    }
}

/// timezone 포함 ISO8601/RFC3339 문자열을 epoch ms로. Claude의
/// `2026-07-17T09:50:00.243466+00:00`(소수 초 + 오프셋)와 Codex의
/// `2026-07-17T11:20:17.595Z`(Z 접미)를 모두 처리한다. 실패 시 None.
pub(super) fn parse_iso8601_ms(s: &str) -> Option<i64> {
    chrono::DateTime::parse_from_rfc3339(s)
        .ok()
        .map(|dt| dt.timestamp_millis())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_claude_style_offset_timestamp() {
        // 2026-07-17T09:50:00Z == 1_784_281_800_000 ms.
        let ms = parse_iso8601_ms("2026-07-17T09:50:00.000000+00:00").unwrap();
        assert_eq!(ms, 1_784_281_800_000);
    }

    #[test]
    fn parses_codex_style_zulu_timestamp() {
        let ms = parse_iso8601_ms("2026-07-17T09:50:00.000Z").unwrap();
        assert_eq!(ms, 1_784_281_800_000);
    }

    #[test]
    fn rejects_garbage_timestamp() {
        assert_eq!(parse_iso8601_ms("not a date"), None);
    }

    #[test]
    fn missing_sources_yield_null_provider_but_snapshot_succeeds() {
        let root = std::env::temp_dir().join(format!("usage-empty-{}", uuid::Uuid::new_v4()));
        let snap = load_usage_snapshot(&root, &root);
        assert!(snap.claude.is_none());
        assert!(snap.codex.is_none());
    }

    // ── merge_claude (실시간/파일 캐시 조립) ──

    fn provider(fetched_at_ms: i64, plan_label: Option<&str>) -> ProviderUsage {
        ProviderUsage {
            provider: Provider::Claude,
            fetched_at_ms,
            plan_label: plan_label.map(str::to_string),
            windows: vec![UsageWindow {
                kind: UsageWindowKind::Session,
                label: None,
                used_percent: 42.0,
                resets_at_ms: None,
                window_minutes: None,
                is_active: None,
            }],
        }
    }

    #[test]
    fn merge_provider_live_wins_when_fresher_and_grafts_plan_label() {
        // live는 plan_label이 없다(응답에 없음). 파일 캐시에서 접목해야 한다.
        let file = provider(1_000, Some("max_20x"));
        let live = provider(2_000, None);
        let merged = merge_provider(Some(file), Some(live)).unwrap();
        assert_eq!(merged.fetched_at_ms, 2_000, "더 신선한 live가 이겨야");
        assert_eq!(merged.plan_label.as_deref(), Some("max_20x"), "plan_label 접목");
    }

    #[test]
    fn merge_provider_file_wins_when_it_is_fresher() {
        // Claude Code가 방금 캐시를 갱신한 경우 파일이 이길 수 있어야 한다.
        let file = provider(5_000, Some("max_20x"));
        let live = provider(2_000, None);
        let merged = merge_provider(Some(file), Some(live)).unwrap();
        assert_eq!(merged.fetched_at_ms, 5_000);
        assert_eq!(merged.plan_label.as_deref(), Some("max_20x"));
    }

    #[test]
    fn merge_provider_falls_back_to_file_when_no_live() {
        let file = provider(1_000, Some("max_20x"));
        let merged = merge_provider(Some(file), None).unwrap();
        assert_eq!(merged.fetched_at_ms, 1_000);
    }

    #[test]
    fn merge_provider_uses_live_when_no_file() {
        let live = provider(2_000, None);
        let merged = merge_provider(None, Some(live)).unwrap();
        assert_eq!(merged.fetched_at_ms, 2_000);
        assert_eq!(merged.plan_label, None);
    }

    #[test]
    fn merge_provider_none_when_both_absent() {
        assert_eq!(merge_provider(None, None), None);
    }
}
