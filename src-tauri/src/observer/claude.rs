use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::types::SessionEventTokens;

use super::event::{
    agent_id, claude_file_usage, claude_subagent_transcripts, claude_transcript_message,
    claude_transcript_progress_message, hook_cwd, message, prompt_text, running_subagents,
    tool_activity_text, transcript_path,
};
use super::hook_command::forwarder_shell_command;
use super::{
    AdapterSessionPlan, CommandWrapperSpec, ObserverAdapter, ObserverAdapterError, ObserverEvent,
    ObserverProvider, ObserverSessionContext, RawObserverHook, WrapperArg,
};

/// transcript 꼬리 읽기(assistant 내레이션 추출) 스로틀 기본 간격. 파일 tail을
/// 매 PostToolUse마다 읽지 않도록 transcript_path별로 이 간격을 둔다(이슈 #43).
const TRANSCRIPT_PROGRESS_INTERVAL: Duration = Duration::from_secs(5);

/// 전사 파일 하나에 대한 사용량 스캔 상태.
#[derive(Default)]
struct UsageWatermark {
    /// 마지막으로 합산한 `message.id`. 다음 스캔은 이 지점까지만 거슬러 올라간다.
    message_id: Option<String>,
    /// 그때 본 파일 크기. 전사는 append-only라 크기가 같으면 내용도 같고, 다시
    /// 읽어 봐야 델타가 0이므로 스캔 자체를 건너뛴다. 서브에이전트 전사는 한
    /// 세션에 수십 개씩 쌓이고 대부분 이미 끝나 있어(=크기 고정) 이 스킵이
    /// 매 Stop마다 수십 MB 재읽기를 없앤다.
    scanned_len: u64,
}

pub struct ClaudeAdapter {
    settings_dir: PathBuf,
    forwarder_executable: PathBuf,
    /// transcript_path별 마지막 tail 읽기 "시도" 시각(스로틀 기준). 읽기 실패해도
    /// 시도 시각을 기록하므로 다음 interval까지 재시도하지 않는다.
    transcript_progress: Mutex<HashMap<String, Instant>>,
    /// 전사 경로별 턴 사용량 워터마크. 키는 메인 전사만이 아니라 **서브에이전트
    /// 전사 경로도 포함**한다(`claude_subagent_transcripts`) — 파일마다 독립적으로
    /// "어디까지 셌는지"를 들고 델타만 싣는다.
    ///
    /// `transcript_progress`와는 수명이 다르다 — 저건 스로틀용이라 Stop에서 그
    /// 경로 엔트리를 지우지만, 이건 **지우지 않는다**. 같은 프롬프트 창 안에서
    /// Stop이 두 번째로 와도(백그라운드 서브에이전트가 남아 있던 running>0
    /// Stop 뒤의 재호출) 직전 몫을 다시 세지 않으려면 그 경계를 넘어 남아
    /// 있어야 하기 때문이다. 세션이 끝나도 맵에서 안 지워지는 누수가 있지만
    /// 경로당 문자열 2개 규모라 무시할 만하다.
    ///
    /// 값이 `Arc<Mutex<..>>`인 이유: 훅 수신은 axum+tokio 멀티스레드이고
    /// forwarder에 1회 재시도가 있어 같은 Stop body가 겹쳐 들어올 수 있다.
    /// 바깥 맵 락은 경로별 락을 꺼내는 동안만 짧게 잡고, 조회~전사 읽기~합산~
    /// 갱신 전 구간은 그 경로별 내부 락으로 직렬화한다(전역 락을 IO 동안 잡으면
    /// 다른 세션의 Stop 훅까지 막힌다).
    transcript_usage_watermark: Mutex<HashMap<String, Arc<Mutex<UsageWatermark>>>>,
    /// tail 읽기 최소 간격(테스트는 with_progress_interval로 조정).
    progress_interval: Duration,
    /// 훅 설정 파일에 함께 실을 추가 최상위 키(동료 대화의 플러그인 선언 등).
    /// **쓸 때마다** 호출하므로 스폰과 입양 복구가 같은 값을 받는다 — 설정이
    /// 그 사이 꺼졌으면 다음 기록부터 조각이 빠진다.
    extra_settings: Option<Arc<dyn Fn() -> Option<serde_json::Value> + Send + Sync>>,
}

impl ClaudeAdapter {
    pub fn new(settings_dir: PathBuf, forwarder_executable: PathBuf) -> Self {
        Self::with_progress_interval(settings_dir, forwarder_executable, TRANSCRIPT_PROGRESS_INTERVAL)
    }

    /// 테스트/튜닝용: transcript tail 읽기 스로틀 간격을 지정해 생성한다.
    pub fn with_progress_interval(
        settings_dir: PathBuf,
        forwarder_executable: PathBuf,
        progress_interval: Duration,
    ) -> Self {
        Self {
            settings_dir,
            forwarder_executable,
            transcript_progress: Mutex::new(HashMap::new()),
            transcript_usage_watermark: Mutex::new(HashMap::new()),
            progress_interval,
            extra_settings: None,
        }
    }

    /// 이번 턴의 사용량 = **메인 전사 + 이 세션의 서브에이전트 전사들**.
    ///
    /// CLI 2.1.x는 Task 서브에이전트 대화를 `<session>/subagents/*.jsonl`에 따로
    /// 쓴다 — 메인 전사만 읽으면 그 몫(실측상 세션 토큰의 약 2/3)이 통째로
    /// 빠진다. 파일마다 워터마크가 독립이라 서브에이전트가 Stop 이후에도 계속
    /// append하는 경우(다음 Stop이 그 뒤를 이어 센다)도 자연히 커버된다.
    ///
    /// 메인 쪽이 None이어도(사용량 줄이 없던 턴) 서브 몫이 있으면 그것만으로
    /// 이벤트를 낸다.
    fn turn_usage(&self, transcript: &Path) -> Option<SessionEventTokens> {
        let mut total = self.usage_for_path(transcript);
        for sub in claude_subagent_transcripts(transcript) {
            let Some(tokens) = self.usage_for_path(&sub) else {
                continue;
            };
            total = Some(match total {
                // 메인 것을 self로 둬야 대표 모델이 메인 쪽으로 유지된다.
                Some(acc) => acc.merged(tokens),
                None => tokens,
            });
        }
        total
    }

