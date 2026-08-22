// src-tauri/src/ipc/commands/talk.rs
//
// 동료 대화(docs/agent-talk-design.md)의 렌더러용 커맨드 — 상태 스냅샷과
// 감사 로그 열람. 대화를 켜고 끄는 건 `set_app_settings`(talkEnabled)이고,
// 메시지를 실제로 주고받는 건 에이전트가 CLI로 하는 일이라 여기 없다.

use serde::Serialize;
use tauri::State;

use crate::state::AppState;

/// 하단바 표시용 스냅샷.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TalkStatus {
    pub enabled: bool,
    /// 아직 상대에게 닿지 않은 메시지 수(상대가 바쁘면 여기 쌓인다).
    pub queued: usize,
    /// 살아 있는 대화들.
    pub conversations: Vec<crate::talk::Conversation>,
}

#[tauri::command(rename_all = "camelCase")]
pub async fn talk_status(app_state: State<'_, AppState>) -> Result<TalkStatus, String> {
    Ok(TalkStatus {
        enabled: app_state.talk.is_enabled(),
        queued: app_state.talk.queued_len(),
        conversations: app_state.talk.open_conversations(),
    })
}

/// 감사 로그가 있는 날짜들(최신 순, `YYYY-MM-DD`).
#[tauri::command(rename_all = "camelCase")]
pub async fn list_talk_log_dates(app_state: State<'_, AppState>) -> Result<Vec<String>, String> {
    let dir = talk_log_dir(&app_state);
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return Ok(Vec::new());
    };
    let mut dates: Vec<String> = entries
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            name.strip_suffix(".jsonl").map(str::to_string)
        })
        .collect();
    dates.sort_unstable_by(|a, b| b.cmp(a));
    Ok(dates)
}

/// 하루치 감사 로그를 읽어 준다(최근 `limit`건, 기본 500). 줄 단위 JSON이라
/// 파싱 실패한 줄은 조용히 버린다 — 로그는 append-only라 꼬리가 잘릴 수 있다.
#[tauri::command(rename_all = "camelCase")]
pub async fn read_talk_log(
    date: String,
    limit: Option<usize>,
    app_state: State<'_, AppState>,
) -> Result<Vec<serde_json::Value>, String> {
    // 경로 조작 차단: 날짜 형식만 받는다.
    if !is_date(&date) {
        return Err("날짜 형식이 올바르지 않습니다(YYYY-MM-DD)".into());
    }
    let path = talk_log_dir(&app_state).join(format!("{date}.jsonl"));
    let Ok(text) = std::fs::read_to_string(&path) else {
        return Ok(Vec::new());
    };
    let limit = limit.unwrap_or(500);
    let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
    let start = lines.len().saturating_sub(limit);
    Ok(lines[start..]
        .iter()
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect())
}

/// 감사 로그 루트. app_data 경로는 control 컨텍스트가 이미 쥐고 있다.
fn talk_log_dir(app_state: &AppState) -> std::path::PathBuf {
    app_state.control_ctx.app_data_dir.join("talks")
}

fn is_date(s: &str) -> bool {
    s.len() == 10
        && s.chars()
            .enumerate()
            .all(|(i, c)| if i == 4 || i == 7 { c == '-' } else { c.is_ascii_digit() })
}

#[cfg(test)]
mod tests {
    use super::is_date;

    #[test]
    fn only_plain_dates_are_accepted() {
        assert!(is_date("2026-08-22"));
        assert!(!is_date("../../etc/passwd"));
        assert!(!is_date("2026-8-2"));
    }
}
