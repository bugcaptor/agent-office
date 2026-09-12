// src-tauri/src/session/external.rs
//
// 외부(논리) 세션: 앱 밖 터미널/tmux pane에서 도는 claude를 캐릭터에 붙인다.
// PTY가 없으므로 화면 미러링도 입력 주입도 없고, 오직 "훅 라우팅 + persona
// 주입"만 제공한다.
//
// 되는 이유는 상태 감지가 PTY와 독립 채널이기 때문이다: forwarder는 셸 env의
// `AGENT_OFFICE_SESSION`+`AGENT_OFFICE_HOOK_URL`만 있으면 어느 셸에서든
// `/hook`으로 POST하고, `NotificationHub`는 그 sid를 `SessionRegistry`로
// 해석해 캐릭터를 찾는다. 즉 등록되지 않은 sid의 이벤트만 버려지므로,
// "레지스트리에 sid를 넣는 논리 세션"이 최소 확장점이다.
//
// manager.rs의 `Session`은 건드리지 않는다(writer/control/waiter를 Option화하는
// 침습 회피) -- 대신 SessionManager에 별도 `externals` 맵을 두고, 이 파일이
// handoff_v1.rs와 같은 `impl SessionManager` 형제 모듈 관례로 확장한다.

use std::path::PathBuf;

use uuid::Uuid;

use super::manager::{cleanup_paths, PreparedPlan, SessionManager};
use crate::observer::ObserverEvent;
use crate::session_events::types::{AgentEventProfile, SessionStartedEvent};
use crate::types::*;

