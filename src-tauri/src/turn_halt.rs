// src-tauri/src/turn_halt.rs
//
// Stop 훅 없이 끝난 Claude 턴을 찾아 턴을 정산한다.
//
// Claude Code는 사용자가 Esc로 끊거나(권한 대화상자에서 거절 포함) API 오류로 턴이 멈추면 Stop 훅을
// 보내지 않는다. 그러면 턴 관측이 "작업 중"에 머물러 대화 보기·웹 상태판이 "멈췄을 수 있음"을 띄우고,
// 보낼 메시지 대기열(`chat_queue.rs`)은 턴 끝을 기다리며 영영 넣지 않는다. 그런 턴은 전사 끝에
// 흔적이 남는다 — 본 대화의 마지막 항목이 `[Request interrupted by user…]` 사람 말이거나 API 오류
// assistant 항목(`isApiErrorMessage`)이다. 턴이 도는 중인데 PTY 출력이 잠잠하면(작업 중인 Claude는
// 스피너·경과 시간을 늘 다시 그린다) 전사 끝을 읽어 이 흔적이면 `NotificationHub::turn_halted`로
// 알림 없이 턴을 정산한다(`ActivityKind::Idle`).
use std::collections::HashMap;
use std::io::{Read, Seek, SeekFrom};
use std::sync::Arc;
use std::time::Duration;

use crate::control::turns::TurnPhase;
use crate::control::ControlContext;

const POLL: Duration = Duration::from_secs(2);
/// 마지막 PTY 출력·턴 신호 뒤 이만큼 잠잠해야 본다.
const QUIET_MS: u64 = 3_000;
/// 전사 끝에서 읽을 바이트.
const TAIL_BYTES: u64 = 256 * 1024;

const INTERRUPT_MARK: &str = "[Request interrupted by user";

/// 전사 끝(JSONL 꼬리)으로 본 턴의 끝. 본 대화(사이드체인 아님)의 마지막 사람·assistant 항목이
/// 끊김 표시거나 API 오류면 true. 마지막 줄이 아직 쓰이는 중이거나 판단할 항목이 없으면 false.
pub fn claude_turn_halted(tail: &str) -> bool {
    // 줄바꿈으로 끝나지 않으면 마지막 항목을 쓰는 중이다.
    if !tail.is_empty() && !tail.ends_with('\n') {
        return false;
    }
    for line in tail.lines().rev() {
        let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else { continue };
        if v.get("isSidechain").and_then(|b| b.as_bool()) == Some(true) {
            continue;
        }
        match v.get("type").and_then(|t| t.as_str()) {
            Some("assistant") => return v.get("isApiErrorMessage").and_then(|b| b.as_bool()) == Some(true),
            Some("user") => return user_interrupt(&v),
            _ => continue,
        }
    }
    false
}

fn user_interrupt(v: &serde_json::Value) -> bool {
    let content = v.get("message").and_then(|m| m.get("content"));
    match content {
        Some(serde_json::Value::String(s)) => s.trim_start().starts_with(INTERRUPT_MARK),
        Some(serde_json::Value::Array(parts)) => parts.iter().any(|p| {
            p.get("type").and_then(|t| t.as_str()) == Some("text")
                && p.get("text").and_then(|t| t.as_str()).is_some_and(|t| t.trim_start().starts_with(INTERRUPT_MARK))
        }),
        _ => false,
    }
}

/// 파일 끝 `TAIL_BYTES`(첫 줄은 잘렸을 수 있어 버린다)와 파일 길이.
fn read_tail(path: &str) -> Option<(String, u64)> {
    let mut f = std::fs::File::open(path).ok()?;
    let len = f.metadata().ok()?.len();
    let start = len.saturating_sub(TAIL_BYTES);
    f.seek(SeekFrom::Start(start)).ok()?;
    let mut buf = Vec::with_capacity((len - start) as usize);
    f.read_to_end(&mut buf).ok()?;
    let mut text = String::from_utf8_lossy(&buf).into_owned();
    if start > 0 {
        let cut = text.find('\n').map_or(text.len(), |i| i + 1);
        text.drain(..cut);
    }
    Some((text, len))
}

/// 세션 → 이미 본 (턴 신호 시각, 전사 길이). 그대로면 다시 읽지 않는다.
type Seen = HashMap<String, (u64, u64)>;

