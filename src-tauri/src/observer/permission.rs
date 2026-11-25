// src-tauri/src/observer/permission.rs
//
// 권한 요청·선택지 중계. Claude Code의 `PermissionRequest` 훅은 권한 대화상자가 뜰 때
// 불리고, 훅이 결정(허용·거부, AskUserQuestion이면 답)을 돌려주면 Claude가 그대로 따른다.
// forwarder가 이 훅 body를 옵저버 서버 `/permission`에 보내고 답이 올 때까지 기다린다.
// 웹 상태판과 앱 터미널 탭의 대화 보기가 그 사이 이 대기열을 보여 주고 답을 넣는다.
// 중계는 앱이 도는 동안 늘 켜져 있다(`lib.rs`) — 꺼 두면 훅이 바로 끝난다(시험·앱 종료 직전).
//
// **터미널과 다투지 않는다**: 훅이 기다리는 동안에도 TUI에는 평소 대화상자가 함께 뜨고,
// 먼저 답한 쪽이 이긴다(2.1.332 실측 — 터미널에서 먼저 고르면 늦게 온 훅의 답은 버려진다).
// 그래서 웹 응답은 "추가 입력 경로"일 뿐 모드 전환이 없다. 키 입력을 흉내 내지 않는다.
//
// 대기 항목이 사라지는 경우:
// - 웹에서 답함 → 결정 JSON을 forwarder에 돌려준다.
// - 웹에서 닫음·중계 꺼짐·대기 상한 → 빈 답(결정 없음). 터미널 대화상자는 그대로다.
// - 터미널에서 먼저 답함 → 그 뒤 오는 훅(같은 도구의 PostToolUse, 턴 경계)으로 정리한다.
// - forwarder가 죽음(CLI 종료) → 연결이 끊겨 서버 쪽 대기가 풀리며 지운다.
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;
use serde_json::{json, Map, Value};
use tokio::sync::oneshot;

/// 세션 하나가 동시에 들고 있을 수 있는 대기 수(병렬 도구 호출이라도 이 정도면 넉넉하다).
const MAX_PENDING_PER_SESSION: usize = 16;
/// 웹으로 보내는 도구 입력의 문자열 하나 상한(Write 본문 같은 것).
const MAX_INPUT_STRING: usize = 6000;
/// 웹으로 보내는 배열 하나의 항목 상한.
const MAX_INPUT_ITEMS: usize = 40;
/// 답 하나(선택지 라벨·직접 입력)의 문자 상한.
const MAX_ANSWER_CHARS: usize = 4000;
/// 거부 사유의 문자 상한.
const MAX_MESSAGE_CHARS: usize = 2000;
/// 웹에서 거부할 때 사유를 안 쓰면 에이전트에게 이렇게 알린다.
const DEFAULT_DENY_MESSAGE: &str = "The user denied this request from the Agent Office web dashboard.";

/// 웹에 보이는 대기 항목.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingAsk {
    pub id: String,
    pub tool_name: String,
    /// 도구 입력(긴 문자열은 잘라서).
    pub tool_input: Value,
    /// "다시 묻지 않기"(Claude가 제안한 권한 규칙)를 고를 수 있는가.
    pub can_always: bool,
    /// 서브에이전트가 낸 요청인가.
    pub sidechain: bool,
    /// 받은 시각(유닉스 ms).
    pub at: u64,
}

/// 웹에서 고른 답.
#[derive(Debug, Clone, PartialEq)]
pub enum AskDecision {
    /// 허용. `answers`는 AskUserQuestion의 질문 → 답, `always`는 제안된 권한 규칙까지 적용.
    Allow { answers: Option<HashMap<String, String>>, always: bool },
    Deny { message: Option<String> },
    /// 결정하지 않고 웹에서만 내린다(터미널 대화상자는 그대로).
    Dismiss,
}

impl AskDecision {
    /// 화면의 단추(`allow`·`always`·`deny`·`dismiss`) → 결정. 웹 상태판과 앱 대화 보기가 같이 쓴다.
    pub fn from_action(
        action: &str,
        answers: Option<HashMap<String, String>>,
        message: Option<String>,
    ) -> Result<Self, String> {
        Ok(match action {
            "allow" => Self::Allow { answers, always: false },
            "always" => Self::Allow { answers: None, always: true },
            "deny" => Self::Deny { message },
            "dismiss" => Self::Dismiss,
            _ => return Err("bad-request".into()),
        })
    }
}