/// 외부 논리 세션 한 건. PTY 자원이 없으므로 훅 라우팅 키(sid)와 정리에
/// 필요한 최소 정보만 든다.
pub(super) struct ExternalSession {
    pub(super) session_id: SessionId,
    /// attach를 요청한 셸의 PID(ctl이 `parent_id()`로 넘긴다). `sweep_externals`가
    /// 이 PID의 생존을 보고 끊어진 세션을 정리한다. None이면 스윕 대상이 아니다.
    pub(super) shell_pid: Option<u32>,
    /// detach 시 지울 observer 아티팩트(claude 훅 settings 파일).
    pub(super) cleanup_paths: Vec<PathBuf>,
    /// 붙인 시각(epoch ms). 진단·표시용.
    #[allow(dead_code)]
    pub(super) attached_at_ms: u64,
    /// 관찰 연결기는 셸 attach와 달리 원본 프로세스에 훅/환경을 주입하지 않는다.
    /// 이 메타데이터는 그 연결의 소유권과 재시도 멱등성을 판별하는 데만 쓴다.
    pub(super) observed: Option<ObservedConnection>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct ObservedConnection {
    pub(super) provider: String,
    pub(super) source_session_id: String,
    pub(super) cwd: String,
    pub(super) owner_id: String,
    pub(super) last_sequence: u64,
    pub(super) last_heartbeat_ms: u64,
    /// Set only by a connector which has verified the source application.
    /// Provider identity alone (Codex/Claude) does not identify its host UI.
    pub(super) focus_target: Option<ObservedFocusTarget>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ObservedFocusTarget {
    VsCode { cwd: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ObservedFocus {
    pub session_id: String,
    pub target: ObservedFocusTarget,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ObservedPersonaConnection {
    pub session_id: String,
    pub provider: String,
    pub source_session_id: String,
    pub cwd: String,
    pub verified_vscode: bool,
}

/// Node 연결기가 등록한 이미 실행 중인 Codex/Claude 세션의 attach 결과.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ObservedAttachOutcome {
    pub session_id: SessionId,
    pub reused: bool,
}

/// 관찰 연결기의 이벤트 종류. 본문/전사 데이터는 이 경계에서 받지 않는다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ObservedEventKind {
    Prompt,
    Tool,
    Stop,
    Attention,
    /// lease만 연장한다. 알림/활동은 방출하지 않는다.
    Heartbeat,
}

/// 외부 세션을 끊는 사유. 방출할 `SessionState`가 여기서 갈린다.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExternalDetachReason {
    /// 사용자/앱이 의도적으로 끊음(ctl detach, dispose, 세션 교체) → Disposed.
    Detach,
    /// 붙어 있던 셸이 사라짐(PID 스윕) → Exited.
    ShellExit,
}

impl ExternalDetachReason {
    fn state(self) -> SessionState {
        match self {
            ExternalDetachReason::Detach => SessionState::Disposed,
            ExternalDetachReason::ShellExit => SessionState::Exited,
        }
    }

    fn intentional(self) -> bool {
        self == ExternalDetachReason::Detach
    }
}

/// `attach_external`의 결과. 어느 쪽이든 호출자(M2의 ctl 핸들러)가 셸
/// 스크립트를 렌더할 수 있게 `plan`(env+wrappers+settings 경로)을 함께 준다.
pub enum ExternalAttachOutcome {
    /// 이 캐릭터의 앱 내 PTY 세션이 살아 있다. 논리 세션을 새로 만들지 않고
    /// **그 세션의 sid**에 합류시킨다(1캐릭터 1세션 불변식) -- tmux pane 안에서
    /// eval하면 앱이 띄운 tmux 클라이언트 세션과 같은 sid로 합쳐지는 합성이
    /// 이 분기다.
    BindExisting {
        session_id: SessionId,
        plan: PreparedPlan,
    },
    /// 새 논리 세션을 발급했다.
    New {
        session_id: SessionId,
        plan: PreparedPlan,
    },
}

impl ExternalAttachOutcome {
    pub fn session_id(&self) -> &str {
        match self {
            ExternalAttachOutcome::BindExisting { session_id, .. }
            | ExternalAttachOutcome::New { session_id, .. } => session_id,
        }
    }

    pub fn plan(&self) -> &PreparedPlan {
        match self {
            ExternalAttachOutcome::BindExisting { plan, .. }
            | ExternalAttachOutcome::New { plan, .. } => plan,
        }
    }

    /// 새 논리 세션인가(false면 기존 in-app 세션에 합류).
    pub fn is_new(&self) -> bool {
        matches!(self, ExternalAttachOutcome::New { .. })
    }
}

impl SessionManager {
    /// 앱 밖 터미널을 캐릭터에 붙인다. 반환된 plan의 env/wrappers를 호출자가
    /// 셸 스크립트로 렌더해 그 터미널에서 eval하면, 그 셸에서 뜬 claude의 훅이
    /// 이 캐릭터의 알림으로 흐른다.
    ///
    /// `shell_pid`는 끊김 감지용(그 셸이 죽으면 `sweep_externals`가 정리한다).
    /// `personality_prompt`는 in-app 세션과 동일하게 claude 래퍼로 주입된다.
    ///
    /// `Result`인 이유는 control 핸들러가 실패를 그대로 `ok:false`로 돌려줄 수
    /// 있게 하기 위함이다(현재 실패 경로는 없다).
    ///
    /// 타임라인 표시용 이름/역할은 `create`와 같은 폴백(이름=agentId, 역할 없음)을
    /// 쓴다. 프로필을 아는 호출자는 `attach_external_with_profile`를 쓴다.
    pub fn attach_external(
        &self,
        agent_id: &str,
        shell_pid: Option<u32>,
        cwd: Option<&str>,
        personality_prompt: Option<&str>,
    ) -> Result<ExternalAttachOutcome, String> {
        let fallback = AgentEventProfile {
            name: agent_id.to_string(),
            role: None,
        };
        self.attach_external_with_profile(agent_id, shell_pid, cwd, personality_prompt, fallback)
    }

    /// `attach_external`에 타임라인/오피스 씬이 쓰는 캐릭터 이름·역할을 실어
    /// 주는 형태(`create` ↔ `create_with_profile`과 같은 관례).
    pub fn attach_external_with_profile(
        &self,
        agent_id: &str,
        shell_pid: Option<u32>,
        cwd: Option<&str>,
        personality_prompt: Option<&str>,
        profile: AgentEventProfile,
    ) -> Result<ExternalAttachOutcome, String> {
        let _lifecycle = self.observed_lifecycle.lock();
        // 앱 내 PTY 세션이 살아 있으면 논리 세션을 만들지 않는다. 같은 sid로
        // plan만 다시 만들어 돌려준다 -- 훅 settings 파일 경로는 sid에서
        // 결정론적으로 나오고 기록은 temp+rename 멱등이라, 이미 있는 파일을
        // 같은 내용으로 다시 쓸 뿐이다(그래서 cleanup 책임도 그 세션에 남는다).
        if let Some(session) = self.find(agent_id) {
            if session.reusable() {
                let plan = self.prepare_session_plan(&session.session_id, personality_prompt);
                return Ok(ExternalAttachOutcome::BindExisting {
                    session_id: session.session_id.clone(),
                    plan,
                });
            }
        }

        // 이미 붙어 있던 외부 세션은 교체 재발급한다. 낡은 sid의 훅은
        // 레지스트리에서 빠져 그냥 폐기되므로(해석 실패=무시) 무해하다.
        self.detach_external(agent_id, ExternalDetachReason::Detach);

        let session_id = Uuid::new_v4().to_string(); // uuid는 URL-safe → hook 라우팅 키로 안전
        let plan = self.prepare_session_plan(&session_id, personality_prompt);

        // 레지스트리 등록이 곧 "훅이 이 캐릭터로 흐른다"는 뜻이다.
        self.registry
            .insert(&session_id, agent_id, SessionState::Running);
        // 타임라인/오피스 씬은 session_started로 세션의 시작을 안다. PTY가
        // 없으므로 shell은 "external"이고, cwd는 앱이 관측할 수 없어 attach를
        // 요청한 쪽(ctl이 자기 작업 폴더를 실어 보낸다)이 알려준 값을 쓴다.
        self.events.session_started(&SessionStartedEvent {
            agent_id: agent_id.to_string(),
            session_id: session_id.clone(),
            agent_name: profile.name,
            agent_role: profile.role,
            cwd: cwd.unwrap_or_default().to_string(),
            shell: "external".into(),
            at: now_ms(),
        });
        self.emit_external_state(agent_id, &session_id, SessionState::Running, None);

        self.externals.lock().insert(
            agent_id.to_string(),
            ExternalSession {
                session_id: session_id.clone(),
                shell_pid,
                cleanup_paths: plan.cleanup_paths.clone(),
                attached_at_ms: now_ms(),
                observed: None,
            },
        );

        Ok(ExternalAttachOutcome::New { session_id, plan })
    }

    /// 외부 세션을 끊는다. 붙어 있지 않았으면 아무것도 하지 않고 false.
    ///
    /// 레지스트리에서 sid를 빼는 순간 그 sid로 오는 훅은 해석 실패로 폐기되고,
    /// 미해결 알림(hub)과 훅 settings 파일도 함께 정리된다.
    pub fn detach_external(&self, agent_id: &str, reason: ExternalDetachReason) -> bool {
        let Some(ext) = self.externals.lock().remove(agent_id) else {
            return false;
        };
        self.finish_external_detach(agent_id, ext, reason);
        true
    }

    fn finish_external_detach(
        &self,
        agent_id: &str,
        ext: ExternalSession,
        reason: ExternalDetachReason,
    ) {
        self.registry.remove(&ext.session_id);
        self.hub.purge_session(&ext.session_id);
        cleanup_paths(&ext.cleanup_paths);
        let state = reason.state();
        self.emit_external_state(
            agent_id,
            &ext.session_id,
            state,
            Some(SessionExitInfo {
                session_id: ext.session_id.clone(),
                // PTY도 자식 프로세스도 없어 종료 코드/시그널은 관측 불가.
                exit_code: None,
                signal: None,
                intentional: reason.intentional(),
            }),
        );
    }

    /// 이미 실행 중인 도구가 내보낸 최소 상태만 관찰하는 연결을 등록한다. 기존
    /// 셸 attach와 달리 PTY 세션에 합류하거나 외부 세션을 교체하지 않는다.
    pub fn attach_observed(
        &self,
        agent_id: &str,
        provider: &str,
        source_session_id: &str,
        cwd: &str,
        owner_id: &str,
        pid: Option<u32>,
        profile: AgentEventProfile,
    ) -> Result<ObservedAttachOutcome, String> {
        self.attach_observed_with_focus(
            agent_id,
            provider,
            source_session_id,
            cwd,
            owner_id,
            pid,
            profile,
            None,
        )
    }

    pub(crate) fn attach_observed_with_focus(
        &self,
        agent_id: &str,
        provider: &str,
        source_session_id: &str,
        cwd: &str,
        owner_id: &str,
        pid: Option<u32>,
        profile: AgentEventProfile,
        focus_target: Option<ObservedFocusTarget>,
    ) -> Result<ObservedAttachOutcome, String> {
        let _lifecycle = self.observed_lifecycle.lock();
        if self
            .find(agent_id)
            .is_some_and(|session| session.reusable())
        {
            return Err("observed-pty-exists".into());
        }

        let mut externals = self.externals.lock();
        if let Some(existing) = externals.get(agent_id) {
            if let Some(observed) = &existing.observed {
                if observed.owner_id == owner_id
                    && observed.provider == provider
                    && observed.source_session_id == source_session_id
                {
                    return Ok(ObservedAttachOutcome {
                        session_id: existing.session_id.clone(),
                        reused: true,
                    });
                }
                return Err("observed-agent-already-attached".into());
            }
            return Err("observed-external-exists".into());
        }
        // source/provider 한 쌍은 캐릭터 하나에만 귀속된다. 이 검사는 위 락 안에
        // 있어 동시 attach도 둘 다 통과하지 못한다.
        if externals.values().any(|existing| {
            existing.observed.as_ref().is_some_and(|observed| {
                observed.provider == provider && observed.source_session_id == source_session_id
            })
        }) {
            return Err("observed-source-already-attached".into());
        }

        // A naturally exited PTY remains in `sessions` for terminal history.
        // Retire that slot only once all attach checks have succeeded: otherwise
        // session_id_for() would keep selecting its old sid over this connection.
        // The lifecycle lock also orders this against its delayed on_exit event.
        if let Some(retired) = self.sessions.lock().remove(agent_id) {
            self.registry.remove(&retired.session_id);
            self.hub.purge_session(&retired.session_id);
        }

        let session_id = Uuid::new_v4().to_string();
        self.registry
            .insert(&session_id, agent_id, SessionState::Running);
        self.events.session_started(&SessionStartedEvent {
            agent_id: agent_id.to_string(),
            session_id: session_id.clone(),
            agent_name: profile.name,
            agent_role: profile.role,
            cwd: cwd.to_string(),
            shell: "observed".into(),
            at: now_ms(),
        });
        self.emit_external_state(agent_id, &session_id, SessionState::Running, None);
        externals.insert(
            agent_id.to_string(),
            ExternalSession {
                session_id: session_id.clone(),
                shell_pid: pid,
                cleanup_paths: vec![],
                attached_at_ms: now_ms(),
                observed: Some(ObservedConnection {
                    provider: provider.to_string(),
                    source_session_id: source_session_id.to_string(),
                    cwd: cwd.to_string(),
                    owner_id: owner_id.to_string(),
                    last_sequence: 0,
                    last_heartbeat_ms: now_ms(),
                    focus_target,
                }),
            },
        );
        Ok(ObservedAttachOutcome {
            session_id,
            reused: false,
        })
    }

    /// Focus uses the live connection's original folder, not an editable
    /// character profile. Detach removes this target with the connection.
    pub(crate) fn observed_focus(&self, agent_id: &str) -> Option<ObservedFocus> {
        let externals = self.externals.lock();
        let session = externals.get(agent_id)?;
        Some(ObservedFocus {
            session_id: session.session_id.clone(),
            target: session.observed.as_ref()?.focus_target.clone()?,
        })
    }

    /// Persona styles are available only to a live connector that verified VS Code.
    pub(crate) fn observed_persona_connection(
        &self,
        agent_id: &str,
    ) -> Option<ObservedPersonaConnection> {
        let externals = self.externals.lock();
        let external = externals.get(agent_id)?;
        let observed = external.observed.as_ref()?;
        Some(ObservedPersonaConnection {
            session_id: external.session_id.clone(),
            provider: observed.provider.clone(),
            source_session_id: observed.source_session_id.clone(),
            cwd: observed.cwd.clone(),
            verified_vscode: matches!(
                observed.focus_target,
                Some(ObservedFocusTarget::VsCode { .. })
            ),
        })
    }

    /// 관찰 이벤트의 소유권·순서를 확인해 기존 NotificationHub로 넘긴다.
    /// false는 중복 sequence라서 조용히 버렸다는 뜻이고, 소유권/세션 불일치는 오류다.
    pub fn ingest_observed_event(
        &self,
        agent_id: &str,
        session_id: &str,
        owner_id: &str,
        sequence: u64,
        kind: ObservedEventKind,
        tool_name: Option<&str>,
    ) -> Result<bool, String> {
        if sequence == 0 {
            return Err("observed-sequence-invalid".into());
        }
        let mut externals = self.externals.lock();
        let Some(external) = externals.get_mut(agent_id) else {
            return Err("observed-not-attached".into());
        };
        if external.session_id != session_id {
            return Err("observed-session-mismatch".into());
        }
        let Some(observed) = external.observed.as_mut() else {
            return Err("observed-not-attached".into());
        };
        if observed.owner_id != owner_id {
            return Err("observed-owner-mismatch".into());
        }
        if sequence <= observed.last_sequence {
            return Ok(false);
        }
        observed.last_sequence = sequence;
        observed.last_heartbeat_ms = now_ms();
        let event = match kind {
            ObservedEventKind::Prompt => Some(ObserverEvent::Prompt {
                text: None,
                cwd: None,
            }),
            ObservedEventKind::Tool => Some(ObserverEvent::Tool {
                text: tool_name.map(str::to_owned),
                assistant: None,
                tokens: None,
            }),
            ObservedEventKind::Stop => Some(ObserverEvent::Stop {
                message: None,
                running: None,
                tokens: None,
            }),
            ObservedEventKind::Attention => Some(ObserverEvent::Attention { message: None }),
            ObservedEventKind::Heartbeat => None,
        };
        if let Some(event) = event {
            // 같은 연결의 다음 event가 앞질러 hub에 도달하지 않게 외부 맵 락을
            // 유지한다. hub는 externals를 다시 잠그지 않는다.
            self.hub.ingest_observer(session_id, event);
        }
        Ok(true)
    }

    /// stale detach가 새 연결을 끊지 않게 현재 sid와 owner를 함께 확인한다.
    pub fn detach_observed(&self, agent_id: &str, session_id: &str, owner_id: &str) -> bool {
        let _lifecycle = self.observed_lifecycle.lock();
        let detached = {
            let mut externals = self.externals.lock();
            let matches = externals.get(agent_id).is_some_and(|external| {
                external.session_id == session_id
                    && external
                        .observed
                        .as_ref()
                        .is_some_and(|observed| observed.owner_id == owner_id)
            });
            matches.then(|| externals.remove(agent_id)).flatten()
        };
        let Some(external) = detached else {
            return false;
        };
        self.finish_external_detach(agent_id, external, ExternalDetachReason::Detach);
        true
    }

    /// 붙여 둔 셸이 사라진 외부 세션을 정리한다. `lib.rs`가 5초 주기로 부른다.
    ///
    /// 셸 종료를 EXIT trap으로 알리는 방식은 사용자의 기존 trap을 덮어쓸 위험이
    /// 있어 기각했고, 대신 `kill(pid, 0)` 폴링으로 감지한다(unix 전용).
    #[cfg(unix)]
    pub fn sweep_externals(&self) {
        let _lifecycle = self.observed_lifecycle.lock();
        let dead: Vec<(AgentId, SessionId)> = {
            let map = self.externals.lock();
            map.iter()
                .filter(|(_, ext)| {
                    let observed_expired = ext.observed.as_ref().is_some_and(|observed| {
                        now_ms().saturating_sub(observed.last_heartbeat_ms) > 30_000
                    });
                    observed_expired || matches!(ext.shell_pid, Some(pid) if !pid_alive(pid))
                })
                .map(|(agent_id, ext)| (agent_id.clone(), ext.session_id.clone()))
                .collect()
        };
        for (agent_id, session_id) in dead {
            let detached = {
                let mut externals = self.externals.lock();
                (externals
                    .get(&agent_id)
                    .is_some_and(|ext| ext.session_id == session_id))
                .then(|| externals.remove(&agent_id))
                .flatten()
            };
            if let Some(external) = detached {
                self.finish_external_detach(&agent_id, external, ExternalDetachReason::ShellExit);
            }
        }
    }

    /// 비unix에서는 PID 생존 확인은 하지 않지만, 관찰 연결 lease는 모든
    /// 플랫폼에서 만료시킨다. Node 연결기가 사라져 detach를 못 보내도 UI가
    /// 영구 Running으로 남지 않게 하는 최소 보장이다.
    #[cfg(not(unix))]
    pub fn sweep_externals(&self) {
        let _lifecycle = self.observed_lifecycle.lock();
        let dead: Vec<(AgentId, SessionId)> = {
            let map = self.externals.lock();
            map.iter()
                .filter(|(_, ext)| {
                    ext.observed.as_ref().is_some_and(|observed| {
                        now_ms().saturating_sub(observed.last_heartbeat_ms) > 30_000
                    })
                })
                .map(|(agent_id, ext)| (agent_id.clone(), ext.session_id.clone()))
                .collect()
        };
        for (agent_id, session_id) in dead {
            let detached = {
                let mut externals = self.externals.lock();
                (externals
                    .get(&agent_id)
                    .is_some_and(|ext| ext.session_id == session_id))
                .then(|| externals.remove(&agent_id))
                .flatten()
            };
            if let Some(external) = detached {
                self.finish_external_detach(&agent_id, external, ExternalDetachReason::ShellExit);
            }
        }
    }

    /// 외부 세션의 `session-state` 방출. `external: Some(true)`로 표시해
    /// 프런트가 PTY 터미널 대신 placeholder를 그릴 수 있게 한다(M4).
    fn emit_external_state(
        &self,
        agent_id: &str,
        session_id: &str,
        state: SessionState,
        exit: Option<SessionExitInfo>,
    ) {
        self.events.session_state(&SessionStateEvent {
            session_id: session_id.to_string(),
            agent_id: agent_id.to_string(),
            state,
            exit,
            at: now_ms(),
            external: Some(true),
        });
    }
}

/// 프로세스가 살아 있는가. `kill(pid, 0)`은 시그널을 보내지 않고 존재/권한만
/// 검사한다 -- ESRCH일 때만 "없음"이고, EPERM(다른 사용자 소유)은 살아 있다는
/// 뜻이므로 살아 있는 것으로 본다.
#[cfg(unix)]
fn pid_alive(pid: u32) -> bool {
    if pid == 0 {
        // 0은 "같은 프로세스 그룹 전체"라는 특수 의미라 생존 검사에 쓸 수 없다.
        return false;
    }
    if unsafe { libc::kill(pid as libc::pid_t, 0) } == 0 {
        return true;
    }
    std::io::Error::last_os_error().raw_os_error() != Some(libc::ESRCH)
}

/// epoch(UTC) 밀리초. manager.rs의 동명 함수와 같은 기준(벽시계)이다 --
/// 이벤트 타임스탬프와 attach 시각에만 쓰므로 단조성은 필요 없다.
fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests;