    /// 전사 파일 하나의 "직전 스캔 이후" 사용량. 경로별 락을 조회~읽기~합산~
    /// 갱신 내내 유지해 겹친 Stop(포워더 재시도 등)이 같은 구간을 두 번 싣는
    /// 경합을 막는다(바깥 맵 락은 그 락을 꺼내는 동안만 잡는다).
    fn usage_for_path(&self, path: &Path) -> Option<SessionEventTokens> {
        let len = std::fs::metadata(path).ok()?.len();
        let path_lock = self
            .transcript_usage_watermark
            .lock()
            .unwrap()
            .entry(path.to_string_lossy().into_owned())
            .or_default()
            .clone();
        let mut mark = path_lock.lock().unwrap();
        // 크기가 그대로면 append가 없었다는 뜻 — 읽어도 델타가 0이다.
        if mark.scanned_len == len {
            return None;
        }
        let scanned = claude_file_usage(path, mark.message_id.as_deref());
        // 합산에 실패해도(유효 사용량 줄 없음) 크기는 기록한다 — 안 그러면
        // 그런 파일을 매 Stop마다 다시 읽는다.
        mark.scanned_len = len;
        let (tokens, newest_id) = scanned?;
        if let Some(newest_id) = newest_id {
            mark.message_id = Some(newest_id);
        }
        Some(tokens)
    }

    /// 훅 설정에 얹을 추가 조각 공급자를 붙인다(lib.rs가 동료 대화 선언을 넣는다).
    pub fn with_extra_settings(
        mut self,
        provider: Arc<dyn Fn() -> Option<serde_json::Value> + Send + Sync>,
    ) -> Self {
        self.extra_settings = Some(provider);
        self
    }

    /// PostToolUse를 도구 요약 + (스로틀 통과 시) assistant 내레이션 + 턴 중간
    /// 토큰 사용량으로 매핑한다.
    ///
    /// 세션 사용량 중간 갱신(터미널 요약 바가 Stop까지 안 기다리고 도구마다
    /// 갱신되게): `progress_due`가 이미 갖고 있던 transcript_path별 5초 스로틀
    /// 위에 그대로 얹는다 — 도구마다 전사를 다시 읽으면 비용이 크므로, 내레이션
    /// tail 읽기와 같은 예산으로 `self.turn_usage`(Stop이 쓰는 그 함수)를 부른다.
    /// `turn_usage`는 파일별 워터마크 델타라 같은 구간을 두 번 세지 않는다 —
    /// 여기서 한 번 세면 그 뒤 Tool이든 그 턴의 최종 Stop이든 다음 스캔은 그
    /// 뒤 증분만 잡는다(호출부 주석 `ObserverEvent::Tool::tokens` 참고).
    fn map_post_tool_use(&self, body: &[u8]) -> ObserverEvent {
        let due = self.progress_due(body);
        let tokens_if_due = || {
            due.then(|| transcript_path(body).and_then(|path| self.turn_usage(Path::new(&path))))
                .flatten()
        };
        // 서브에이전트 내부 도구(agent_id 있음)는 하트비트만 유지하고 라벨용
        // 텍스트를 싣지 않는다 — 부모 라벨이 서브에이전트 도구/내레이션으로
        // 오염되는 걸 막는다. 다만 tokens는 싣는다: 서브 갈래의 transcript_path는
        // 서브 자신의 `subagents/*.jsonl`을 가리키므로(메인과 무관한 별도
        // 워터마크 엔트리), 여기서 세도 메인 세션의 다음 Stop 합산과 겹치지 않는다.
        if agent_id(body).is_some() {
            return ObserverEvent::Tool {
                text: None,
                assistant: None,
                tokens: tokens_if_due(),
            };
        }
        let text = tool_activity_text(body);
        let assistant = if due {
            claude_transcript_progress_message(body)
        } else {
            None
        };
        ObserverEvent::Tool {
            text,
            assistant,
            tokens: tokens_if_due(),
        }
    }

    /// transcript_path별 스로틀: 마지막 읽기 시도 후 progress_interval이 지났으면
    /// true를 돌려주고 그 시각을 기록한다. transcript_path 부재면 false.
    fn progress_due(&self, body: &[u8]) -> bool {
        let Some(path) = transcript_path(body) else {
            return false;
        };
        let now = Instant::now();
        let mut map = self.transcript_progress.lock().unwrap();
        match map.get(&path) {
            Some(last) if now.duration_since(*last) < self.progress_interval => false,
            _ => {
                map.insert(path, now);
                true
            }
        }
    }

    /// 훅 명령을 앱 바이너리 forwarder(`--observer-forward claude <event>`)로 만든다.
    ///
    /// 예전에는 curl로 훅 URL을 명령에 박아 넣었는데, 그러면 재시작 후 sessiond로
    /// 입양된 세션이 죽은(스폰 시점) 포트를 계속 때리고 `|| true`로 조용히 실패했다
    /// (docs/session-handoff-design.md §핵심 5, 이슈 #30). forwarder는 실행 시점에
    /// 세션 env의 `AGENT_OFFICE_HOOK_URL`을 읽고, 연결이 거부되면
    /// `AGENT_OFFICE_APP_DATA/observer-port` 파일의 최신 포트로 1회 재시도한다.
    ///
    /// SessionStart/SessionEnd처럼 훅 stdout이 대화 컨텍스트로 주입되는 이벤트에도
    /// 같은 명령을 쓴다. forwarder는 stdout에 아무것도 쓰지 않으므로(서버 응답을
    /// 버린다) 예전 curl `-o /dev/null` 변형이 필요 없다.
    fn hook_command(&self, event: &str) -> Result<String, ObserverAdapterError> {
        forwarder_shell_command(&self.forwarder_executable, &["claude", event])
    }