struct Entry {
    session_id: String,
    view: PendingAsk,
    input: Value,
    suggestions: Option<Value>,
    reply: oneshot::Sender<String>,
}

#[derive(Default)]
pub struct PermissionRelay {
    enabled: AtomicBool,
    next: AtomicU64,
    entries: Mutex<HashMap<String, Entry>>,
}

fn now_ms() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

impl PermissionRelay {
    pub fn new() -> Self {
        Self::default()
    }

    /// 켜고 끈다. 끄면 기다리던 것은 모두 "결정 없음"으로 푼다.
    pub fn set_enabled(&self, on: bool) {
        self.enabled.store(on, Ordering::SeqCst);
        if !on {
            self.entries.lock().unwrap().clear();
        }
    }

    pub fn is_enabled(&self) -> bool {
        self.enabled.load(Ordering::SeqCst)
    }

    /// 훅 body를 대기열에 넣는다. 꺼져 있거나 body를 못 읽으면 None(forwarder는 바로 끝난다).
    pub fn open(&self, session_id: &str, body: &[u8]) -> Option<(String, oneshot::Receiver<String>)> {
        if !self.is_enabled() {
            return None;
        }
        let body: Value = serde_json::from_slice(body).ok()?;
        let tool_name = body.get("tool_name")?.as_str()?.to_string();
        let input = body.get("tool_input").cloned().unwrap_or(Value::Object(Map::new()));
        let suggestions = body
            .get("permission_suggestions")
            .filter(|s| s.as_array().is_some_and(|a| !a.is_empty()))
            .cloned();
        let sidechain = body.get("agent_id").and_then(Value::as_str).is_some_and(|s| !s.is_empty());
        let mut entries = self.entries.lock().unwrap();
        if entries.values().filter(|e| e.session_id == session_id).count() >= MAX_PENDING_PER_SESSION {
            return None;
        }
        let id = format!("ask-{}", self.next.fetch_add(1, Ordering::SeqCst) + 1);
        let (tx, rx) = oneshot::channel();
        let view = PendingAsk {
            id: id.clone(),
            tool_name,
            tool_input: clip_input(&input),
            // AskUserQuestion은 규칙이 없다(물을 때마다 답해야 한다).
            can_always: suggestions.is_some(),
            sidechain,
            at: now_ms(),
        };
        entries.insert(id.clone(), Entry { session_id: session_id.to_string(), view, input, suggestions, reply: tx });
        Some((id, rx))
    }

    /// forwarder 연결이 끝났다(답했든 끊겼든). 남아 있으면 지운다.
    pub fn withdraw(&self, id: &str) {
        self.entries.lock().unwrap().remove(id);
    }

    /// 한 세션의 대기 항목(받은 순서).
    pub fn list(&self, session_id: &str) -> Vec<PendingAsk> {
        let entries = self.entries.lock().unwrap();
        let mut out: Vec<PendingAsk> = entries
            .values()
            .filter(|e| e.session_id == session_id && !e.reply.is_closed())
            .map(|e| e.view.clone())
            .collect();
        out.sort_by_key(|a| (a.at, a.id.clone()));
        out
    }

    /// 웹의 답을 넣는다. 이미 없어진 항목이면 `ask-gone`.
    pub fn answer(&self, id: &str, decision: AskDecision) -> Result<(), String> {
        let mut entries = self.entries.lock().unwrap();
        let entry = entries.get(id).ok_or_else(|| "ask-gone".to_string())?;
        let reply = match decision {
            AskDecision::Dismiss => None,
            AskDecision::Deny { message } => {
                let message = message
                    .map(|m| m.trim().to_string())
                    .filter(|m| !m.is_empty())
                    .unwrap_or_else(|| DEFAULT_DENY_MESSAGE.to_string());
                if message.chars().count() > MAX_MESSAGE_CHARS {
                    return Err("too-large".into());
                }
                Some(json!({ "behavior": "deny", "message": message }))
            }
            AskDecision::Allow { answers, always } => {
                let mut decision = json!({ "behavior": "allow" });
                if entry.view.tool_name == "AskUserQuestion" {
                    let answers = validate_answers(&entry.input, answers.unwrap_or_default())?;
                    let mut updated = entry.input.clone();
                    if let Some(map) = updated.as_object_mut() {
                        map.insert("answers".into(), json!(answers));
                    }
                    decision["updatedInput"] = updated;
                } else if always {
                    let Some(suggestions) = &entry.suggestions else {
                        return Err("ask-no-rule".into());
                    };
                    decision["updatedPermissions"] = suggestions.clone();
                }
                Some(decision)
            }
        };
        let entry = entries.remove(id).expect("checked above");
        if let Some(decision) = reply {
            let out = json!({
                "hookSpecificOutput": { "hookEventName": "PermissionRequest", "decision": decision }
            });
            // forwarder가 이미 끊겼으면 보낼 곳이 없다 — 터미널에서 답한 뒤라는 뜻이다.
            entry.reply.send(out.to_string()).map_err(|_| "ask-gone".to_string())?;
        }
        Ok(())
    }

