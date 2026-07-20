// src-tauri/src/ipc/commands/usage.rs
//
// Subscription usage (rate limit) snapshot command plus its pure root-path
// resolvers (kept separate from the `std::env::var` reads so unit tests can
// exercise the path logic without touching real process env,
// docs/usage-limits-design.md §2).

use tauri::State;

use crate::state::AppState;

/// 사용량 소스 루트 결정의 순수 계산부. 전역 `std::env::var` 접근과
/// 분리해 두어야 단위 테스트에서 프로세스 전역 env에 손대지 않고 조합을
/// 검증할 수 있다(docs/usage-limits-design.md §2). 실제 CLI가 존중하는
/// 표준 오버라이드를 그대로 따른다: Codex는 `CODEX_HOME`(설정되면
/// `<CODEX_HOME>/sessions`를 읽음), Claude는 `CLAUDE_CONFIG_DIR`(설정되면
/// `<CLAUDE_CONFIG_DIR>/.claude.json`을 읽음 -- claude.rs::load의 파일명
/// 결합 로직은 그대로 두고 루트만 바꾼다). 빈 문자열 env는 미설정으로
/// 취급(일부 셸/런처가 unset 대신 빈 문자열을 넘기는 경우 대비).
pub(crate) fn resolve_usage_roots(
    home: &std::path::Path,
    codex_home_env: Option<&str>,
    claude_config_env: Option<&str>,
) -> (std::path::PathBuf, std::path::PathBuf) {
    let codex_root = codex_home_env
        .filter(|s| !s.is_empty())
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| home.join(".codex"));
    let claude_root = claude_config_env
        .filter(|s| !s.is_empty())
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| home.to_path_buf());
    (codex_root, claude_root)
}

/// Claude 자격증명(.credentials.json)·스코프 Keychain의 기준 디렉터리
/// (docs/claude-usage-live-fetch-design.md §2.2). `.claude.json`을 읽는
/// `claude_root`와 다르다: CLAUDE_CONFIG_DIR가 설정되면 그 경로, 미설정이면
/// `~/.claude`(claude_root=홈과 구분됨). 빈 문자열 env는 미설정 취급.
pub(crate) fn resolve_claude_config_dir(
    home: &std::path::Path,
    claude_config_env: Option<&str>,
) -> std::path::PathBuf {
    claude_config_env
        .filter(|s| !s.is_empty())
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| home.join(".claude"))
}

/// 구독 사용량(rate limit) 스냅샷을 읽는다(인자 없음,
/// docs/usage-limits-design.md). Claude(`.claude.json`)와 Codex
/// (`sessions/`)를 각각 독립 파싱하므로 한쪽 소스가 실패해도 커맨드는
/// 성공하고 실패한 provider만 `null`이 된다. 상태를 건드리지 않는 순수 읽기라
/// AppState가 필요 없다 -- 루트 경로만 `resolve_usage_roots`로 유도해 usage
/// 모듈에 넘긴다. 기본은 홈 디렉터리 하위(`~/.codex`, `~/.claude.json`)이고,
/// `CODEX_HOME`/`CLAUDE_CONFIG_DIR` 환경변수가 설정돼 있으면 그쪽을 우선한다.
/// 이슈 #33: 실시간 조회를 얹었다. 스로틀 상태(`AppState::live_usage`)를
/// 넘겨 렌더러 60초 폴링에 얹혀 리셋 경계 후 실제 값을 빠르게 반영한다.
/// 실시간 경로가 실패하면 현행과 동일하게 파일 캐시 미러만 반환한다.
#[tauri::command(rename_all = "camelCase")]
pub async fn load_usage_snapshot(
    app_state: State<'_, AppState>,
) -> Result<crate::usage::UsageSnapshot, String> {
    Ok(load_usage_snapshot_body(&app_state.live_usage, chrono::Utc::now().timestamp_millis()).await)
}

/// 커맨드 본체(AppState 없이 호출 가능 — 기존 커맨드 테스트 관례). 전역 env를
/// 여기서만 읽고 순수 리졸버로 경로를 유도한 뒤 usage 모듈에 위임한다.
pub(crate) async fn load_usage_snapshot_body(
    live: &crate::usage::LiveUsageState,
    now_ms: i64,
) -> crate::usage::UsageSnapshot {
    let home = std::path::PathBuf::from(crate::session::manager::home_dir());
    let codex_home_env = std::env::var("CODEX_HOME").ok();
    let claude_config_env = std::env::var("CLAUDE_CONFIG_DIR").ok();
    let (codex_root, claude_root) = resolve_usage_roots(
        &home,
        codex_home_env.as_deref(),
        claude_config_env.as_deref(),
    );
    let claude_config_dir = resolve_claude_config_dir(&home, claude_config_env.as_deref());
    crate::usage::load_usage_snapshot_with_live(
        live,
        &claude_root,
        &claude_config_dir,
        &codex_root,
        now_ms,
    )
    .await
}