    /// 훅 설정 JSON을 `path`에 (부모 디렉터리 생성 포함) 원자적으로 쓴다. 내용은
    /// **세션 무관**이다 — forwarder 명령만 담기고 sessionId·포트는 박히지 않는다
    /// (이슈 #30). 그래서 스폰(`prepare_session`)과 입양 복구(`restore_session_artifact`)가
    /// 같은 함수를 쓴다. temp+rename으로 부분 기록된 파일이 노출되지 않게 한다.
    fn write_settings_file(&self, path: &Path) -> Result<(), ObserverAdapterError> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|error| {
                ObserverAdapterError::new(format!("Claude settings directory failed: {error}"))
            })?;
        }
        // forwarder 경로 검증 실패 시 여기서 Err를 전파한다(codex와 동일 계약).
        let entry = |command: String| {
            serde_json::json!([{
                "matcher": "",
                "hooks": [{
                    "type": "command",
                    "command": command,
                }],
            }])
        };
        // SessionStart/SessionEnd는 map_hook에서 이벤트로 매핑되지 않지만(허브
        // 무영향), ingest의 리줌 ID 캡처가 body를 본다 — 프롬프트 한 번 없이
        // 시작·종료한 세션도 리줌 ID를 남기기 위한 등록(리뷰 지적 반영,
        // docs/claude-session-resume-design.md §2). 8개 이벤트 모두 forwarder
        // 명령을 쓴다(위 hook_command 주석: 예전 silent 변형은 불필요).
        let mut settings = serde_json::json!({
            "hooks": {
                "UserPromptSubmit": entry(self.hook_command("UserPromptSubmit")?),
                "PostToolUse": entry(self.hook_command("PostToolUse")?),
                "Notification": entry(self.hook_command("Notification")?),
                "Stop": entry(self.hook_command("Stop")?),
                "SubagentStart": entry(self.hook_command("SubagentStart")?),
                "SubagentStop": entry(self.hook_command("SubagentStop")?),
                "SessionStart": entry(self.hook_command("SessionStart")?),
                "SessionEnd": entry(self.hook_command("SessionEnd")?),
            },
        });
        // 동료 대화 등 추가 조각을 최상위에 얹는다(훅 키는 건드리지 않는다).
        if let Some(extra) = self.extra_settings.as_ref().and_then(|f| f()) {
            if let (Some(target), Some(extra)) = (settings.as_object_mut(), extra.as_object()) {
                for (key, value) in extra {
                    target.insert(key.clone(), value.clone());
                }
            }
        }
        let contents = serde_json::to_vec_pretty(&settings)
            .expect("serializing Claude hook settings cannot fail");
        // temp+rename: 같은 디렉터리에 임시 파일로 쓴 뒤 원자적으로 옮긴다.
        // (`foo.settings.json` → `foo.settings.json.tmp` — `.settings.json`으로
        // 끝나지 않아 gc/복구 파일명 매칭에 걸리지 않는다.)
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, contents)
            .and_then(|()| std::fs::rename(&tmp, path))
            .map_err(|error| {
                let _ = std::fs::remove_file(&tmp);
                ObserverAdapterError::new(format!("Claude settings write failed: {error}"))
            })
    }
}

/// `dir` 안에서 `*.settings.json` 파일 중 mtime이 `max_age`를 넘긴 것을 지운다
/// (이슈 #40 D8). 설정 파일이 OS temp에서 app_data로 이동하면서 더블-크래시로
/// 정리 못 한 아티팩트가 영구화될 수 있어, 부트 시 1회 백그라운드로 청소한다.
/// 살아 있는 세션은 매 입양마다 재작성돼 mtime이 갱신되므로 안전하다.
pub fn gc_stale_settings(dir: &Path, max_age: Duration) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return; // 디렉터리 부재(설정이 한 번도 안 만들어짐) = 청소할 것 없음.
    };
    let now = std::time::SystemTime::now();
    for entry in entries.flatten() {
        let path = entry.path();
        if !path
            .file_name()
            .is_some_and(|n| n.to_string_lossy().ends_with(".settings.json"))
        {
            continue;
        }
        let stale = entry
            .metadata()
            .and_then(|m| m.modified())
            .ok()
            .and_then(|modified| now.duration_since(modified).ok())
            .is_some_and(|age| age > max_age);
        if stale {
            if let Err(error) = std::fs::remove_file(&path) {
                if error.kind() != std::io::ErrorKind::NotFound {
                    eprintln!("agent-office: stale settings cleanup failed for {path:?}: {error}");
                }
            }
        }
    }
}

impl ObserverAdapter for ClaudeAdapter {
    fn provider(&self) -> ObserverProvider {
        ObserverProvider::Claude
    }

    fn prepare_session(
        &self,
        context: &ObserverSessionContext,
    ) -> Result<AdapterSessionPlan, ObserverAdapterError> {
        let path = self
            .settings_dir
            .join(format!("{}.settings.json", context.session_id));
        self.write_settings_file(&path)?;

        Ok(AdapterSessionPlan {
            env: vec![(
                "AGENT_OFFICE_SETTINGS".into(),
                path.to_string_lossy().into_owned(),
            )],
            wrappers: vec![CommandWrapperSpec {
                command: "claude".into(),
                prefix_args: vec![
                    WrapperArg::Literal("--settings".into()),
                    WrapperArg::Env("AGENT_OFFICE_SETTINGS".into()),
                ],
                skip_if_present: vec!["--settings".into()],
                // 이슈 #40: 앱이 꺼진 사이 설정 파일이 사라져도 `claude --settings
                // <없는 파일>`로 하드 실패하지 않고 비관찰로 강등 실행하게 한다.
                skip_prefix_if_env_file_missing: Some("AGENT_OFFICE_SETTINGS".into()),
            }],
            cleanup_paths: vec![path],
        })
    }

    /// 입양 시 설정 파일 복구(이슈 #40). 파일명이 `.settings.json`으로 끝나는
    /// 경로만 이 어댑터 소관이다. 존재 여부와 무관하게 **멱등 재작성**하므로,
    /// 파일이 사라졌든 낡은 forwarder 경로가 남았든 현재 값으로 복원된다.
    fn restore_session_artifact(&self, path: &Path) -> Result<bool, ObserverAdapterError> {
        if !path
            .file_name()
            .is_some_and(|n| n.to_string_lossy().ends_with(".settings.json"))
        {
            return Ok(false);
        }
        self.write_settings_file(path)?;
        Ok(true)
    }