    /// 세션의 대기를 모두 푼다(턴이 끝나거나 새 프롬프트 — 터미널에서 이미 답했다).
    pub fn settle_session(&self, session_id: &str) {
        self.entries.lock().unwrap().retain(|_, e| e.session_id != session_id);
    }

    /// 같은 도구 호출이 끝났다(터미널에서 허용했다). 입력이 같은 대기를 푼다.
    pub fn settle_tool(&self, session_id: &str, tool_name: &str, tool_input: &Value) {
        let key = without_answers(tool_input);
        self.entries.lock().unwrap().retain(|_, e| {
            !(e.session_id == session_id && e.view.tool_name == tool_name && without_answers(&e.input) == key)
        });
    }
}

fn without_answers(input: &Value) -> Value {
    let mut v = input.clone();
    if let Some(map) = v.as_object_mut() {
        map.remove("answers");
    }
    v
}

/// AskUserQuestion 답 검사: 질문마다 비지 않은 답이 하나씩, 질문에 없는 키는 받지 않는다.
fn validate_answers(input: &Value, answers: HashMap<String, String>) -> Result<Map<String, Value>, String> {
    let questions: Vec<&str> = input
        .get("questions")
        .and_then(Value::as_array)
        .map(|qs| qs.iter().filter_map(|q| q.get("question").and_then(Value::as_str)).collect())
        .unwrap_or_default();
    if questions.is_empty() {
        return Err("ask-invalid".into());
    }
    if answers.keys().any(|k| !questions.contains(&k.as_str())) {
        return Err("ask-invalid".into());
    }
    let mut out = Map::new();
    for q in questions {
        let a = answers.get(q).map(|a| a.trim()).unwrap_or("");
        if a.is_empty() {
            return Err("ask-unanswered".into());
        }
        if a.chars().count() > MAX_ANSWER_CHARS {
            return Err("too-large".into());
        }
        out.insert(q.to_string(), Value::String(a.to_string()));
    }
    Ok(out)
}