async fn check_once(ctx: &ControlContext, seen: &mut Seen) {
    let now = crate::types::now_ms();
    let turns = ctx.turns.in_turn();
    seen.retain(|sid, _| turns.iter().any(|(s, _)| s == sid));
    for (sid, turn) in turns {
        let agent = turn.agent_id.clone();
        let live = ctx.manager.session_id_for(&agent).filter(|_| ctx.manager.is_running(&agent));
        if live.as_deref() != Some(sid.as_str()) || now.saturating_sub(turn.at) < QUIET_MS {
            continue;
        }
        if ctx.manager.last_output_ms(&agent).is_some_and(|out| now.saturating_sub(out) < QUIET_MS) {
            continue;
        }
        let Some(path) = ctx.agent_session_lookup.latest_session(&agent).and_then(|s| s.transcript_path) else {
            continue;
        };
        let read = tokio::task::spawn_blocking(move || read_tail(&path)).await.ok().flatten();
        let Some((tail, len)) = read else { continue };
        if seen.insert(sid.clone(), (turn.at, len)) == Some((turn.at, len)) || !claude_turn_halted(&tail) {
            continue;
        }
        // 이 캐릭터가 예전에 Claude를 썼던 전사일 수 있다. 지금 포그라운드가 Claude일 때만 믿는다.
        let (manager, a) = (ctx.manager.clone(), agent.clone());
        let claude = tokio::task::spawn_blocking(move || {
            crate::webdash::foreground_cli(&manager, &a, manager.foreground_command(&a).as_deref()) == Some("claude")
        })
        .await
        .unwrap_or(false);
        if !claude {
            continue;
        }
        // 읽는 사이 새 신호(새 프롬프트 등)가 왔으면 그 턴을 덮지 않는다.
        let still = ctx.turns.get(&sid).is_some_and(|t| {
            t.at == turn.at && matches!(t.phase, TurnPhase::Working | TurnPhase::NeedsHuman)
        });
        if !still {
            continue;
        }
        ctx.observer.permissions().settle_session(&sid);
        ctx.hub.turn_halted(&sid);
    }
}

pub fn spawn(ctx: Arc<ControlContext>) {
    tauri::async_runtime::spawn(async move {
        let mut seen = Seen::new();
        loop {
            tokio::time::sleep(POLL).await;
            check_once(&ctx, &mut seen).await;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(items: &[&str]) -> String {
        items.iter().map(|l| format!("{l}\n")).collect()
    }

    #[test]
    fn interrupt_and_api_error_end_the_turn() {
        let interrupted = lines(&[
            r#"{"type":"assistant","isSidechain":false,"message":{"content":[{"type":"text","text":"합니다."}]}}"#,
            r#"{"type":"user","isSidechain":false,"message":{"content":[{"type":"text","text":"[Request interrupted by user]"}]}}"#,
            r#"{"type":"last-prompt"}"#,
            r#"{"type":"ai-title"}"#,
        ]);
        assert!(claude_turn_halted(&interrupted));
        let tool_denied = lines(&[
            r#"{"type":"user","message":{"content":"[Request interrupted by user for tool use]"}}"#,
        ]);
        assert!(claude_turn_halted(&tool_denied));
        let api_error = lines(&[r#"{"type":"assistant","isApiErrorMessage":true,"message":{"content":[]}}"#]);
        assert!(claude_turn_halted(&api_error));
    }

    #[test]
    fn a_live_or_finished_turn_is_not_halted() {
        // 끊긴 뒤 새 프롬프트를 냈다.
        let resumed = lines(&[
            r#"{"type":"user","message":{"content":[{"type":"text","text":"[Request interrupted by user]"}]}}"#,
            r#"{"type":"user","message":{"content":"다시 해 줘"}}"#,
        ]);
        assert!(!claude_turn_halted(&resumed));
        // 도구 호출 중(권한 대화상자일 수 있다).
        let tool = lines(&[r#"{"type":"assistant","message":{"content":[{"type":"tool_use","name":"Bash"}]}}"#]);
        assert!(!claude_turn_halted(&tool));
        // 서브에이전트의 끊김은 본 대화가 아니다.
        let side = lines(&[
            r#"{"type":"assistant","message":{"content":[{"type":"tool_use","name":"Agent"}]}}"#,
            r#"{"type":"user","isSidechain":true,"message":{"content":"[Request interrupted by user]"}}"#,
        ]);
        assert!(!claude_turn_halted(&side));
        // 마지막 줄을 아직 쓰는 중.
        let partial = r#"{"type":"user","message":{"content":"[Request interrupted by user]"}}"#;
        assert!(!claude_turn_halted(partial));
        assert!(!claude_turn_halted(""));
    }

    #[test]
    fn tail_drops_the_cut_first_line() {
        let dir = std::env::temp_dir().join(format!("turn-halt-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("t.jsonl");
        let big = format!("{{\"type\":\"user\",\"message\":{{\"content\":\"{}\"}}}}\n", "x".repeat(TAIL_BYTES as usize));
        let last = r#"{"type":"user","message":{"content":"[Request interrupted by user]"}}"#;
        std::fs::write(&path, format!("{big}{last}\n")).unwrap();
        let (tail, len) = read_tail(path.to_str().unwrap()).unwrap();
        assert_eq!(tail, format!("{last}\n"));
        assert_eq!(len, (big.len() + last.len() + 1) as u64);
        std::fs::remove_dir_all(dir).ok();
    }
}