    fn map_hook(&self, raw: &RawObserverHook<'_>) -> Option<ObserverEvent> {
        // Subagent-internal hooks (agent_id present) must never open or close the main
        // turn boundary (opening via Prompt or closing via Stop). Every time a Task
        // subagent finishes an internal turn, Claude Code fires a Stop hook with
        // agent_id populated; treating that as main-session termination grays out the
        // label. SubagentStart/Stop are lifecycle signals with agent_id normally
        // present, so let them pass through. Tool (PostToolUse) / Attention
        // (Notification) are heartbeat/attention signals, so let them pass through too.
        if matches!(raw.event_name, "Stop" | "UserPromptSubmit") && agent_id(raw.body).is_some() {
            return None;
        }

        match raw.event_name {
            "UserPromptSubmit" => Some(ObserverEvent::Prompt {
                text: prompt_text(raw.body),
                cwd: hook_cwd(raw.body),
            }),
            "PostToolUse" => Some(self.map_post_tool_use(raw.body)),
            "SubagentStart" => Some(ObserverEvent::SubStart),
            // 절대 카운트는 self 제외를 위해 top-level agent_id가 반드시 있어야 신뢰할 수
            // 있다(리뷰 지적: agent_id 부재 시 자기 자신까지 세어 off-by-one으로 미니미
            // 잔존). agent_id 또는 background_tasks가 없으면 안전하게 SubStop 델타로 강등.
            "SubagentStop" => Some(match (agent_id(raw.body), running_subagents(raw.body)) {
                (Some(_), Some(running)) => ObserverEvent::SubCount { running },
                _ => ObserverEvent::SubStop,
            }),
            "Notification" => Some(ObserverEvent::Attention {
                message: message(raw.body),
            }),
            // 이슈 #39: Claude Stop 훅 body 엔 message 필드가 없다 → transcript_path
            // (JSONL)의 마지막 assistant 텍스트를 완료 본문으로 뽑는다. 파일 부재/
            // 포맷 이상은 None 폴백 → hub 의 STOP_FALLBACK 유지. body 에 message 가
            // 실려 오는 경로(pi 등 미래 확장)는 그대로 우선한다.
            "Stop" => {
                // 턴 종료 → 이 transcript의 progress 스로틀 엔트리 제거(맵 누수 방지).
                // 사용량 워터마크는 지우지 않는다(위 필드 주석 참고 — 같은 프롬프트
                // 창의 2회차 Stop이 필요로 한다).
                if let Some(path) = transcript_path(raw.body) {
                    self.transcript_progress.lock().unwrap().remove(&path);
                }
                // 턴 사용량은 메인 전사 + 이 세션의 서브에이전트 전사들에서 뽑는다
                // (추출 실패는 None).
                let tokens =
                    transcript_path(raw.body).and_then(|path| self.turn_usage(Path::new(&path)));
                Some(ObserverEvent::Stop {
                    message: message(raw.body).or_else(|| claude_transcript_message(raw.body)),
                    running: running_subagents(raw.body),
                    tokens,
                })
            }
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::ClaudeAdapter;
    use crate::observer::{
        ObserverAdapter, ObserverEvent, ObserverSessionContext, RawObserverHook, WrapperArg,
    };

    fn scratch_dir() -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "agent-office-observer-adapter-test-{}",
            uuid::Uuid::new_v4(),
        ))
    }

    /// 훅 명령 빌더는 절대 경로 forwarder를 요구한다.
    fn forwarder_exe() -> std::path::PathBuf {
        if cfg!(windows) {
            std::path::PathBuf::from(r"C:\Program Files\Agent Office\agent-office.exe")
        } else {
            std::path::PathBuf::from("/opt/agent-office/agent-office")
        }
    }

    #[test]
    fn claude_plan_writes_four_hooks_and_settings_wrapper() {
        let dir = scratch_dir();
        let adapter = ClaudeAdapter::new(dir.clone(), forwarder_exe());
        let context = ObserverSessionContext::new("ao-s1", "http://127.0.0.1:43123/hook");

        let plan = adapter.prepare_session(&context).unwrap();

        let path = dir.join("ao-s1.settings.json");
        assert_eq!(
            plan.env,
            vec![(
                "AGENT_OFFICE_SETTINGS".into(),
                path.to_string_lossy().into_owned(),
            )],
        );
        assert_eq!(plan.cleanup_paths, vec![path.clone()]);
        assert_eq!(plan.wrappers.len(), 1);
        assert_eq!(plan.wrappers[0].command, "claude");
        assert_eq!(
            plan.wrappers[0].prefix_args,
            vec![
                WrapperArg::Literal("--settings".into()),
                WrapperArg::Env("AGENT_OFFICE_SETTINGS".into()),
            ],
        );
        assert_eq!(plan.wrappers[0].skip_if_present, vec!["--settings"]);

        let raw = std::fs::read_to_string(&path).unwrap();
        // 스테일 포트 회귀 방지의 핵심 어서션: 훅 URL/포트가 더 이상 설정 파일에
        // 박히지 않는다 — forwarder가 실행 시점에 최신 포트로 라우팅한다(이슈 #30).
        assert!(
            !raw.contains("127.0.0.1"),
            "settings must not embed a spawn-time observer port: {raw}",
        );
        let json: serde_json::Value = serde_json::from_str(&raw).unwrap();
        for event in ["UserPromptSubmit", "PostToolUse", "Notification", "Stop"] {
            let entry = &json["hooks"][event][0];
            assert_eq!(entry["matcher"], "", "wrong matcher for {event}: {json}");
            assert_eq!(entry["hooks"][0]["type"], "command");
            let command = entry["hooks"][0]["command"].as_str().unwrap();
            // unix는 forwarder 명령이 평문이라 인자를 직접 검증한다. windows는
            // powershell EncodedCommand(base64)라 실행 픽스처로 검증한다(codex와 동일).
            #[cfg(not(windows))]
            {
                assert!(
                    command.contains("--observer-forward")
                        && command.contains("claude")
                        && command.contains(event),
                    "hook must forward via the app binary for {event}: {command}",
                );
            }
            #[cfg(windows)]
            {
                assert!(
                    command.contains("powershell.exe"),
                    "windows hook must use powershell forwarder for {event}: {command}",
                );
            }
        }

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn claude_plan_preserves_subagent_lifecycle_hooks_from_the_legacy_observer() {
        let dir = scratch_dir();
        let adapter = ClaudeAdapter::new(dir.clone(), forwarder_exe());
        let context = ObserverSessionContext::new("ao-s1", "http://127.0.0.1:43123/hook");

        adapter.prepare_session(&context).unwrap();
        let path = dir.join("ao-s1.settings.json");
        let json: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();

        for event in ["SubagentStart", "SubagentStop"] {
            let entry = &json["hooks"][event][0];
            assert_eq!(entry["matcher"], "", "missing {event} hook: {json}");
            let command = entry["hooks"][0]["command"].as_str().unwrap();
            #[cfg(not(windows))]
            assert!(
                command.contains("--observer-forward")
                    && command.contains("claude")
                    && command.contains(event),
                "wrong {event} command: {command}",
            );
            #[cfg(windows)]
            assert!(
                command.contains("powershell.exe"),
                "wrong {event} command: {command}",
            );
        }

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn claude_plan_registers_session_lifecycle_hooks_via_forwarder() {
        let dir = scratch_dir();
        let adapter = ClaudeAdapter::new(dir.clone(), forwarder_exe());
        let context = ObserverSessionContext::new("ao-s1", "http://127.0.0.1:43123/hook");

        adapter.prepare_session(&context).unwrap();
        let path = dir.join("ao-s1.settings.json");
        let raw = std::fs::read_to_string(&path).unwrap();
        let json: serde_json::Value = serde_json::from_str(&raw).unwrap();

        for event in ["SessionStart", "SessionEnd"] {
            let entry = &json["hooks"][event][0];
            assert_eq!(entry["matcher"], "", "missing {event} hook: {json}");
            let command = entry["hooks"][0]["command"].as_str().unwrap();
            // SessionStart/End는 훅 stdout이 세션 컨텍스트로 주입되지만, forwarder는
            // stdout에 아무것도 쓰지 않으므로(서버 응답을 버린다) 예전 curl
            // `-o /dev/null` 변형 없이 나머지 이벤트와 같은 명령을 쓴다.
            #[cfg(not(windows))]
            assert!(
                command.contains("--observer-forward")
                    && command.contains("claude")
                    && command.contains(event),
                "wrong {event} command: {command}",
            );
            #[cfg(windows)]
            assert!(
                command.contains("powershell.exe"),
                "wrong {event} command: {command}",
            );
        }
        // 스테일 포트 회귀 방지(이슈 #30): 어떤 훅에도 포트가 박히지 않는다.
        assert!(!raw.contains("127.0.0.1"), "settings must not embed a port: {raw}");

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn session_lifecycle_hooks_map_to_no_observer_event() {
        // 리줌 ID 캡처 전용 등록 — 허브 이벤트(턴 경계·활동)에는 영향이 없어야 한다.
        let adapter = ClaudeAdapter::new(scratch_dir(), forwarder_exe());
        for event_name in ["SessionStart", "SessionEnd"] {
            assert_eq!(
                adapter.map_hook(&RawObserverHook {
                    event_name,
                    body: br#"{"session_id":"native-1"}"#,
                }),
                None,
            );
        }
    }

    // 훅 명령이 URL/포트를 담지 않고 앱 바이너리 forwarder를 경유하는지 확인한다
    // (이슈 #30 스테일 포트 회귀 방지). unix는 명령이 평문이라 형태를 직접 검증한다.
    #[cfg(not(windows))]
    #[test]
    fn claude_hook_command_forwards_via_app_binary_on_unix() {
        let adapter = ClaudeAdapter::new(
            scratch_dir(),
            std::path::PathBuf::from("/tmp/Agent 'Office'/agent-office"),
        );

        let command = adapter.hook_command("Stop").unwrap();
        assert_eq!(
            command,
            "'/tmp/Agent '\"'\"'Office'\"'\"'/agent-office' --observer-forward claude Stop",
        );
        // 포트가 명령에 없어야 한다 — forwarder가 실행 시점에 라우팅한다.
        assert!(!command.contains("127.0.0.1"));
    }

    // forwarder 경로가 절대경로가 아니면 prepare_session이 Err를 반환한다(codex와 동일 계약).
    #[test]
    fn claude_prepare_session_rejects_relative_forwarder_path() {
        let adapter = ClaudeAdapter::new(scratch_dir(), std::path::PathBuf::from("agent-office"));
        let context = ObserverSessionContext::new("ao-s1", "http://127.0.0.1:43123/hook");
        assert_eq!(
            adapter.prepare_session(&context).unwrap_err().to_string(),
            "observer forwarder path must be absolute",
        );
    }

    #[test]
    fn claude_missing_messages_defer_to_hub_fallback() {
        let adapter = ClaudeAdapter::new(scratch_dir(), forwarder_exe());

        for body in [
            b"{}".as_slice(),
            b"not json".as_slice(),
            br#"{"message":"   "}"#.as_slice(),
        ] {
            assert_eq!(
                adapter.map_hook(&RawObserverHook {
                    event_name: "Notification",
                    body,
                }),
                Some(ObserverEvent::Attention { message: None }),
            );
            assert_eq!(
                adapter.map_hook(&RawObserverHook {
                    event_name: "Stop",
                    body,
                }),
                Some(ObserverEvent::Stop {
                    message: None,
                    running: None,
                    tokens: None,
                }),
            );
        }
    }

    #[test]
    fn claude_stop_reads_completion_from_transcript_tail() {
        // 이슈 #39: message 필드가 없어도 transcript_path 의 마지막 assistant
        // 텍스트를 완료 본문으로 실어야 한다.
        let adapter = ClaudeAdapter::new(scratch_dir(), forwarder_exe());
        let dir = scratch_dir();
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("transcript.jsonl");
        let lines = [
            r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"이전 응답"}]}}"#,
            r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"작업을 마쳤습니다"}]}}"#,
        ];
        std::fs::write(&path, lines.join("\n")).unwrap();
        let body = serde_json::json!({ "transcript_path": path.to_string_lossy() })
            .to_string()
            .into_bytes();

        assert_eq!(
            adapter.map_hook(&RawObserverHook {
                event_name: "Stop",
                body: &body,
            }),
            Some(ObserverEvent::Stop {
                message: Some("작업을 마쳤습니다".into()),
                running: None,
                tokens: None,
            }),
        );

        // transcript 부재 시엔 None 폴백(hub STOP_FALLBACK).
        assert_eq!(
            adapter.map_hook(&RawObserverHook {
                event_name: "Stop",
                body: br#"{"transcript_path":"/nonexistent/transcript.jsonl"}"#,
            }),
            Some(ObserverEvent::Stop {
                message: None,
                running: None,
                tokens: None,
            }),
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    /// Stop은 완료 본문과 함께 그 턴의 토큰 사용량도 같은 전사 꼬리에서 실어야
    /// 한다(usage 없는 전사는 tokens=None 으로 강등).
    #[test]
    fn claude_stop_carries_turn_usage_from_the_transcript() {
        let adapter = ClaudeAdapter::new(scratch_dir(), forwarder_exe());
        let dir = scratch_dir();
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("transcript.jsonl");
        let lines = [
            r#"{"type":"user","message":{"role":"user","content":"작업 지시"}}"#,
            r#"{"type":"assistant","message":{"id":"m1","model":"claude-opus-5","role":"assistant","content":[{"type":"text","text":"완료"}],"usage":{"input_tokens":11,"output_tokens":22,"cache_read_input_tokens":33,"cache_creation_input_tokens":44}}}"#,
        ];
        std::fs::write(&path, lines.join("\n")).unwrap();
        let body = serde_json::json!({ "transcript_path": path.to_string_lossy() })
            .to_string()
            .into_bytes();

        let Some(ObserverEvent::Stop { tokens, .. }) = adapter.map_hook(&RawObserverHook {
            event_name: "Stop",
            body: &body,
        }) else {
            panic!("Stop 이벤트가 나와야 한다");
        };
        let tokens = tokens.expect("전사에 usage가 있으면 실려야 한다");
        assert_eq!(tokens.input, Some(11));
        assert_eq!(tokens.output, Some(22));
        assert_eq!(tokens.cache_read, Some(33));
        assert_eq!(tokens.cache_write, Some(44));
        assert_eq!(tokens.model.as_deref(), Some("claude-opus-5"));

        let _ = std::fs::remove_dir_all(dir);
    }

    /// 워터마크(S5): 같은 전사에 대해 Stop이 두 번 오면(백그라운드 서브에이전트가
    /// 남아 있던 running>0 Stop 뒤 재호출 등) 1회차가 이미 합산한 몫을 2회차가
    /// 다시 세면 안 된다. 그 사이 새 assistant 줄이 추가되면 2회차는 그 증분만
    /// 실어야 한다.
    #[test]
    fn repeated_stop_on_the_same_transcript_only_carries_the_increment() {
        let adapter = ClaudeAdapter::new(scratch_dir(), forwarder_exe());
        let dir = scratch_dir();
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("transcript.jsonl");
        let lines = [
            r#"{"type":"user","message":{"role":"user","content":"작업 지시"}}"#.to_string(),
            r#"{"type":"assistant","message":{"id":"m1","model":"claude-opus-5","role":"assistant","content":[{"type":"text","text":"1차 응답"}],"usage":{"input_tokens":11,"output_tokens":22,"cache_read_input_tokens":33,"cache_creation_input_tokens":44}}}"#.to_string(),
        ];
        std::fs::write(&path, lines.join("\n")).unwrap();
        let body = serde_json::json!({ "transcript_path": path.to_string_lossy() })
            .to_string()
            .into_bytes();

        // 1회차: 평소대로 합산된다.
        let Some(ObserverEvent::Stop { tokens, .. }) = adapter.map_hook(&RawObserverHook {
            event_name: "Stop",
            body: &body,
        }) else {
            panic!("Stop 이벤트가 나와야 한다");
        };
        let tokens = tokens.expect("전사에 usage가 있으면 실려야 한다");
        assert_eq!(tokens.input, Some(11));

        // 2회차: 새 줄이 안 늘었으므로 워터마크가 전부 걸러 None.
        let Some(ObserverEvent::Stop { tokens, .. }) = adapter.map_hook(&RawObserverHook {
            event_name: "Stop",
            body: &body,
        }) else {
            panic!("Stop 이벤트가 나와야 한다");
        };
        assert_eq!(tokens, None, "직전 Stop이 이미 합산한 몫을 다시 세면 안 된다");

        // 그 사이 새 assistant 줄이 추가되면(같은 프롬프트 창 안에서 Stop이 또
        // 온 경우 — 백그라운드 서브에이전트가 남아 있던 running>0 Stop 뒤의
        // 재호출 등) 3회차는 증분만 싣는다. **진짜 user 프롬프트 줄은 넣지
        // 않는다** — 넣으면 프롬프트 경계만으로도 같은 결과가 나와 워터마크가
        // 실제로 이 결과를 만들었는지 증명하지 못한다(필수 3).
        let mut appended = lines.clone().to_vec();
        appended.push(r#"{"type":"assistant","message":{"id":"m2","model":"claude-opus-5","role":"assistant","content":[{"type":"text","text":"2차 응답"}],"usage":{"input_tokens":5,"output_tokens":6,"cache_read_input_tokens":7,"cache_creation_input_tokens":8}}}"#.to_string());
        std::fs::write(&path, appended.join("\n")).unwrap();

        let Some(ObserverEvent::Stop { tokens, .. }) = adapter.map_hook(&RawObserverHook {
            event_name: "Stop",
            body: &body,
        }) else {
            panic!("Stop 이벤트가 나와야 한다");
        };
        let tokens = tokens.expect("증분이 있으면 실려야 한다");
        assert_eq!(tokens.input, Some(5));
        assert_eq!(tokens.output, Some(6));

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn subagent_internal_hooks_cannot_open_or_close_the_main_turn() {
        let adapter = ClaudeAdapter::new(scratch_dir(), forwarder_exe());
        let map = |event_name, body| adapter.map_hook(&RawObserverHook { event_name, body });

        assert_eq!(map("Stop", br#"{"agent_id":"sub-1","message":"m"}"#), None,);
        assert_eq!(
            map("Stop", br#"{"message":"m"}"#),
            Some(ObserverEvent::Stop {
                message: Some("m".into()),
                running: None,
                tokens: None,
            }),
        );
        assert_eq!(
            map("UserPromptSubmit", br#"{"agent_id":"sub-1","prompt":"x"}"#,),
            None,
        );
        // 서브에이전트 내부 도구는 하트비트만 — 텍스트/내레이션 모두 None(부모 라벨 보호).
        assert_eq!(
            map(
                "PostToolUse",
                br#"{"agent_id":"sub-1","tool_name":"Bash","tool_input":{"command":"npm test"}}"#,
            ),
            Some(ObserverEvent::Tool {
                text: None,
                assistant: None,
                tokens: None,
            }),
        );
        assert_eq!(
            map(
                "Notification",
                br#"{"agent_id":"sub-1","message":"needs permission"}"#,
            ),
            Some(ObserverEvent::Attention {
                message: Some("needs permission".into()),
            }),
        );
        assert_eq!(
            map("SubagentStart", br#"{"agent_id":"sub-1"}"#),
            Some(ObserverEvent::SubStart),
        );
        // agent_id 있는 SubagentStop이라도 background_tasks가 없으면 SubStop 델타로 강등.
        assert_eq!(
            map("SubagentStop", br#"{"agent_id":"sub-1"}"#),
            Some(ObserverEvent::SubStop),
        );
        assert_eq!(
            map("Stop", br#"{"agent_id":"","message":"m"}"#),
            Some(ObserverEvent::Stop {
                message: Some("m".into()),
                running: None,
                tokens: None,
            }),
        );
    }

    #[test]
    fn claude_maps_background_task_snapshots_to_absolute_counts() {
        let adapter = ClaudeAdapter::new(scratch_dir(), forwarder_exe());
        let subagent_body = br#"{
            "agent_id":"self",
            "background_tasks":[
                {"id":"self","type":"subagent","status":"running"},
                {"id":"other","type":"subagent","status":"running"}
            ]
        }"#;
        assert_eq!(
            adapter.map_hook(&RawObserverHook {
                event_name: "SubagentStop",
                body: subagent_body,
            }),
            Some(ObserverEvent::SubCount { running: 1 }),
        );
        assert_eq!(
            adapter.map_hook(&RawObserverHook {
                event_name: "SubagentStop",
                body: b"{}",
            }),
            Some(ObserverEvent::SubStop),
        );

        let stop_body = br#"{
            "message":"done",
            "background_tasks":[
                {"id":"one","type":"subagent","status":"running"},
                {"id":"two","type":"subagent","status":"running"}
            ]
        }"#;
        assert_eq!(
            adapter.map_hook(&RawObserverHook {
                event_name: "Stop",
                body: stop_body,
            }),
            Some(ObserverEvent::Stop {
                message: Some("done".into()),
                running: Some(2),
                tokens: None,
            }),
        );
    }

    #[test]
    fn user_prompt_carries_hook_cwd_into_prompt_event() {
        // 이슈 #44 작업 D: UserPromptSubmit body의 top-level cwd가 Prompt.cwd로 실려야 한다.
        let adapter = ClaudeAdapter::new(scratch_dir(), forwarder_exe());
        assert_eq!(
            adapter.map_hook(&RawObserverHook {
                event_name: "UserPromptSubmit",
                body: r#"{"prompt":"버그 고쳐줘","cwd":"/home/x/project"}"#.as_bytes(),
            }),
            Some(ObserverEvent::Prompt {
                text: Some("버그 고쳐줘".into()),
                cwd: Some("/home/x/project".into()),
            }),
        );
        // cwd 부재 body는 None.
        assert_eq!(
            adapter.map_hook(&RawObserverHook {
                event_name: "UserPromptSubmit",
                body: r#"{"prompt":"버그 고쳐줘"}"#.as_bytes(),
            }),
            Some(ObserverEvent::Prompt {
                text: Some("버그 고쳐줘".into()),
                cwd: None,
            }),
        );
    }

    #[test]
    fn post_tool_use_carries_tool_summary_for_main_session() {
        // 이슈 #43: 메인 세션 PostToolUse는 도구 요약을 라벨용 text로 싣는다.
        let adapter = ClaudeAdapter::new(scratch_dir(), forwarder_exe());
        assert_eq!(
            adapter.map_hook(&RawObserverHook {
                event_name: "PostToolUse",
                body: br#"{"tool_name":"Bash","tool_input":{"command":"npm test"}}"#,
            }),
            Some(ObserverEvent::Tool {
                text: Some("Bash: npm test".into()),
                assistant: None, // transcript_path 부재 → 내레이션 없음
                tokens: None,    // transcript_path 부재 → 사용량도 없음
            }),
        );
    }

    #[test]
    fn post_tool_use_throttles_transcript_reads_per_path() {
        use std::time::Duration;
        // 진짜 프롬프트 → assistant 내레이션 → tool_result user(턴 중간 실황).
        let dir = scratch_dir();
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("transcript.jsonl");
        let lines = [
            r#"{"type":"user","message":{"role":"user","content":"작업"}}"#,
            r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"text","text":"진행 중"}]}}"#,
            r#"{"type":"user","message":{"role":"user","content":[{"type":"tool_result","content":"ok"}]}}"#,
        ];
        std::fs::write(&path, lines.join("\n")).unwrap();
        let body = serde_json::json!({ "transcript_path": path.to_string_lossy() })
            .to_string()
            .into_bytes();
        let post = |adapter: &ClaudeAdapter| {
            adapter.map_hook(&RawObserverHook {
                event_name: "PostToolUse",
                body: &body,
            })
        };
        // 이 전사엔 usage 필드가 없는 assistant 줄뿐이라(내레이션 검증 전용
        // 픽스처) due=true여도 tokens는 항상 None — 이 테스트의 관심사는
        // 내레이션 스로틀이지 사용량 추출이 아니다.
        let with_narration = Some(ObserverEvent::Tool {
            text: None,
            assistant: Some("진행 중".into()),
            tokens: None,
        });
        let without = Some(ObserverEvent::Tool {
            text: None,
            assistant: None,
            tokens: None,
        });

        // interval ZERO: 매 호출마다 tail을 읽어 내레이션을 싣는다.
        let always =
            ClaudeAdapter::with_progress_interval(scratch_dir(), forwarder_exe(), Duration::ZERO);
        assert_eq!(post(&always), with_narration);
        assert_eq!(post(&always), with_narration);

        // 큰 interval: 첫 호출만 읽고 두 번째는 스로틀로 assistant=None.
        let throttled = ClaudeAdapter::with_progress_interval(
            scratch_dir(),
            forwarder_exe(),
            Duration::from_secs(3600),
        );
        assert_eq!(post(&throttled), with_narration);
        assert_eq!(post(&throttled), without);

        // Stop이 스로틀 엔트리를 제거하면 이후 PostToolUse가 다시 읽는다.
        throttled.map_hook(&RawObserverHook {
            event_name: "Stop",
            body: &body,
        });
        assert_eq!(post(&throttled), with_narration);

        let _ = std::fs::remove_dir_all(dir);
    }

    /// 세션 사용량 중간 갱신(터미널 요약 바가 Stop까지 안 기다림): PostToolUse도
    /// Stop과 같은 워터마크 델타로 `tokens`를 싣되, 그 위에 내레이션과 같은 5초
    /// progress 스로틀이 얹힌다 — 스로틀 안의 두 번째 호출은 파일을 다시 읽지
    /// 않으므로 tokens가 항상 None이다(도구 요약 텍스트가 유무와 무관하게).
    #[test]
    fn post_tool_use_carries_mid_turn_token_usage_and_throttles_it_like_narration() {
        use std::time::Duration;
        let dir = scratch_dir();
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("transcript.jsonl");
        let lines = [
            r#"{"type":"user","message":{"role":"user","content":"작업 지시"}}"#.to_string(),
            r#"{"type":"assistant","message":{"id":"m1","model":"claude-opus-5","role":"assistant","content":[{"type":"text","text":"진행 중"}],"usage":{"input_tokens":11,"output_tokens":22,"cache_read_input_tokens":0,"cache_creation_input_tokens":0}}}"#.to_string(),
        ];
        std::fs::write(&path, lines.join("\n")).unwrap();
        let body = serde_json::json!({ "tool_name": "Bash", "transcript_path": path.to_string_lossy() })
            .to_string()
            .into_bytes();

        // 큰 interval: 첫 호출만 스로틀을 통과해 사용량을 싣고, 그 사이 파일이
        // 그대로여도 두 번째 호출은 애초에 안 읽으므로(due=false) tokens: None.
        let adapter = ClaudeAdapter::with_progress_interval(
            scratch_dir(),
            forwarder_exe(),
            Duration::from_secs(3600),
        );
        let post = || {
            adapter.map_hook(&RawObserverHook {
                event_name: "PostToolUse",
                body: &body,
            })
        };
        let Some(ObserverEvent::Tool { tokens, .. }) = post() else {
            panic!("Tool 이벤트가 나와야 한다");
        };
        let tokens = tokens.expect("첫 호출은 스로틀을 통과해 사용량을 실어야 한다");
        assert_eq!(tokens.input, Some(11));
        assert_eq!(tokens.output, Some(22));

        let Some(ObserverEvent::Tool { tokens, .. }) = post() else {
            panic!("Tool 이벤트가 나와야 한다");
        };
        assert_eq!(tokens, None, "스로틀 안의 두 번째 호출은 tokens를 싣지 않는다");

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn restore_rewrites_a_deleted_settings_file_identically() {
        // 이슈 #40: 입양 시 사라진 설정 파일을 멱등 재작성한다. 재작성 내용은
        // prepare_session 산출과 동일해야 한다(훅 8종, 포트 미포함).
        use crate::observer::ObserverAdapter;
        let dir = scratch_dir();
        let adapter = ClaudeAdapter::new(dir.clone(), forwarder_exe());
        let context = ObserverSessionContext::new("ao-s1", "http://127.0.0.1:43123/hook");

        let plan = adapter.prepare_session(&context).unwrap();
        let path = dir.join("ao-s1.settings.json");
        let original = std::fs::read_to_string(&path).unwrap();

        // 앱이 꺼진 사이 사라진 상황을 흉내낸다.
        std::fs::remove_file(&path).unwrap();
        assert!(!path.exists());

        assert!(adapter.restore_session_artifact(&path).unwrap());
        let restored = std::fs::read_to_string(&path).unwrap();
        assert_eq!(restored, original, "restore must reproduce the settings file");
        assert!(!restored.contains("127.0.0.1"), "no spawn-time port: {restored}");
        // plan의 cleanup_paths/env가 같은 경로를 가리키는지도 확인(계약 불변).
        assert_eq!(plan.cleanup_paths, vec![path.clone()]);

        // 파일이 이미 존재해도 멱등(다시 true, 내용 동일).
        assert!(adapter.restore_session_artifact(&path).unwrap());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), original);

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn restore_ignores_non_settings_paths() {
        use crate::observer::ObserverAdapter;
        let dir = scratch_dir();
        std::fs::create_dir_all(&dir).unwrap();
        let adapter = ClaudeAdapter::new(dir.clone(), forwarder_exe());
        let other = dir.join("codex-hook.toml");

        // codex 등 다른 어댑터의 아티팩트는 claude 소관이 아니다 → Ok(false), 미생성.
        assert!(!adapter.restore_session_artifact(&other).unwrap());
        assert!(!other.exists());

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn gc_respects_age_and_filename_filter() {
        use std::time::Duration;
        let dir = scratch_dir();
        std::fs::create_dir_all(&dir).unwrap();
        let settings_a = dir.join("a.settings.json");
        let settings_b = dir.join("b.settings.json");
        let unrelated = dir.join("keep.json");
        for p in [&settings_a, &settings_b, &unrelated] {
            std::fs::write(p, "{}").unwrap();
        }

        // 넉넉한 max_age: 방금 쓴 파일은 전부 살아남는다(살아 있는 세션 보호).
        super::gc_stale_settings(&dir, Duration::from_secs(30 * 24 * 3600));
        assert!(settings_a.exists() && settings_b.exists() && unrelated.exists());

        // max_age=0: 모든 `.settings.json`은 age>0이라 지워지고, 비매칭 파일은
        // 남는다(파일명 필터 검증).
        super::gc_stale_settings(&dir, Duration::ZERO);
        assert!(!settings_a.exists(), "stale settings must be removed");
        assert!(!settings_b.exists(), "stale settings must be removed");
        assert!(unrelated.exists(), "non-settings files must survive");

        // 디렉터리 부재는 조용히 no-op(패닉 없음).
        super::gc_stale_settings(&scratch_dir(), Duration::ZERO);

        let _ = std::fs::remove_dir_all(dir);
    }

    /// CLI 2.1.x는 Task 서브에이전트 대화를 메인 전사가 아니라
    /// `<session>/subagents/*.jsonl`에 쓴다 — 메인만 읽으면 그 몫(실측상 세션
    /// 토큰의 약 2/3)이 통째로 빠진다. Stop이 둘을 합산하고, 두 번째 Stop은
    /// 각 파일의 워터마크 뒤 델타만 싣는지 본다.
    #[test]
    fn claude_stop_sums_subagent_transcripts_and_then_only_their_delta() {
        let dir = scratch_dir();
        let subagents = dir.join("sess-1").join("subagents");
        std::fs::create_dir_all(&subagents).unwrap();
        let transcript = dir.join("sess-1.jsonl");
        let sub = subagents.join("agent-aaa.jsonl");

        let assistant = |id: &str, input: u64, output: u64, sidechain: bool| {
            format!(
                r#"{{"type":"assistant","isSidechain":{sidechain},"message":{{"id":"{id}","model":"claude-opus-5","usage":{{"input_tokens":{input},"output_tokens":{output},"cache_read_input_tokens":0,"cache_creation_input_tokens":0}}}}}}"#
            )
        };
        std::fs::write(
            &transcript,
            [
                r#"{"type":"user","message":{"role":"user","content":"작업"}}"#.to_string(),
                assistant("msg-main-1", 10, 5, false),
            ]
            .join("\n"),
        )
        .unwrap();
        std::fs::write(
            &sub,
            [
                r#"{"type":"user","isSidechain":true,"message":{"role":"user","content":"하위 작업"}}"#.to_string(),
                assistant("msg-sub-1", 100, 50, true),
            ]
            .join("\n"),
        )
        .unwrap();

        let adapter = ClaudeAdapter::new(scratch_dir(), forwarder_exe());
        let body = serde_json::json!({ "transcript_path": transcript.to_string_lossy() })
            .to_string()
            .into_bytes();
        let stop = || {
            adapter.map_hook(&RawObserverHook {
                event_name: "Stop",
                body: &body,
            })
        };

        let Some(ObserverEvent::Stop { tokens, .. }) = stop() else {
            panic!("Stop 이벤트가 나와야 한다");
        };
        let tokens = tokens.expect("메인+서브 사용량");
        assert_eq!(tokens.input, Some(110));
        assert_eq!(tokens.output, Some(55));
        // 대표 모델은 메인 세션 응답의 것(서브는 다른 모델일 수 있다).
        assert_eq!(tokens.model.as_deref(), Some("claude-opus-5"));

        // 파일이 그대로면 두 번째 Stop은 아무것도 세지 않는다.
        let Some(ObserverEvent::Stop { tokens, .. }) = stop() else {
            panic!("Stop 이벤트가 나와야 한다");
        };
        assert_eq!(tokens, None);

        // 서브에이전트만 더 돌았다면 그 델타만 실린다.
        let mut grown = std::fs::read_to_string(&sub).unwrap();
        grown.push('\n');
        grown.push_str(&assistant("msg-sub-2", 7, 3, true));
        std::fs::write(&sub, grown).unwrap();
        let Some(ObserverEvent::Stop { tokens, .. }) = stop() else {
            panic!("Stop 이벤트가 나와야 한다");
        };
        let tokens = tokens.expect("서브 델타");
        assert_eq!(tokens.input, Some(7));
        assert_eq!(tokens.output, Some(3));

        let _ = std::fs::remove_dir_all(dir);
    }
}