/// 웹에 보낼 도구 입력. 긴 문자열·배열은 자른다(원본은 답할 때 그대로 쓴다).
fn clip_input(v: &Value) -> Value {
    match v {
        Value::String(s) if s.chars().count() > MAX_INPUT_STRING => {
            let mut cut: String = s.chars().take(MAX_INPUT_STRING).collect();
            cut.push_str("\n…");
            Value::String(cut)
        }
        Value::Array(items) => Value::Array(items.iter().take(MAX_INPUT_ITEMS).map(clip_input).collect()),
        Value::Object(map) => Value::Object(map.iter().map(|(k, v)| (k.clone(), clip_input(v))).collect()),
        other => other.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ask_body() -> Vec<u8> {
        json!({
            "hook_event_name": "PermissionRequest",
            "tool_name": "AskUserQuestion",
            "tool_input": { "questions": [
                { "question": "Color?", "header": "C", "multiSelect": false, "options": [{ "label": "Red" }, { "label": "Blue" }] }
            ]}
        })
        .to_string()
        .into_bytes()
    }

    fn bash_body(cmd: &str) -> Vec<u8> {
        json!({
            "tool_name": "Bash",
            "tool_input": { "command": cmd },
            "permission_suggestions": [{ "type": "addRules", "rules": [{ "toolName": "Bash", "ruleContent": cmd }], "behavior": "allow", "destination": "localSettings" }]
        })
        .to_string()
        .into_bytes()
    }

    fn relay() -> PermissionRelay {
        let r = PermissionRelay::new();
        r.set_enabled(true);
        r
    }

    #[test]
    fn disabled_relay_lets_the_hook_return_at_once() {
        let r = PermissionRelay::new();
        assert!(r.open("s", &ask_body()).is_none());
    }

    #[test]
    fn answering_a_question_returns_updated_input_with_answers() {
        let r = relay();
        let (id, mut rx) = r.open("s", &ask_body()).unwrap();
        assert_eq!(r.list("s").len(), 1);
        assert_eq!(r.answer(&id, AskDecision::Allow { answers: Some(HashMap::new()), always: false }), Err("ask-unanswered".into()));
        let bad = HashMap::from([("Other?".to_string(), "x".to_string())]);
        assert_eq!(r.answer(&id, AskDecision::Allow { answers: Some(bad), always: false }), Err("ask-invalid".into()));
        let ok = HashMap::from([("Color?".to_string(), "Blue".to_string())]);
        r.answer(&id, AskDecision::Allow { answers: Some(ok), always: false }).unwrap();
        let out: Value = serde_json::from_str(&rx.try_recv().unwrap()).unwrap();
        let d = &out["hookSpecificOutput"]["decision"];
        assert_eq!(out["hookSpecificOutput"]["hookEventName"], "PermissionRequest");
        assert_eq!(d["behavior"], "allow");
        assert_eq!(d["updatedInput"]["answers"]["Color?"], "Blue");
        assert_eq!(d["updatedInput"]["questions"][0]["question"], "Color?");
        assert!(r.list("s").is_empty());
    }

    #[test]
    fn always_allow_applies_the_suggested_rule_and_deny_carries_a_message() {
        let r = relay();
        let (a, mut rx_a) = r.open("s", &bash_body("touch x")).unwrap();
        assert!(r.list("s")[0].can_always);
        r.answer(&a, AskDecision::Allow { answers: None, always: true }).unwrap();
        let out: Value = serde_json::from_str(&rx_a.try_recv().unwrap()).unwrap();
        assert_eq!(out["hookSpecificOutput"]["decision"]["updatedPermissions"][0]["type"], "addRules");

        let (b, mut rx_b) = r.open("s", &bash_body("rm -rf y")).unwrap();
        r.answer(&b, AskDecision::Deny { message: None }).unwrap();
        let out: Value = serde_json::from_str(&rx_b.try_recv().unwrap()).unwrap();
        assert_eq!(out["hookSpecificOutput"]["decision"]["behavior"], "deny");
        assert_eq!(out["hookSpecificOutput"]["decision"]["message"], DEFAULT_DENY_MESSAGE);
    }

    #[test]
    fn dismiss_and_terminal_answers_release_without_a_decision() {
        let r = relay();
        let (a, mut rx_a) = r.open("s", &bash_body("one")).unwrap();
        r.answer(&a, AskDecision::Dismiss).unwrap();
        assert!(matches!(rx_a.try_recv(), Err(oneshot::error::TryRecvError::Closed)));

        let (_b, mut rx_b) = r.open("s", &bash_body("two")).unwrap();
        let (_c, _rx_c) = r.open("s", &bash_body("three")).unwrap();
        let (_d, _rx_d) = r.open("other", &bash_body("two")).unwrap();
        r.settle_tool("s", "Bash", &json!({ "command": "two" }));
        assert!(matches!(rx_b.try_recv(), Err(oneshot::error::TryRecvError::Closed)));
        assert_eq!(r.list("s").len(), 1);
        assert_eq!(r.list("other").len(), 1);
        r.settle_session("s");
        assert!(r.list("s").is_empty());
        assert_eq!(r.answer("ask-1", AskDecision::Dismiss), Err("ask-gone".into()));
    }

    #[test]
    fn answered_question_is_settled_by_post_tool_use_even_with_answers_attached() {
        let r = relay();
        let (_id, _rx) = r.open("s", &ask_body()).unwrap();
        let body: Value = serde_json::from_slice(&ask_body()).unwrap();
        let mut input = body["tool_input"].clone();
        input["answers"] = json!({ "Color?": "Red" });
        r.settle_tool("s", "AskUserQuestion", &input);
        assert!(r.list("s").is_empty());
    }

    #[test]
    fn turning_off_releases_everything_and_long_input_is_clipped() {
        let r = relay();
        let long = "x".repeat(MAX_INPUT_STRING + 50);
        let (_id, mut rx) = r.open("s", &bash_body(&long)).unwrap();
        let shown = r.list("s")[0].tool_input["command"].as_str().unwrap().chars().count();
        assert!(shown < MAX_INPUT_STRING + 5, "{shown}");
        r.set_enabled(false);
        assert!(matches!(rx.try_recv(), Err(oneshot::error::TryRecvError::Closed)));
        assert!(r.list("s").is_empty());
    }
}
