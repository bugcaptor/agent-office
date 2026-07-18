# 서브시스템 A 상세 설계 (Rust / Tauri v2) — 세션 · 알림 · 영속화

> 설계: Opus 하위 설계 / 주요 판단: Fable. **Tauri v2 개정판** — 원본 Electron 설계는 `docs/design/archive/subsystem-a-sessions-electron.md`에 보존(상태 머신·dedup·배칭·훅 메커니즘의 논리를 1:1 이식). 계약 정합화(R1~R6)는 마스터 플랜(`docs/superpowers/plans/2026-07-06-agent-office.md`)이 최우선. `src/shared/types.ts`가 TS 단일 소스이고, Rust `types.rs`는 serde로 미러링한다.

작성 범위: 모듈 레이아웃 · 동시성 모델, Rust 타입(serde), SessionManager/OutputBatcher/NotificationHub/HookServer/HookSettingsWriter/Tauri 커맨드/프런트 어댑터 전체 코드, 엣지 케이스 E1~E9, 테스트 시드, 태스크 분해.

---

## 0. 파일 레이아웃 & 동시성 모델

### 0.1 `src-tauri/` 모듈 레이아웃

```
src-tauri/
  Cargo.toml
  build.rs
  tauri.conf.json
  src/
    main.rs                     # 얇은 진입점: agent_office_lib::run()
    lib.rs                      # run(): Builder + setup + invoke_handler + RunEvent
    state.rs                    # AppState, AppEvents 트레잇, TauriEvents, SessionRegistry
    types.rs                    # 모든 serde 계약 타입 (types.ts 미러)
    session/
      mod.rs
      manager.rs                # SessionManager (PTY 라이프사이클 소유)
      pty_factory.rs            # PtyFactory 트레잇 + PortablePtyFactory + (cfg test) FakePty
      output_batcher.rs         # PTY→IPC 백프레셔 배칭 (UTF-8 경계 캐리 포함)
    notification/
      mod.rs
      hub.rs                    # NotificationHub (dedup/큐/clear) + Clock 트레잇
      hook_server.rs            # axum 로컬 HTTP 서버 (POST /hook)
      hook_settings.rs          # per-session settings.json 생성/정리
    persistence/
      mod.rs
      profile_store.rs          # PersistedState JSON 영속화
    ipc/
      mod.rs
      commands.rs               # #[tauri::command] 함수들
```

### 0.2 동시성 모델 (정밀 규정)

Electron은 단일 이벤트 루프였지만 Rust/Tauri는 **OS 스레드 + tokio 태스크 혼합**이다. `portable-pty`의 리더는 **블로킹 I/O**(`Read`)라 async로 감쌀 수 없으므로 세션당 전용 스레드가 필요하다.

세션 1개당 자원:

| 자원 | 종류 | 역할 |
|---|---|---|
| **reader thread** | `std::thread` (블로킹) | master PTY에서 raw 바이트를 blocking `read` → `tokio::sync::mpsc::UnboundedSender<ReaderMsg>`로 전달. EOF면 `ReaderMsg::Eof` 후 종료. |
| **output pump task** | `tokio::task` | 위 채널을 수신, `OutputBatcher` 소유. 16ms 데드라인(`sleep_until`) + 64KB 상한으로 코얼레싱, `FlushSink`(Channel)로 방출. BEL(0x07) 감지 시 `hub.on_bell`. |
| **wait thread** | `std::thread` (블로킹) | `child.wait()`(블로킹) → `ExitOutcome`. `kill_requested`로 intentional 판정, `Exited`/`Disposed` 전이 이벤트 방출. |
| **PTY writer** | `Mutex<Box<dyn Write + Send>>` | 커맨드 스레드에서 짧게 락 잡고 stdin 주입. |

전역 자원:

- **hook server**: tokio 태스크 1개(axum). `AppHandle`을 통해 어느 스레드에서든 이벤트 emit(Send+Sync).
- **AppState**: Tauri `Manager::manage`로 등록, 커맨드는 `tauri::State<'_, AppState>`로 접근. **핫 패스(PTY 출력)는 전역 락을 절대 잡지 않는다** — 각 세션의 출력은 자기 `OutputSink`(Channel)로 직접 흐른다.
- **SessionRegistry**: `sid → (agentId, state)`의 `RwLock<HashMap>`. SessionManager가 쓰고 NotificationHub가 읽어 **순환 의존을 끊는다**(훅 라우팅 `resolveAgent`).

```
[PTY master] ──read(blocking)──> reader thread ──mpsc::Data(bytes)──> output pump task
                                                                        │  OutputBatcher(16ms/64KB, seq, utf8 carry)
                                                                        ├─ 0x07 감지 → hub.on_bell()
                                                                        └─ FlushSink → tauri::ipc::Channel<OutputChunk> → [webview]
[PTY child] ──wait(blocking)──> wait thread ── AppEvents.session_state() ──emit "session-state"──> [webview]
[claude curl] ──POST /hook──> axum task ── hub.ingest() ── AppEvents.notification_new() ──emit "notification-new"──> [webview]
```

---

## 1. Rust 계약 타입 — `src-tauri/src/types.rs`

**필드 매핑 규칙**: 구조체는 `#[serde(rename_all = "camelCase")]`로 Rust `snake_case` → TS `camelCase`. enum은 `#[serde(rename_all = "lowercase")]`로 PascalCase variant → TS 소문자 문자열 값. epoch ms는 `u64`(TS `number`), `Option<T>`는 `T | undefined`이며 `skip_serializing_if`로 생략. **`types.ts`가 정본이고 이 파일은 미러**이므로 두 파일의 이 규칙이 어긋나면 안 된다(태스크 완료 기준에 왕복 스냅샷 테스트 포함, §5·§6).

```rust
// src-tauri/src/types.rs
use serde::{Deserialize, Serialize};

pub type AgentId = String;
pub type SessionId = String;

/// 세션 라이프사이클 상태. TS SessionState('starting'|'running'|'exited'|'disposed')와 동일.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SessionState {
    Starting,
    Running,
    Exited,
    Disposed,
}

/// 세션 종료 사유. Exited/Disposed 전이 시 동반.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionExitInfo {
    pub session_id: SessionId,
    /// portable-pty ExitStatus.exit_code()를 i32로. (§4.4 시그널 주석 참조)
    pub exit_code: Option<i32>,
    /// portable-pty는 크로스플랫폼 ExitStatus에서 시그널을 분리 노출하지 않는다 → 항상 None.
    pub signal: Option<i32>,
    /// true=앱이 의도적으로 kill(dispose/quit), false=예기치 않은 종료.
    pub intentional: bool,
}

/// 세션 상태 전이 브로드캐스트. 이벤트 "session-state".
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionStateEvent {
    pub session_id: SessionId,
    pub agent_id: AgentId,
    pub state: SessionState,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub exit: Option<SessionExitInfo>,
    pub at: u64,
}

/// 알림 출처. TS NotificationSource와 동일.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum NotificationSource {
    Hook,
    Stop,
    Bell,
}

impl NotificationSource {
    /// dedupKey 계산용 안정 문자열.
    pub fn as_key(self) -> &'static str {
        match self {
            NotificationSource::Hook => "hook",
            NotificationSource::Stop => "stop",
            NotificationSource::Bell => "bell",
        }
    }
}

/// 정규화된 알림 이벤트. hook POST/BEL 모두 이 형태로 수렴. 이벤트 "notification-new".
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NotificationEvent {
    pub id: String, // uuid v4, NotificationHub가 발급 (R4: 렌더러 재발급 금지)
    pub session_id: SessionId,
    pub agent_id: AgentId,
    pub source: NotificationSource,
    pub message: String,
    pub dedup_key: String,
    pub at: u64,
}

/// renderer→backend 세션 생성 옵션. 프런트 AgentOfficeApi.createSession(agentId, opts?).
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateSessionRequest {
    pub agent_id: AgentId,
    pub cols: Option<u16>,
    pub rows: Option<u16>,
    pub cwd: Option<String>,
    /// 동결 API opts에는 없음 → 프런트 어댑터는 항상 미지정(=기본값). 아키텍트 결정(변경): 기본 false —
    /// 세션은 빈 로그인 셸로 시작하고, 사용자가 `claude --settings "$AGENT_OFFICE_SETTINGS"`로 직접 기동한다.
    pub autostart_claude: Option<bool>,
}

/// createSession 응답.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CreateSessionResult {
    pub session_id: SessionId,
    pub state: SessionState,
}

/// PTY 출력 청크(배치). backend→webview, tauri::ipc::Channel로 전송.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct OutputChunk {
    pub session_id: SessionId,
    pub agent_id: AgentId, // R1: 렌더러 필터링용
    pub data: String,      // UTF-8. OutputBatcher가 이어붙인 결과(경계 캐리 처리됨)
    pub frames: u32,       // 담은 원본 read 이벤트 수(진단용)
    pub seq: u64,          // 세션별 단조 증가
}

/// 알림 클리어됨 브로드캐스트. 이벤트 "notification-cleared".
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NotificationClearedEvent {
    pub agent_id: AgentId,
    pub ids: Vec<String>,
}

/// R5 프로필 스키마(단일 정의).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentProfile {
    pub id: String,
    pub name: String,
    pub role: String,
    pub note: String,
    pub seed: String,
    pub created_at: u64,
    pub desk_index: u32,
}

/// R5 영속 상태. version은 리터럴 1.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PersistedState {
    pub agents: Vec<AgentProfile>,
    pub version: u32,
}

impl PersistedState {
    pub fn empty() -> Self {
        Self { agents: Vec::new(), version: 1 }
    }
}

/// epoch ms 헬퍼.
pub fn now_ms() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis() as u64
}
```

> **`list_notifications` 반환 형태 주의**: 동결 API는 `listNotifications(agentId): Promise<NotificationEvent[]>` — **배열**이다. Electron 설계의 `NotificationSnapshot`은 IPC 표면에서 제거하고, Hub 내부 조회 결과 `Vec<NotificationEvent>`(pending 큐)만 커맨드로 반환한다.

---

## 2. 렌더러 경계 매핑 — Tauri 커맨드 & 이벤트 (정확한 문자열)

동결 `AgentOfficeApi`를 아래 커맨드/이벤트로 매핑한다. **모든 커맨드는 `#[tauri::command(rename_all = "camelCase")]`** 로 선언해 JS는 `{ agentId, ... }` 카멜케이스로 인자를 넘긴다.

### 2.1 커맨드 (invoke) — 정확한 이름

| AgentOfficeApi | invoke 커맨드명 | 인자(JS) | 반환 |
|---|---|---|---|
| `createSession` | `"create_session"` | `{ agentId, opts? }` | `CreateSessionResult` |
| `disposeSession` | `"dispose_session"` | `{ agentId }` | `void` |
| `writeInput` | `"write_input"` | `{ agentId, data }` | `void`(fire-and-forget) |
| `resize` | `"resize_session"` | `{ agentId, cols, rows }` | `void` |
| `clearNotifications` | `"clear_notifications"` | `{ agentId, ids? }` | `void` |
| `listNotifications` | `"list_notifications"` | `{ agentId }` | `NotificationEvent[]` |
| `loadState` | `"load_state"` | `{}` | `PersistedState` |
| `saveState` | `"save_state"` | `{ state }` | `void` |
| `setBadgeCount` | `"set_badge_count"` | `{ count }` | `void` |
| `onData` (내부) | `"subscribe_output"` / `"unsubscribe_output"` | `{ agentId, channel }` / `{ agentId }` | `void` |

### 2.2 이벤트 (emit/listen) — 정확한 이름

| AgentOfficeApi 구독 | 전송 방식 | 이름 | 페이로드 |
|---|---|---|---|
| `onData(agentId, cb)` | **`tauri::ipc::Channel<OutputChunk>`** | (커맨드 인자로 전달) | `OutputChunk` |
| `onSessionState` | 이벤트 emit | `"session-state"` | `SessionStateEvent` |
| `onNotification` | 이벤트 emit | `"notification-new"` | `NotificationEvent` |
| `onNotificationCleared` | 이벤트 emit | `"notification-cleared"` | `NotificationClearedEvent` |

### 2.3 PTY 출력 전송 방식 결정 — Channel 채택 (근거)

**결론: PTY 출력(최다 트래픽)은 Tauri v2 `Channel<OutputChunk>`, 나머지 저빈도 신호는 전역 이벤트.**

- **Channel 채택 근거**: (1) Channel은 특정 스트림 전용 fast-path로 **순서 보장** + JSON 브로드캐스트/전역 리스너 깨움 오버헤드 회피. (2) `OutputBatcher`가 이미 16ms/64KB로 빈도를 세션당 ~60/s로 낮췄고, Channel은 그 위에서 백프레셔에 유리. (3) `onData(agentId, cb)`는 구독 모델이지만, Channel도 `subscribe_output(agentId, channel)` 커맨드 + 반환 unsubscribe(`unsubscribe_output`)로 자연스럽게 감싼다.
- **다중 구독자/조기 출력 처리**: 동결 API는 같은 agentId에 `onData`를 여러 번 부를 수 있다. 백엔드는 **agentId당 Channel 하나**만 두고, **어댑터(JS)가 콜백 Set으로 팬아웃**(§3.7). 또한 Channel 등록 이전 출력은 백엔드 `OutputSink`가 소량(256청크) **백로그**에 담았다가 `attach` 시 드레인 → 조기 출력 유실 방지.
- **이벤트를 안 쓰는 이유**: 전역 이벤트는 broadcast + JS측 필터가 필요하고 고빈도에서 낭비. 단, 세션 상태/알림은 **여러 리스너**(오피스 씬 + 티커 + 배지)가 듣는 저빈도 신호라 이벤트가 적합.
- 이 전송 방식은 `AgentOfficeApi`에 노출되지 않으므로, 추후 이벤트로 교체해도 계약 무변경.

---

## 3. 핵심 코드 스켈레톤

### 3.1 PtyFactory (테스트 주입점) — `session/pty_factory.rs`

```rust
// src-tauri/src/session/pty_factory.rs
use std::io::{self, Read, Write};
use std::sync::{Arc, Mutex};

/// PTY spawn 결과 번들. 리더/라이터/제어/대기를 분리해 페이크 주입을 쉽게 한다.
pub struct SpawnedPty {
    pub reader: Box<dyn Read + Send>,
    pub writer: Box<dyn Write + Send>,
    pub control: Arc<dyn PtyControl>, // resize + kill (여러 스레드 공유)
    pub waiter: Box<dyn PtyWaiter>,   // 블로킹 wait, 소유 이전
}

pub trait PtyControl: Send + Sync {
    fn resize(&self, cols: u16, rows: u16) -> io::Result<()>;
    fn kill(&self) -> io::Result<()>;
}

pub trait PtyWaiter: Send {
    /// 블로킹. 프로세스 종료까지 대기 후 결과 반환.
    fn wait(self: Box<Self>) -> ExitOutcome;
}

#[derive(Debug, Clone, Copy)]
pub struct ExitOutcome {
    pub exit_code: Option<i32>,
    pub signal: Option<i32>,
}

pub struct PtySpawnOptions {
    pub shell: String,
    pub args: Vec<String>,
    pub cols: u16,
    pub rows: u16,
    pub cwd: String,
    pub env: Vec<(String, String)>,
}

/// 부작용 경계. SessionManager는 이 트레잇만 안다. 테스트는 FakePtyFactory 주입.
pub trait PtyFactory: Send + Sync {
    fn spawn(&self, opts: PtySpawnOptions) -> io::Result<SpawnedPty>;
}

// ── 실제 portable-pty 구현 ─────────────────────────────────────────────
use portable_pty::{native_pty_system, CommandBuilder, MasterPty, PtySize};

pub struct PortablePtyFactory;

struct RealControl {
    master: Mutex<Box<dyn MasterPty + Send>>,
    killer: Mutex<Box<dyn portable_pty::ChildKiller + Send + Sync>>,
}
impl PtyControl for RealControl {
    fn resize(&self, cols: u16, rows: u16) -> io::Result<()> {
        self.master.lock().unwrap()
            .resize(PtySize { rows, cols, pixel_width: 0, pixel_height: 0 })
            .map_err(|e| io::Error::new(io::ErrorKind::Other, e.to_string()))
    }
    fn kill(&self) -> io::Result<()> {
        self.killer.lock().unwrap().kill()
    }
}

struct RealWaiter {
    child: Box<dyn portable_pty::Child + Send + Sync>,
}
impl PtyWaiter for RealWaiter {
    fn wait(mut self: Box<Self>) -> ExitOutcome {
        match self.child.wait() {
            Ok(status) => ExitOutcome { exit_code: Some(status.exit_code() as i32), signal: None },
            Err(_) => ExitOutcome { exit_code: None, signal: None },
        }
    }
}

impl PtyFactory for PortablePtyFactory {
    fn spawn(&self, o: PtySpawnOptions) -> io::Result<SpawnedPty> {
        let sys = native_pty_system();
        let pair = sys
            .openpty(PtySize { rows: o.rows, cols: o.cols, pixel_width: 0, pixel_height: 0 })
            .map_err(to_io)?;

        let mut cmd = CommandBuilder::new(&o.shell);
        for a in &o.args {
            cmd.arg(a);
        }
        cmd.cwd(&o.cwd);
        // portable-pty CommandBuilder는 기본적으로 부모 env를 상속한다. 우리는 override만 얹는다.
        for (k, v) in &o.env {
            cmd.env(k, v);
        }

        let child = pair.slave.spawn_command(cmd).map_err(to_io)?;
        drop(pair.slave); // slave는 spawn 후 즉시 닫는다(권장).

        let reader = pair.master.try_clone_reader().map_err(to_io)?;
        let writer = pair.master.take_writer().map_err(to_io)?;
        let killer = child.clone_killer(); // wait 스레드가 child를 소유해도 별도로 kill 가능
        let control = Arc::new(RealControl {
            master: Mutex::new(pair.master),
            killer: Mutex::new(killer),
        });
        let waiter = Box::new(RealWaiter { child });
        Ok(SpawnedPty { reader, writer, control, waiter })
    }
}

fn to_io(e: impl std::fmt::Display) -> io::Error {
    io::Error::new(io::ErrorKind::Other, e.to_string())
}
```

### 3.2 OutputBatcher — `session/output_batcher.rs`

```rust
// src-tauri/src/session/output_batcher.rs
use crate::types::OutputChunk;

pub const MAX_BYTES: usize = 65_536; // 64KB
pub const WINDOW_MS: u64 = 16;       // ≈60fps

/// 배치 방출 싱크(테스트 주입점). 프로덕션은 OutputSink(Channel), 테스트는 Vec 수집.
pub trait FlushSink: Send + Sync {
    fn emit(&self, chunk: OutputChunk);
}

/// 순수 배칭 로직. 타이밍(16ms 데드라인)은 output pump task가 소유하고,
/// 이 구조체는 push/flush만 담당해 결정론적으로 테스트된다.
pub struct OutputBatcher {
    session_id: String,
    agent_id: String,
    buf: Vec<u8>,
    frames: u32,
    seq: u64,
}

impl OutputBatcher {
    pub fn new(session_id: String, agent_id: String) -> Self {
        Self { session_id, agent_id, buf: Vec::new(), frames: 0, seq: 0 }
    }

    pub fn pending_bytes(&self) -> usize {
        self.buf.len()
    }

    pub fn push(&mut self, bytes: &[u8]) {
        self.buf.extend_from_slice(bytes);
        self.frames += 1;
    }

    /// 시간창/크기 도달 시 방출. UTF-8 경계에서 끊긴 후미 바이트는 다음 flush로 캐리한다.
    pub fn flush(&mut self, sink: &dyn FlushSink) {
        self.flush_inner(sink, false);
    }

    /// exit/dispose 시 잔여 강제 방출(마지막 출력 유실 방지). 불완전 후미는 lossy 변환.
    pub fn flush_final(&mut self, sink: &dyn FlushSink) {
        self.flush_inner(sink, true);
    }

    fn flush_inner(&mut self, sink: &dyn FlushSink, final_: bool) {
        if self.buf.is_empty() {
            return;
        }
        let take = if final_ { self.buf.len() } else { valid_utf8_prefix(&self.buf) };
        if take == 0 {
            // 버퍼 전체가 불완전 멀티바이트 선두 → 비-final이면 더 기다린다.
            return;
        }
        let bytes: Vec<u8> = self.buf.drain(..take).collect();
        let data = if final_ {
            String::from_utf8_lossy(&bytes).into_owned()
        } else {
            // valid_utf8_prefix가 보장한 유효 구간이므로 안전.
            String::from_utf8(bytes).expect("valid utf8 prefix")
        };
        let chunk = OutputChunk {
            session_id: self.session_id.clone(),
            agent_id: self.agent_id.clone(),
            data,
            frames: self.frames,
            seq: self.seq,
        };
        self.seq += 1;
        self.frames = 0;
        sink.emit(chunk);
    }
}

/// 유효한 UTF-8 접두 길이. 끝에서 잘린 멀티바이트는 제외해 다음 배치로 넘긴다.
fn valid_utf8_prefix(b: &[u8]) -> usize {
    match std::str::from_utf8(b) {
        Ok(_) => b.len(),
        Err(e) => e.valid_up_to(),
    }
}
```

### 3.3 SessionManager — `session/manager.rs`

```rust
// src-tauri/src/session/manager.rs
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::io::{Read, Write};

use tauri::ipc::Channel;
use uuid::Uuid;

use crate::types::*;
use crate::state::{AppEvents, SessionRegistry};
use crate::notification::hub::NotificationHub;
use crate::notification::hook_settings::HookSettingsWriter;
use crate::session::output_batcher::{FlushSink, OutputBatcher, MAX_BYTES, WINDOW_MS};
use crate::session::pty_factory::{PtyFactory, PtySpawnOptions, PtyControl, ExitOutcome};

const BACKLOG_CAP: usize = 256;

enum ReaderMsg {
    Data(Vec<u8>),
    Eof,
}

/// agentId당 출력 Channel + 등록 이전 백로그. FlushSink 구현체.
pub struct OutputSink {
    channel: Mutex<Option<Channel<OutputChunk>>>,
    backlog: Mutex<std::collections::VecDeque<OutputChunk>>,
}
impl OutputSink {
    fn new() -> Self {
        Self { channel: Mutex::new(None), backlog: Mutex::new(Default::default()) }
    }
    fn attach(&self, ch: Channel<OutputChunk>) {
        // 락 순서 항상 channel → backlog (데드락 방지, emit과 동일 순서).
        let mut c = self.channel.lock().unwrap();
        let mut b = self.backlog.lock().unwrap();
        for chunk in b.drain(..) {
            let _ = ch.send(chunk);
        }
        *c = Some(ch);
    }
    fn detach(&self) {
        *self.channel.lock().unwrap() = None;
    }
}
impl FlushSink for OutputSink {
    fn emit(&self, chunk: OutputChunk) {
        let c = self.channel.lock().unwrap();
        if let Some(ch) = c.as_ref() {
            let _ = ch.send(chunk); // Channel 전송 실패(웹뷰 소멸)는 무시
        } else {
            let mut b = self.backlog.lock().unwrap();
            if b.len() >= BACKLOG_CAP {
                b.pop_front();
            }
            b.push_back(chunk);
        }
    }
}

struct Session {
    session_id: SessionId,
    agent_id: AgentId,
    state: Mutex<SessionState>,
    writer: Mutex<Box<dyn Write + Send>>,
    control: Arc<dyn PtyControl>,
    settings_path: std::path::PathBuf,
    kill_requested: AtomicBool,
    output: Arc<OutputSink>,
}

pub struct SessionManager {
    factory: Arc<dyn PtyFactory>,
    hook_writer: HookSettingsWriter, // Clone 가능(PathBuf만 보유)
    registry: Arc<SessionRegistry>,
    events: Arc<dyn AppEvents>,
    hub: Arc<NotificationHub>,
    sessions: Mutex<HashMap<AgentId, Arc<Session>>>,
    get_hook_port: Arc<dyn Fn() -> u16 + Send + Sync>,
    shell_resolver: Arc<dyn Fn() -> (String, Vec<String>) + Send + Sync>,
}

impl SessionManager {
    pub fn new(
        factory: Arc<dyn PtyFactory>,
        hook_writer: HookSettingsWriter,
        registry: Arc<SessionRegistry>,
        events: Arc<dyn AppEvents>,
        hub: Arc<NotificationHub>,
        get_hook_port: Arc<dyn Fn() -> u16 + Send + Sync>,
    ) -> Self {
        Self {
            factory, hook_writer, registry, events, hub,
            sessions: Mutex::new(HashMap::new()),
            get_hook_port,
            shell_resolver: Arc::new(default_shell),
        }
    }

    fn find(&self, agent_id: &str) -> Option<Arc<Session>> {
        self.sessions.lock().unwrap().get(agent_id).cloned()
    }

    pub fn session_id_for(&self, agent_id: &str) -> Option<SessionId> {
        self.find(agent_id).map(|s| s.session_id.clone())
    }

    /// 1 에이전트 1 세션 불변식. self: &Arc<Self>로 wait 스레드에 소유 이전.
    pub fn create(self: &Arc<Self>, req: CreateSessionRequest) -> Result<CreateSessionResult, String> {
        // E9: 살아있는 세션이 있으면 재사용, 새 PTY 안 만듦.
        if let Some(s) = self.find(&req.agent_id) {
            let st = *s.state.lock().unwrap();
            if matches!(st, SessionState::Running | SessionState::Starting) {
                return Ok(CreateSessionResult { session_id: s.session_id.clone(), state: st });
            }
        }

        let session_id = Uuid::new_v4().to_string(); // uuid는 URL-safe → hook 라우팅 키로 안전
        let port = (self.get_hook_port)();
        let settings_path = self.hook_writer.write(&session_id, port).map_err(|e| e.to_string())?;

        let (shell, base_args) = (self.shell_resolver)();
        let cwd = req.cwd.clone().unwrap_or_else(home_dir);
        let spawned = self.factory.spawn(PtySpawnOptions {
            shell,
            args: base_args,
            cols: req.cols.unwrap_or(80),
            rows: req.rows.unwrap_or(24),
            cwd,
            env: vec![
                ("AGENT_OFFICE_SESSION".into(), session_id.clone()),
                ("AGENT_OFFICE_HOOK_URL".into(), format!("http://127.0.0.1:{port}/hook")),
                // 사용자가 수동으로 `claude --settings "$AGENT_OFFICE_SETTINGS"`를 실행할 수 있게 경로 노출.
                ("AGENT_OFFICE_SETTINGS".into(), settings_path.to_string_lossy().into_owned()),
                ("TERM".into(), "xterm-256color".into()),
            ],
        }).map_err(|e| e.to_string())?;

        let output = Arc::new(OutputSink::new());
        let session = Arc::new(Session {
            session_id: session_id.clone(),
            agent_id: req.agent_id.clone(),
            state: Mutex::new(SessionState::Starting),
            writer: Mutex::new(spawned.writer),
            control: spawned.control,
            settings_path,
            kill_requested: AtomicBool::new(false),
            output: output.clone(),
        });

        self.sessions.lock().unwrap().insert(req.agent_id.clone(), session.clone());
        self.registry.insert(&session_id, &req.agent_id, SessionState::Starting);
        self.emit_state(&session, SessionState::Starting, None);

        // 1) reader thread (블로킹 read → mpsc)
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<ReaderMsg>();
        let mut reader = spawned.reader;
        std::thread::spawn(move || {
            let mut buf = [0u8; 8192];
            loop {
                match reader.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => {
                        if tx.send(ReaderMsg::Data(buf[..n].to_vec())).is_err() {
                            break;
                        }
                    }
                    Err(_) => break,
                }
            }
            let _ = tx.send(ReaderMsg::Eof);
        });

        // 2) output pump task (배칭 + BEL 감지 + Channel 방출)
        spawn_output_pump(session_id.clone(), req.agent_id.clone(), rx, output, self.hub.clone());

        // 3) wait thread (블로킹 wait → 상태 전이)
        let me = Arc::clone(self);
        let sess = session.clone();
        let waiter = spawned.waiter;
        std::thread::spawn(move || {
            let outcome = waiter.wait();
            me.on_exit(&sess, outcome);
        });

        // Running 전이
        *session.state.lock().unwrap() = SessionState::Running;
        self.registry.set_state(&session_id, SessionState::Running);
        self.emit_state(&session, SessionState::Running, None);

        // autostart(기본 false): 세션은 기본적으로 빈 로그인 셸만 띄운다. 사용자가
        // `claude --settings "$AGENT_OFFICE_SETTINGS"`로 직접 기동한다. 명시적으로
        // Some(true)를 요청한 경우에만 stdin 주입.
        if req.autostart_claude.unwrap_or(false) {
            let line = format!("claude --settings \"{}\"\n", session.settings_path.display());
            let _ = session.writer.lock().unwrap().write_all(line.as_bytes());
        }

        Ok(CreateSessionResult { session_id, state: SessionState::Running })
    }

    pub fn write_input(&self, agent_id: &str, data: &str) {
        if let Some(s) = self.find(agent_id) {
            if *s.state.lock().unwrap() == SessionState::Running {
                let _ = s.writer.lock().unwrap().write_all(data.as_bytes());
            }
        }
    }

    pub fn resize(&self, agent_id: &str, cols: u16, rows: u16) {
        if let Some(s) = self.find(agent_id) {
            if *s.state.lock().unwrap() == SessionState::Running {
                let _ = s.control.resize(cols, rows);
            }
        }
    }

    /// 의도적 종료. 최종 Disposed 전이는 wait 스레드의 on_exit에서 확정.
    pub fn dispose(&self, agent_id: &str) {
        if let Some(s) = self.find(agent_id) {
            s.kill_requested.store(true, Ordering::SeqCst);
            let _ = s.control.kill();
            self.hook_writer.cleanup(&s.session_id);
        }
    }

    /// 앱 quit: 모든 PTY kill + settings 정리(동기, 빠름).
    pub fn dispose_all(&self) {
        let ids: Vec<AgentId> = self.sessions.lock().unwrap().keys().cloned().collect();
        for a in ids {
            self.dispose(&a);
        }
    }

    /// subscribe_output 커맨드가 호출: agentId에 Channel 등록(+백로그 드레인).
    pub fn attach_output(&self, agent_id: &str, channel: Channel<OutputChunk>) {
        if let Some(s) = self.find(agent_id) {
            s.output.attach(channel);
        }
    }
    pub fn detach_output(&self, agent_id: &str) {
        if let Some(s) = self.find(agent_id) {
            s.output.detach();
        }
    }

    pub fn pending_notifications(&self, agent_id: &str) -> Vec<NotificationEvent> {
        match self.session_id_for(agent_id) {
            Some(sid) => self.hub.pending(&sid),
            None => Vec::new(),
        }
    }

    fn on_exit(&self, sess: &Arc<Session>, outcome: ExitOutcome) {
        let intentional = sess.kill_requested.load(Ordering::SeqCst);
        let exit = SessionExitInfo {
            session_id: sess.session_id.clone(),
            exit_code: outcome.exit_code,
            signal: outcome.signal,
            intentional,
        };
        let next = if intentional { SessionState::Disposed } else { SessionState::Exited };
        *sess.state.lock().unwrap() = next;
        self.registry.set_state(&sess.session_id, next);
        self.emit_state(sess, next, Some(exit));

        // 미해결 알림 정리(E1/E3).
        self.hub.purge_session(&sess.session_id);

        if next == SessionState::Disposed {
            // 재사용 안 함 → 맵/레지스트리에서 제거(E8: 이후 hook은 폐기).
            self.sessions.lock().unwrap().remove(&sess.agent_id);
            self.registry.remove(&sess.session_id);
        }
        // Exited(예기치 않은 종료)는 진단/재기동 위해 레지스트리에 유지.
    }

    fn emit_state(&self, sess: &Arc<Session>, state: SessionState, exit: Option<SessionExitInfo>) {
        self.events.session_state(&SessionStateEvent {
            session_id: sess.session_id.clone(),
            agent_id: sess.agent_id.clone(),
            state,
            exit,
            at: now_ms(),
        });
    }
}

fn spawn_output_pump(
    session_id: String,
    agent_id: String,
    mut rx: tokio::sync::mpsc::UnboundedReceiver<ReaderMsg>,
    sink: Arc<OutputSink>,
    hub: Arc<NotificationHub>,
) {
    tokio::spawn(async move {
        let mut batcher = OutputBatcher::new(session_id.clone(), agent_id);
        let mut deadline: Option<tokio::time::Instant> = None;
        loop {
            let timer = async {
                match deadline {
                    Some(d) => tokio::time::sleep_until(d).await,
                    None => std::future::pending::<()>().await, // 데드라인 없으면 영원히 대기
                }
            };
            tokio::select! {
                _ = timer => {
                    batcher.flush(&*sink);
                    deadline = None;
                }
                msg = rx.recv() => match msg {
                    Some(ReaderMsg::Data(bytes)) => {
                        if bytes.contains(&0x07) {
                            hub.on_bell(&session_id); // BEL 폴백(dedup이 연속 억제)
                        }
                        batcher.push(&bytes);
                        if batcher.pending_bytes() >= MAX_BYTES {
                            batcher.flush(&*sink);
                            deadline = None;
                        } else if deadline.is_none() {
                            deadline = Some(tokio::time::Instant::now()
                                + std::time::Duration::from_millis(WINDOW_MS));
                        }
                    }
                    Some(ReaderMsg::Eof) | None => {
                        batcher.flush_final(&*sink); // 잔여 강제 방출
                        break;
                    }
                }
            }
        }
    });
}

fn default_shell() -> (String, Vec<String>) {
    #[cfg(windows)]
    {
        // 아키텍트 결정: Windows는 powershell, -l -i 없이.
        ("powershell.exe".to_string(), Vec::new())
    }
    #[cfg(not(windows))]
    {
        let shell = std::env::var("SHELL")
            .unwrap_or_else(|_| if cfg!(target_os = "macos") { "/bin/zsh".into() } else { "/bin/bash".into() });
        (shell, vec!["-l".into(), "-i".into()])
    }
}

fn home_dir() -> String {
    std::env::var("HOME")
        .or_else(|_| std::env::var("USERPROFILE"))
        .unwrap_or_else(|_| ".".into())
}
```

### 3.4 NotificationHub + Clock — `notification/hub.rs`

```rust
// src-tauri/src/notification/hub.rs
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use uuid::Uuid;

use crate::types::*;
use crate::state::{AppEvents, SessionRegistry};

/// 주입 가능한 시계. dedup 윈도우(Instant) + at 타임스탬프(epoch ms).
pub trait Clock: Send + Sync {
    fn now(&self) -> Instant;
    fn now_ms(&self) -> u64;
}

pub struct SystemClock;
impl Clock for SystemClock {
    fn now(&self) -> Instant { Instant::now() }
    fn now_ms(&self) -> u64 { now_ms() }
}

pub struct NotificationHub {
    registry: Arc<SessionRegistry>,
    events: Arc<dyn AppEvents>,
    clock: Arc<dyn Clock>,
    dedup_window: Duration,
    queues: Mutex<HashMap<SessionId, Vec<NotificationEvent>>>,
    last_seen: Mutex<HashMap<String, Instant>>,
}

impl NotificationHub {
    pub fn new(
        registry: Arc<SessionRegistry>,
        events: Arc<dyn AppEvents>,
        clock: Arc<dyn Clock>,
        dedup_window: Duration,
    ) -> Self {
        Self {
            registry, events, clock, dedup_window,
            queues: Mutex::new(HashMap::new()),
            last_seen: Mutex::new(HashMap::new()),
        }
    }

    /// axum 핸들러가 호출: 원본 hook body에서 메시지 추출 후 ingest.
    pub fn ingest_hook(&self, session_id: &str, source: NotificationSource, body: &[u8]) {
        let message = extract_message(body, source);
        self.ingest(session_id, source, message);
    }

    /// BEL 폴백: output pump가 0x07 감지 시.
    pub fn on_bell(&self, session_id: &str) {
        self.ingest(session_id, NotificationSource::Bell, "Terminal bell".to_string());
    }

    fn ingest(&self, session_id: &str, source: NotificationSource, message: String) {
        // E8: 죽은/미지 세션의 hook은 폐기.
        let Some(agent_id) = self.registry.resolve_agent(session_id) else {
            return;
        };

        let key = dedup_key(session_id, source, &message);
        let now_i = self.clock.now();
        {
            let mut ls = self.last_seen.lock().unwrap();
            if let Some(prev) = ls.get(&key) {
                if now_i.duration_since(*prev) < self.dedup_window {
                    ls.insert(key, now_i); // 윈도우 슬라이드
                    return; // 억제
                }
            }
            ls.insert(key.clone(), now_i);
        }

        let ev = NotificationEvent {
            id: Uuid::new_v4().to_string(),
            session_id: session_id.to_string(),
            agent_id,
            source,
            message,
            dedup_key: key,
            at: self.clock.now_ms(),
        };
        self.queues.lock().unwrap().entry(session_id.to_string()).or_default().push(ev.clone());
        self.events.notification_new(&ev);
    }

    pub fn pending(&self, session_id: &str) -> Vec<NotificationEvent> {
        self.queues.lock().unwrap().get(session_id).cloned().unwrap_or_default()
    }

    /// 터미널 열림 시 클리어. ids 없으면 세션 전체. cleared된 id 방출.
    pub fn clear(&self, session_id: &str, ids: Option<Vec<String>>) -> Vec<String> {
        let cleared: Vec<String> = {
            let mut q = self.queues.lock().unwrap();
            let Some(list) = q.get_mut(session_id) else { return Vec::new(); };
            match ids {
                Some(ids) if !ids.is_empty() => {
                    let set: std::collections::HashSet<_> = ids.into_iter().collect();
                    let hit: Vec<String> = list.iter().filter(|e| set.contains(&e.id)).map(|e| e.id.clone()).collect();
                    list.retain(|e| !set.contains(&e.id));
                    hit
                }
                _ => {
                    let all: Vec<String> = list.iter().map(|e| e.id.clone()).collect();
                    q.remove(session_id);
                    all
                }
            }
        };
        if !cleared.is_empty() {
            if let Some(agent_id) = self.registry.resolve_agent(session_id) {
                self.events.notification_cleared(&agent_id, &cleared);
            }
        }
        cleared
    }

    pub fn purge_session(&self, session_id: &str) {
        self.queues.lock().unwrap().remove(session_id);
    }
}

fn dedup_key(session_id: &str, source: NotificationSource, message: &str) -> String {
    // sha1-or-equivalent. sha1_smol(순수 Rust, 추가 트랜지티브 의존 없음).
    let mut h = sha1_smol::Sha1::new();
    h.update(format!("{}|{}|{}", session_id, source.as_key(), message.trim()).as_bytes());
    h.digest().to_string()
}

fn extract_message(body: &[u8], source: NotificationSource) -> String {
    if let Ok(v) = serde_json::from_slice::<serde_json::Value>(body) {
        if let Some(m) = v.get("message").and_then(|m| m.as_str()) {
            if !m.trim().is_empty() {
                return m.to_string();
            }
        }
    }
    match source {
        NotificationSource::Stop => "Claude finished a task",
        _ => "Claude needs your attention",
    }
    .to_string()
}
```

### 3.5 HookServer (axum) — `notification/hook_server.rs`

**크레이트 선택 근거**: `tiny_http`(동기, 최소 의존)와 `axum`(async) 중 **axum 채택** — (1) Tauri가 이미 싣는 **tokio 런타임 재사용**(두 번째 스레딩 모델을 안 만든다). (2) `axum::serve(...).with_graceful_shutdown(rx)`로 **RunEvent::ExitRequested와 깔끔히 연동**(§5 E3). (3) 쿼리 파싱·바디 수신이 간결. 훅 요청 처리는 락 잡고 큐 push + emit뿐이라 async 오버헤드 미미.

```rust
// src-tauri/src/notification/hook_server.rs
use std::sync::Arc;

use axum::{
    body::Bytes,
    extract::{Query, State},
    http::StatusCode,
    response::IntoResponse,
    routing::post,
    Router,
};
use serde::Deserialize;
use tokio::net::TcpListener;
use tokio::sync::oneshot;
use tokio::task::JoinHandle;

use crate::notification::hub::NotificationHub;
use crate::types::NotificationSource;

#[derive(Deserialize)]
struct HookQuery {
    session: String,
    #[serde(default)]
    source: String,
}

async fn handle_hook(
    State(hub): State<Arc<NotificationHub>>,
    Query(q): Query<HookQuery>,
    body: Bytes, // curl --data-binary @- 의 원본 이벤트 JSON
) -> impl IntoResponse {
    let source = if q.source == "stop" {
        NotificationSource::Stop
    } else {
        NotificationSource::Hook
    };
    hub.ingest_hook(&q.session, source, &body);
    (
        StatusCode::OK,
        [(axum::http::header::CONTENT_TYPE, "application/json")],
        r#"{"ok":true}"#,
    )
}

/// 127.0.0.1 랜덤 포트에 바인딩하고 (실제 포트, 서버 태스크 핸들) 반환.
/// E4: 포트 0 = OS 할당으로 정적 충돌 원천 차단.
pub async fn serve(
    hub: Arc<NotificationHub>,
    shutdown_rx: oneshot::Receiver<()>,
) -> std::io::Result<(u16, JoinHandle<()>)> {
    let listener = TcpListener::bind(("127.0.0.1", 0)).await?;
    let port = listener.local_addr()?.port();
    let app = Router::new().route("/hook", post(handle_hook)).with_state(hub);
    let handle = tokio::spawn(async move {
        let _ = axum::serve(listener, app)
            .with_graceful_shutdown(async move {
                let _ = shutdown_rx.await;
            })
            .await;
    });
    Ok((port, handle))
}
```

`/hook` 외 경로·메서드는 axum이 자동 404/405. Electron 설계의 라우팅 의미와 동일.

### 3.6 HookSettingsWriter — `notification/hook_settings.rs`

```rust
// src-tauri/src/notification/hook_settings.rs
use std::fs;
use std::path::PathBuf;

use serde_json::json;

#[derive(Clone)]
pub struct HookSettingsWriter {
    base_dir: PathBuf, // <temp>/agent-office/hooks
}

impl HookSettingsWriter {
    pub fn new(base_dir: PathBuf) -> Self {
        Self { base_dir }
    }

    fn curl(port: u16, session_id: &str, source: &str) -> String {
        // sessionId는 uuid v4(URL-safe)라 인코딩 불필요.
        let url = format!("http://127.0.0.1:{port}/hook?session={session_id}&source={source}");
        format!("curl -sS -m 2 -X POST '{url}' -H 'Content-Type: application/json' --data-binary @- || true")
    }

    /// Electron 설계 §3.3과 동일한 Notification/Stop 훅 JSON.
    pub fn build(&self, session_id: &str, port: u16) -> serde_json::Value {
        let entry = |source: &str| {
            json!([{
                "matcher": "",
                "hooks": [{ "type": "command", "command": Self::curl(port, session_id, source) }]
            }])
        };
        json!({ "hooks": { "Notification": entry("hook"), "Stop": entry("stop") } })
    }

    pub fn write(&self, session_id: &str, port: u16) -> std::io::Result<PathBuf> {
        fs::create_dir_all(&self.base_dir)?;
        let p = self.path_for(session_id);
        fs::write(&p, serde_json::to_vec_pretty(&self.build(session_id, port))?)?;
        Ok(p)
    }

    pub fn cleanup(&self, session_id: &str) {
        let _ = fs::remove_file(self.path_for(session_id));
    }

    fn path_for(&self, session_id: &str) -> PathBuf {
        self.base_dir.join(format!("{session_id}.settings.json"))
    }
}
```

생성되는 `command` 문자열(예):
`curl -sS -m 2 -X POST 'http://127.0.0.1:52413/hook?session=<uuid>&source=hook' -H 'Content-Type: application/json' --data-binary @- || true`

### 3.7 프런트엔드 어댑터 — `src/renderer/ipc/tauriApi.ts` (전체)

`AgentOfficeApi`를 `@tauri-apps/api` 위에 얇게 구현. `TerminalRegistry`/`sessionBridge`는 무변경으로 소비.

```typescript
// src/renderer/ipc/tauriApi.ts
import { invoke, Channel } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import type {
  AgentOfficeApi,
  SessionStateEvent,
  NotificationEvent,
  OutputChunk,
  PersistedState,
} from '../../shared/types';

// agentId당 Channel 하나 + JS측 콜백 팬아웃(동결 API의 다중 onData 허용 대응).
interface OutputSub {
  channel: Channel<OutputChunk>;
  cbs: Set<(data: string) => void>;
}
const outputSubs = new Map<string, OutputSub>();

export const tauriApi: AgentOfficeApi = {
  async createSession(agentId, opts) {
    // autostartClaude는 동결 opts에 없음 → 백엔드 기본값(false, 빈 로그인 셸) 사용.
    return await invoke('create_session', { agentId, opts: opts ?? null });
  },

  async disposeSession(agentId) {
    await invoke('dispose_session', { agentId });
  },

  writeInput(agentId, data) {
    void invoke('write_input', { agentId, data }); // fire-and-forget
  },

  resize(agentId, cols, rows) {
    void invoke('resize_session', { agentId, cols, rows });
  },

  clearNotifications(agentId, ids) {
    void invoke('clear_notifications', { agentId, ids: ids ?? null });
  },

  async listNotifications(agentId) {
    return await invoke('list_notifications', { agentId });
  },

  async loadState() {
    return await invoke('load_state');
  },

  async saveState(state: PersistedState) {
    await invoke('save_state', { state });
  },

  setBadgeCount(n) {
    void invoke('set_badge_count', { count: n });
  },

  onData(agentId, cb) {
    let sub = outputSubs.get(agentId);
    if (!sub) {
      const channel = new Channel<OutputChunk>();
      const created: OutputSub = { channel, cbs: new Set() };
      channel.onmessage = (chunk) => {
        for (const f of created.cbs) f(chunk.data);
      };
      outputSubs.set(agentId, created);
      // 등록을 createSession보다 먼저/동시에 → 조기 출력은 백엔드 백로그가 방어.
      void invoke('subscribe_output', { agentId, channel });
      sub = created;
    }
    sub.cbs.add(cb);
    return () => {
      const s = outputSubs.get(agentId);
      if (!s) return;
      s.cbs.delete(cb);
      if (s.cbs.size === 0) {
        outputSubs.delete(agentId);
        void invoke('unsubscribe_output', { agentId });
      }
    };
  },

  onSessionState(cb) {
    return wrapListen<SessionStateEvent>('session-state', cb);
  },

  onNotification(cb) {
    return wrapListen<NotificationEvent>('notification-new', cb);
  },

  onNotificationCleared(cb) {
    return wrapListen<{ agentId: string; ids: string[] }>('notification-cleared', cb);
  },
};

// listen()은 Promise<UnlistenFn>을 반환하므로, 동기 unsubscribe 계약을 래핑한다.
function wrapListen<T>(event: string, cb: (payload: T) => void): () => void {
  let un: UnlistenFn | null = null;
  let disposed = false;
  listen<T>(event, (e) => cb(e.payload)).then((f) => {
    if (disposed) f();
    else un = f;
  });
  return () => {
    disposed = true;
    if (un) un();
  };
}
```

`src/renderer/ipc/window.d.ts`의 `AgentOfficeApi`(R1)와 `../../shared/types`는 그대로 사용된다. 기존 preload `window.api` 대신 이 `tauriApi` 객체를 주입 지점(`sessionBridge`가 소비하는 API 참조)에 연결한다.

---

## 4. 상태·부트스트랩 — `state.rs`, `lib.rs`, `ipc/commands.rs`

### 4.1 AppEvents / SessionRegistry / AppState — `state.rs`

```rust
// src-tauri/src/state.rs
use std::collections::HashMap;
use std::sync::{Arc, Mutex, RwLock};

use serde::Serialize;
use tauri::{AppHandle, Emitter};
use tokio::sync::oneshot;
use tokio::task::JoinHandle;

use crate::types::*;
use crate::session::manager::SessionManager;
use crate::notification::hub::NotificationHub;
use crate::persistence::profile_store::ProfileStore;

/// 이벤트 방출 경계(테스트 주입점). 프로덕션=TauriEvents, 테스트=RecordingEvents.
pub trait AppEvents: Send + Sync {
    fn session_state(&self, ev: &SessionStateEvent);
    fn notification_new(&self, ev: &NotificationEvent);
    fn notification_cleared(&self, agent_id: &str, ids: &[String]);
}

pub struct TauriEvents {
    pub app: AppHandle,
}
impl AppEvents for TauriEvents {
    fn session_state(&self, ev: &SessionStateEvent) {
        let _ = self.app.emit("session-state", ev);
    }
    fn notification_new(&self, ev: &NotificationEvent) {
        let _ = self.app.emit("notification-new", ev);
    }
    fn notification_cleared(&self, agent_id: &str, ids: &[String]) {
        #[derive(Serialize)]
        #[serde(rename_all = "camelCase")]
        struct Payload<'a> { agent_id: &'a str, ids: &'a [String] }
        let _ = self.app.emit("notification-cleared", &Payload { agent_id, ids });
    }
}

/// sid → (agentId, state). SessionManager가 쓰고 NotificationHub가 읽어 순환 의존 제거.
#[derive(Default)]
pub struct SessionRegistry {
    map: RwLock<HashMap<SessionId, (AgentId, SessionState)>>,
}
impl SessionRegistry {
    pub fn new() -> Self { Self::default() }
    pub fn insert(&self, sid: &str, agent: &str, state: SessionState) {
        self.map.write().unwrap().insert(sid.into(), (agent.into(), state));
    }
    pub fn set_state(&self, sid: &str, state: SessionState) {
        if let Some(e) = self.map.write().unwrap().get_mut(sid) { e.1 = state; }
    }
    pub fn remove(&self, sid: &str) {
        self.map.write().unwrap().remove(sid);
    }
    pub fn resolve_agent(&self, sid: &str) -> Option<AgentId> {
        self.map.read().unwrap().get(sid).map(|(a, _)| a.clone())
    }
}

pub struct AppState {
    pub manager: Arc<SessionManager>,
    pub hub: Arc<NotificationHub>,
    pub store: ProfileStore,
    pub hook_shutdown: Mutex<Option<oneshot::Sender<()>>>,
    pub server_handle: Mutex<Option<JoinHandle<()>>>,
}
```

### 4.2 커맨드 — `ipc/commands.rs`

```rust
// src-tauri/src/ipc/commands.rs
use tauri::{ipc::Channel, State, AppHandle, Manager};

use crate::state::AppState;
use crate::types::*;

#[derive(Default, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionOpts {
    pub cols: Option<u16>,
    pub rows: Option<u16>,
    pub cwd: Option<String>,
}

#[tauri::command(rename_all = "camelCase")]
pub async fn create_session(app_state: State<'_, AppState>, agent_id: String, opts: Option<SessionOpts>) -> Result<CreateSessionResult, String> {
    let o = opts.unwrap_or_default();
    app_state.manager.create(CreateSessionRequest {
        agent_id,
        cols: o.cols, rows: o.rows, cwd: o.cwd,
        autostart_claude: None, // 항상 기본값(false) → 빈 로그인 셸
    })
}

#[tauri::command(rename_all = "camelCase")]
pub async fn dispose_session(app_state: State<'_, AppState>, agent_id: String) -> Result<(), String> {
    app_state.manager.dispose(&agent_id);
    Ok(())
}

#[tauri::command(rename_all = "camelCase")]
pub async fn write_input(app_state: State<'_, AppState>, agent_id: String, data: String) -> Result<(), String> {
    app_state.manager.write_input(&agent_id, &data);
    Ok(())
}

#[tauri::command(rename_all = "camelCase")]
pub async fn resize_session(app_state: State<'_, AppState>, agent_id: String, cols: u16, rows: u16) -> Result<(), String> {
    app_state.manager.resize(&agent_id, cols, rows);
    Ok(())
}

#[tauri::command(rename_all = "camelCase")]
pub async fn subscribe_output(app_state: State<'_, AppState>, agent_id: String, channel: Channel<OutputChunk>) -> Result<(), String> {
    app_state.manager.attach_output(&agent_id, channel);
    Ok(())
}

#[tauri::command(rename_all = "camelCase")]
pub async fn unsubscribe_output(app_state: State<'_, AppState>, agent_id: String) -> Result<(), String> {
    app_state.manager.detach_output(&agent_id);
    Ok(())
}

#[tauri::command(rename_all = "camelCase")]
pub async fn list_notifications(app_state: State<'_, AppState>, agent_id: String) -> Result<Vec<NotificationEvent>, String> {
    Ok(app_state.manager.pending_notifications(&agent_id))
}

#[tauri::command(rename_all = "camelCase")]
pub async fn clear_notifications(app_state: State<'_, AppState>, agent_id: String, ids: Option<Vec<String>>) -> Result<(), String> {
    if let Some(sid) = app_state.manager.session_id_for(&agent_id) {
        app_state.hub.clear(&sid, ids);
    }
    Ok(())
}

#[tauri::command(rename_all = "camelCase")]
pub async fn load_state(app_state: State<'_, AppState>) -> Result<PersistedState, String> {
    Ok(app_state.store.load())
}

/// 주의: Tauri State 파라미터는 `app_state`, JS 페이로드 `{ state }`는 `state` 파라미터로 받는다
/// (이름 충돌 회피 — JS 인자 키와 Rust 파라미터명이 일치해야 매핑된다).
#[tauri::command(rename_all = "camelCase")]
pub async fn save_state(app_state: State<'_, AppState>, state: PersistedState) -> Result<(), String> {
    app_state.store.save(&state).map_err(|e| e.to_string())
}

#[tauri::command(rename_all = "camelCase")]
pub async fn set_badge_count(app: AppHandle, count: i64) -> Result<(), String> {
    // Tauri v2 배지 API. 대상 버전에 따라 window/app 레벨 시그니처 확인 필요.
    if let Some(win) = app.get_webview_window("main") {
        let _ = win.set_badge_count(if count > 0 { Some(count) } else { None });
    }
    Ok(())
}
```

### 4.3 부트스트랩 & graceful quit — `lib.rs`

```rust
// src-tauri/src/lib.rs
mod types; mod state; mod session; mod notification; mod persistence; mod ipc;

use std::sync::{Arc, Mutex};
use std::time::Duration;

use tauri::{Manager, RunEvent};
use tokio::sync::oneshot;

use crate::state::*;
use crate::session::manager::SessionManager;
use crate::session::pty_factory::PortablePtyFactory;
use crate::notification::hub::{NotificationHub, SystemClock};
use crate::notification::hook_server;
use crate::notification::hook_settings::HookSettingsWriter;
use crate::persistence::profile_store::ProfileStore;

pub fn run() {
    tauri::Builder::default()
        .setup(|app| {
            let handle = app.handle().clone();
            let events: Arc<dyn AppEvents> = Arc::new(TauriEvents { app: handle.clone() });
            let registry = Arc::new(SessionRegistry::new());
            let hub = Arc::new(NotificationHub::new(
                registry.clone(),
                events.clone(),
                Arc::new(SystemClock),
                Duration::from_millis(3000), // dedup 3s
            ));

            // hook 서버 기동(포트 동기 획득). E4: 바인딩 실패 시 1회 재시도.
            //
            // > (구현 노트: 위 스케치는 배선 버그가 있다 — `serve`는 sender drop도
            // > shutdown 신호로 취급하는데(`let _ = shutdown_rx.await;`가 send/drop을
            // > 구분하지 않음), 재시도 분기에서 만든 `_tx2`를 바로 버리면(`_` 프리픽스로
            // > drop) 재시도해서 막 띄운 서버가 그 즉시 셧다운 신호를 받아 죽는다. 게다가
            // > `AppState`에는 첫 시도의 `tx`가 저장되는데, 그 `rx`는 이미 실패한 첫
            // > `serve` 호출 안에서 소비된 뒤라 어디에도 닿지 않는 유령 sender가 된다.
            // > 실제 구현은 `hook_server::serve_with_retry`로 이 로직을 캡슐화한다 —
            // > 시도마다 새 oneshot 쌍을 만들고, **성공한(=살아있는 서버와 짝이 맞는)
            // > 시도의 sender만** 반환해 `AppState`에 저장한다.)
            let (port, tx, server_handle) = tauri::async_runtime::block_on(
                hook_server::serve_with_retry(|rx| hook_server::serve(hub.clone(), rx)),
            )?;

            let temp = app.path().temp_dir()?.join("agent-office").join("hooks");
            let hook_writer = HookSettingsWriter::new(temp);
            let get_port = Arc::new(move || port);

            let manager = Arc::new(SessionManager::new(
                Arc::new(PortablePtyFactory),
                hook_writer,
                registry.clone(),
                events.clone(),
                hub.clone(),
                get_port,
            ));

            let data_dir = app.path().app_data_dir()?;
            let store = ProfileStore::new(data_dir.join("profiles.json"));

            app.manage(AppState {
                manager,
                hub,
                store,
                hook_shutdown: Mutex::new(Some(tx)),
                server_handle: Mutex::new(Some(server_handle)),
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            ipc::commands::create_session,
            ipc::commands::dispose_session,
            ipc::commands::write_input,
            ipc::commands::resize_session,
            ipc::commands::subscribe_output,
            ipc::commands::unsubscribe_output,
            ipc::commands::list_notifications,
            ipc::commands::clear_notifications,
            ipc::commands::load_state,
            ipc::commands::save_state,
            ipc::commands::set_badge_count,
        ])
        .build(tauri::generate_context!())
        .expect("failed to build tauri app")
        .run(|app, event| {
            // E3: 앱 종료 — 모든 PTY kill + settings 정리 + hook 서버 graceful shutdown.
            if let RunEvent::ExitRequested { .. } = event {
                let state = app.state::<AppState>();
                state.manager.dispose_all(); // kill + settings cleanup(동기)
                if let Some(tx) = state.hook_shutdown.lock().unwrap().take() {
                    let _ = tx.send(()); // axum graceful shutdown 트리거
                }
                // wait 스레드가 Disposed 확정 후 OS가 자식 reap. 프로세스 종료는 정상 진행.
            }
        });
}
```

### 4.4 ProfileStore — `persistence/profile_store.rs`

```rust
// src-tauri/src/persistence/profile_store.rs
use std::fs;
use std::path::PathBuf;

use crate::types::PersistedState;

pub struct ProfileStore {
    file: PathBuf,
}
impl ProfileStore {
    pub fn new(file: PathBuf) -> Self {
        Self { file }
    }

    pub fn load(&self) -> PersistedState {
        match fs::read(&self.file) {
            Ok(bytes) => match serde_json::from_slice::<PersistedState>(&bytes) {
                Ok(s) if s.version == 1 => s,
                _ => PersistedState::empty(), // 버전 불일치/파손 → 빈 상태(마이그레이션 훅 지점)
            },
            Err(_) => PersistedState::empty(),
        }
    }

    pub fn save(&self, state: &PersistedState) -> std::io::Result<()> {
        if let Some(parent) = self.file.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&self.file, serde_json::to_vec_pretty(state)?)
    }
}
```

sessionId는 **런타임 전용**(SessionManager/SessionRegistry 메모리)으로 디스크에 저장하지 않는다. `NotificationHub.resolve_agent`는 `SessionRegistry`가 단일 소스.

---

## 5. 엣지 케이스 E1~E9 (Rust 매핑)

| # | 상황 | Rust 처리 |
|---|------|-----------|
| **E1** | 세션 프로세스 예기치 않은 종료 | wait 스레드 `waiter.wait()` 반환 → `kill_requested=false` → `on_exit`가 `SessionState::Exited` 전이 이벤트(`session-state`) 방출. 레코드는 sessions/registry에 **유지**(재기동/진단). `hub.purge_session`으로 미해결 알림 정리. |
| **E2** | claude만 종료, 쉘 생존 | PTY 자체는 살아있어 `read`/`wait` 미종료 → 상태 전이 없음(`Running` 유지). Stop hook이 이미 알림 발생시켰을 수 있음. 사용자는 프롬프트에서 재실행. |
| **E3** | 앱 quit | `RunEvent::ExitRequested`에서 `manager.dispose_all()`(각 세션 `kill_requested=true` + `control.kill()` + settings cleanup) + `hook_shutdown` oneshot 전송으로 axum graceful shutdown. 각 세션은 wait 스레드에서 `Disposed` 확정. |
| **E4** | hook 서버 포트 충돌 | `TcpListener::bind(("127.0.0.1", 0))` → OS가 빈 포트 할당(정적 충돌 없음). 바인딩 실패 시 setup에서 **1회 재시도**(새 포트 0). 실제 포트는 `get_hook_port` 클로저로 create 시점에 읽어 settings에 주입. |
| **E5** | 동일 세션 다중 알림/dedup | 큐는 `Mutex<HashMap<SessionId, Vec<..>>>`로 순서 유지. `last_seen`에서 동일 `dedupKey`가 3s 내 재도착하면 억제(윈도우 슬라이드). 서로 다른 메시지는 별도 항목. |
| **E6** | PTY 대량 출력 백프레셔 | reader thread → mpsc → output pump의 `OutputBatcher`가 16ms 데드라인 + 64KB 상한 코얼레싱. `seq` 단조 증가로 순서 검증. `flush_final`이 EOF/dispose 시 잔여 강제 방출. **추가**: UTF-8 경계 캐리(`valid_utf8_prefix`)로 배치 경계에서 코드포인트 분할 방지. |
| **E7** | hook curl 실패/서버 다운 | settings의 `curl -sS -m 2 ... \|\| true` → hook은 항상 exit 0, claude 흐름 비차단. 알림만 누락, BEL 폴백이 부분 방어. |
| **E8** | 죽은 세션으로 온 hook | `registry.resolve_agent(sid)`가 `None`(Disposed는 registry에서 제거됨) → `ingest` 조기 반환·폐기. |
| **E9** | 에이전트당 중복 생성 | `create`가 sessions 맵에서 기존 `Running/Starting` 조회 시 재사용, 새 PTY 미생성(1 에이전트 1 세션). |

추가 Rust 고유 리스크:
- **UTF-8 분할** — E6에서 처리(위).
- **kill 후 wait 경합** — `clone_killer()`로 wait 스레드가 child 소유해도 별도 kill 가능. intentional 판정은 `AtomicBool`.
- **Channel 미등록 조기 출력** — `OutputSink` 백로그(256청크)로 방어, `attach` 시 순서 유지 드레인.

---

## 6. 테스트 가능성 (Rust 이음새 + cargo 테스트)

### 6.1 주입 가능한 순수/이음새

| 컴포넌트 | 이음새 | 페이크/주입 |
|---|---|---|
| `SessionManager` | `trait PtyFactory` | `FakePtyFactory` — 인메모리 파이프 리더, 기록용 writer(`Arc<Mutex<Vec<u8>>>`), 테스트가 firing하는 exit 채널. |
| 상태/알림 방출 | `trait AppEvents` | `RecordingEvents`(`Arc<Mutex<Vec<...>>>` 수집) — Tauri 앱 없이 단위 테스트. |
| `NotificationHub` | `trait Clock` | `FakeClock`(atomic ms 오프셋 + `advance()`), `SessionRegistry` 직접 조립, dedup 결정론적. |
| `OutputBatcher` | `trait FlushSink` | `RecordingSink`(Vec 수집), 타이머 없이 push/flush 직접 호출로 결정론. |
| `HookSettingsWriter` | 순수 `build()` | 파일 IO 없이 JSON 구조/curl 문자열 단언. `write/cleanup`은 tempdir. |
| `HookServer` | 통합 | 실제 `serve` + reqwest POST(포트 0). |

`FakeClock`은 `Instant`를 직접 만들 수 없으므로 생성 시 base `Instant`를 잡고 `AtomicU64` 오프셋 ms를 더한 값을 `now()`로 반환한다.

### 6.2 구체 테스트 케이스 (단언 스케치)

**T-A. SessionManager: exit 상태 전이 + intentional 플래그** — `#[tokio::test]`(pump/reader 스레드 사용).
```rust
let events = Arc::new(RecordingEvents::default());
let (fac, ctl) = FakePtyFactory::new(); // ctl.fire_exit(code) 노출
let mgr = Arc::new(SessionManager::new(Arc::new(fac), writer, registry.clone(),
    events.clone(), hub, Arc::new(|| 12345)));
mgr.create(req("a1")).unwrap();
ctl.fire_exit(1);                        // 예기치 않은 종료
wait_for(|| events.states().len() == 3);
assert_eq!(events.states(), vec![Starting, Running, Exited]);
let last = events.last_state().exit.unwrap();
assert!(!last.intentional && last.exit_code == Some(1));
```

**T-B. SessionManager: autostart stdin 주입** — `#[tokio::test]`.
```rust
mgr.create(CreateSessionRequest { agent_id: "a1".into(), autostart_claude: Some(true), .. }).unwrap();
let written = ctl.writes_utf8();
assert!(regex(r#"^claude --settings ".+\.settings\.json"\n$"#).is_match(&written));
// autostart_claude=Some(false) 또는 None(기본값)이면 writes 비어 있음(빈 로그인 셸).
```

**T-C. NotificationHub: dedup 3s 윈도우** — 동기 테스트(런타임 불필요).
```rust
let clock = Arc::new(FakeClock::new());
registry.insert("s1", "a1", Running);
let hub = NotificationHub::new(registry, events.clone(), clock.clone(), Duration::from_millis(3000));
hub.ingest_hook("s1", Hook, br#"{"message":"need input"}"#);      // 통과
clock.advance(1000);
hub.ingest_hook("s1", Hook, br#"{"message":"need input"}"#);      // 억제
clock.advance(4000);
hub.ingest_hook("s1", Hook, br#"{"message":"need input"}"#);      // 통과(윈도우 밖)
assert_eq!(events.notifications().len(), 2);
assert_eq!(hub.pending("s1").len(), 2);
```

**T-D. NotificationHub: clear 부분 vs 전체** — 동기.
```rust
for m in ["m1","m2","m3"] { hub.ingest_hook("s1", Hook, msg(m)); }
let ids: Vec<_> = hub.pending("s1").iter().map(|e| e.id.clone()).collect();
assert_eq!(hub.clear("s1", Some(vec![ids[1].clone()])), vec![ids[1].clone()]); // 부분
assert_eq!(hub.pending("s1").iter().map(|e|&e.id).collect::<Vec<_>>(), vec![&ids[0], &ids[2]]);
assert_eq!(hub.clear("s1", None).len(), 2);                                    // 전체
assert!(hub.pending("s1").is_empty());
```

**T-E. OutputBatcher: 크기 상한 + seq + UTF-8 경계 캐리** — 동기.
```rust
let sink = RecordingSink::default();
let mut b = OutputBatcher::new("s1".into(), "a1".into());
b.push(b"abc"); b.push(b"def");
b.flush(&sink);
assert_eq!(sink.at(0), OutputChunk { data: "abcdef".into(), frames: 2, seq: 0, .. });
// 멀티바이트('한'=ED 95 9C)를 경계에서 분할
b.push(&[0xED, 0x95]);      // 불완전 선두
b.flush(&sink);
assert_eq!(sink.len(), 1);  // 방출 없음(캐리)
b.push(&[0x9C]);            // 완성
b.flush(&sink);
assert_eq!(sink.at(1).data, "한");
assert_eq!(sink.at(1).seq, 1);
```

**T-F. HookServer 라우팅 (통합)** — `#[tokio::test]`.
```rust
let (tx, rx) = oneshot::channel();
let hub = /* RecordingEvents + registry.insert("s1","a1",Running) 조립 */;
let (port, _h) = hook_server::serve(hub.clone(), rx).await.unwrap();
reqwest::Client::new()
    .post(format!("http://127.0.0.1:{port}/hook?session=s1&source=stop"))
    .body(r#"{"message":"done"}"#).send().await.unwrap();
wait_for(|| !hub.pending("s1").is_empty());
let ev = &hub.pending("s1")[0];
assert!(matches!(ev.source, NotificationSource::Stop) && ev.message == "done");
let _ = tx.send(()); // graceful shutdown
```

`#[tokio::test]` 필요: T-A, T-B(스레드/pump), T-F(axum). 순수 동기: T-C, T-D, T-E. 배칭 타이머(16ms 데드라인) 검증이 필요하면 `tokio::time::pause()` + `advance()`로 별도 `#[tokio::test]` 추가.

---

## 7. Cargo.toml 의존성

```toml
# src-tauri/Cargo.toml
[package]
name = "agent-office"
version = "0.1.0"
edition = "2021"

[lib]
name = "agent_office_lib"
crate-type = ["staticlib", "cdylib", "rlib"]

[build-dependencies]
tauri-build = { version = "2", features = [] }

[dependencies]
tauri = { version = "2", features = [] }        # Channel/이벤트/경로 API 포함
serde = { version = "1", features = ["derive"] }
serde_json = "1"
tokio = { version = "1", features = ["rt-multi-thread", "macros", "net", "time", "sync"] }
axum = "0.7"                                     # latest stable. hook HTTP 서버
portable-pty = "0.8"                             # latest stable. wezterm PTY
uuid = { version = "1", features = ["v4"] }      # sessionId/notification id
sha1_smol = "1"                                  # dedupKey(순수 Rust, 추가 트랜지티브 의존 없음)

[dev-dependencies]
reqwest = { version = "0.12", default-features = false }  # HookServer 통합 테스트
tokio = { version = "1", features = ["rt-multi-thread", "macros", "time", "test-util"] }
```

- **디렉터리**: `dirs` 크레이트 불필요 — Tauri `app.path().temp_dir()`/`app_data_dir()` 사용.
- **배지**: 별도 플러그인 없이 Tauri v2 window/app 배지 API 사용(버전에 따라 시그니처 확인). 불가 시 후행 확장.
- `portable-pty`/`axum` 정확 마이너는 빌드 시점 `cargo update`로 최신 안정판 확정.

---

## 8. 구현 태스크 분해 (순서, 독립 테스트 가능)

모두 TDD(테스트 먼저 → 실패 → 구현 → 통과 → 커밋). 각 태스크는 타입(T1)에만 의존, 상호 페이크 주입으로 병렬 개발 가능.

| 순서 | 태스크 | 산출 파일 | 완료 기준(테스트) |
|------|--------|-----------|-------------------|
| **T1** | 계약 타입 + serde 미러 | `src-tauri/src/types.rs` | 컴파일 통과. `SessionState`/`NotificationSource` serde 소문자, 구조체 camelCase 왕복 스냅샷 테스트(`types.ts`와 문자열 값 일치). |
| **T2** | HookSettingsWriter | `notification/hook_settings.rs` | `build()` JSON 구조/curl(`-m 2`, `--data-binary @-`, `\|\| true`, 포트/세션 치환) 단언, `write/cleanup` tempdir IO. |
| **T3** | OutputBatcher + FlushSink | `session/output_batcher.rs` | T-E(크기 상한/seq/UTF-8 캐리/flush_final). |
| **T4** | PtyFactory 트레잇 + Fake | `session/pty_factory.rs` (+ `#[cfg(test)]` FakePtyFactory) | 페이크가 read(인메모리 파이프)/write 기록/resize/kill/exit-fire 시뮬레이트. |
| **T5** | AppEvents/SessionRegistry | `state.rs`(AppState 제외 부분) | `RecordingEvents` 수집 확인, registry insert/set_state/remove/resolve. |
| **T6** | SessionManager | `session/manager.rs` | T-A(전이+intentional), T-B(autostart), E9(재사용), dispose→Disposed 제거, resize/write Running 가드. `#[tokio::test]`. |
| **T7** | HookServer(axum) | `notification/hook_server.rs` | T-F(포트 0 바인딩, `/hook` 라우팅/파싱, graceful shutdown). `#[tokio::test]`. |
| **T8** | Clock + NotificationHub | `notification/hub.rs` | T-C(dedup 3s), T-D(부분/전체 clear), on_bell, purge_session, E8(죽은 세션 폐기). |
| **T9** | ProfileStore | `persistence/profile_store.rs` | load/save 왕복(tempdir), version!=1 → empty, 파손 파일 → empty. |
| **T10** | 커맨드 + 배선 | `ipc/commands.rs`, `state.rs`(AppState) | 각 커맨드가 manager/hub/store로 위임하는지(모의 State 조립), `save_state` 인자 매핑, `list_notifications`가 Vec 반환. |
| **T11** | 부트스트랩 + quit | `lib.rs`, `main.rs` | setup 순서(hook serve→port→manager), `RunEvent::ExitRequested`에서 dispose_all+shutdown 전송 순서, E4 포트 재시도. 모의 통합. |
| **T12** | 프런트 어댑터 | `src/renderer/ipc/tauriApi.ts` | (vitest) invoke/Channel 모킹 — onData 팬아웃/unsubscribe refcount, wrapListen pre-resolution unsubscribe. |

의존: T6←T2/T3/T4/T5, T8←T5, T7←T8, T10←T6/T8/T9, T11←T10, T12←T1(타입).

---

## 9. 요약 결정 사항

- **동시성**: 세션당 reader thread(블로킹) → tokio mpsc → output pump task(배칭/BEL/Channel) + wait thread(블로킹). 전역 락은 핫 패스에서 회피, `SessionRegistry`(RwLock)로 hook 라우팅.
- **PTY 출력 전송**: **Tauri v2 `Channel<OutputChunk>`**(순서 보장·전역 브로드캐스트 회피), 어댑터가 agentId당 팬아웃 + 백엔드 백로그로 조기 출력 방어. 상태/알림은 이벤트(`session-state`/`notification-new`/`notification-cleared`).
- **PTY = 로그인 인터랙티브 쉘**(`$SHELL -l -i`; Windows powershell). 기본값은 빈 로그인 쉘(autostart_claude 기본 false) — 스폰 env에 노출되는 `AGENT_OFFICE_SETTINGS`(settings.json 경로)로 사용자가 직접 `claude --settings "$AGENT_OFFICE_SETTINGS"`를 실행한다. `autostart_claude: Some(true)`를 명시한 경우에만 그 커맨드를 stdin에 주입 → 쉘 상시 생존.
- **hook**: per-session settings JSON(temp dir)의 Notification/Stop → `curl ... -m 2 --data-binary @- || true` → `axum` 127.0.0.1 랜덤 포트. **BEL(0x07) 폴백** 상시.
- **dedup**: `sha1_smol(session|source|message)` + 3s 윈도우, 세션별 큐(부분/전체 clear), `Clock` 주입.
- **백프레셔**: 16ms/64KB 코얼레싱 + `seq`, **UTF-8 경계 캐리**(Rust 고유), exit/dispose 시 `flush_final`.
- **테스트**: 모든 부작용(pty/http/fs/event/clock)을 트레잇 뒤로 → cargo 테스트 T-A~T-F, `#[tokio::test]`는 스레드/타이머/HTTP 케이스에만.
- **영속화**: `PersistedState`(agents+version:1) → app data dir JSON, sessionId는 런타임 전용 미저장. **quit**: `RunEvent::ExitRequested`에서 PTY kill + settings cleanup + axum graceful shutdown.

---

## 10. 완료 알림 고도화 (이슈 #39)

> 이 절이 §3.4 스켈레톤의 `extract_message`/hub 알림 동작을 대체하는 정본이다.
> observer 어댑터 리팩터(§3.4는 curl+extract_message 시절 스케치)를 반영한다.

### 10.1 완료 알림에 완료 내용 담기

`ObserverEvent::Stop { message, running }`의 `message`를 실제 완료 내용으로 채운다.
비면 종전대로 hub의 `STOP_FALLBACK`("작업이 완료되었습니다.")을 쓴다.

- **Codex**: `observer/codex.rs`의 `Stop` 매핑이 body의 `last_assistant_message`를
  `event::codex_stop_message`로 추출·절단한다(예전엔 의도적으로 버렸다).
- **Claude**: Stop 훅 body엔 `message`가 없다. `observer/claude.rs`가
  `message(body).or_else(|| claude_transcript_message(body))`로, body의
  `transcript_path`(JSONL)를 끝에서 최대 64KB만 읽어(`read_file_tail`) 뒤에서부터
  줄을 스캔, 마지막 `type=="assistant"` 라인의 `message.content[]` 중 `type=="text"`
  조각을 이어붙인다. 파일 부재/포맷 이상은 None으로 폴백.
- **절단**: `event::MAX_STOP_MESSAGE_CHARS`(300, chars 기준) 초과 시 `truncate_stop_message`가
  `head + "…"`로 자르고, 공백뿐이면 None. 프런트는 이 위에서 `MAX_EXCERPT`(80)로 더 줄인다.

### 10.2 완료 후에도 계속 진행 중이면 "진행중"으로 복귀

두 경로로 idle→working 복귀를 만든다.

- **(a) 결정적 신호 — `turnReducer`**: idle 상태에서 `tool` 입력이 오면 새 턴을 연다
  (`openTurn`). Stop 이후 PreToolUse/PostToolUse는 확실한 "작업 재개" 신호.
- **(b) 출력 휴리스틱 — `NotificationHub`**: `spawn_output_pump`가 BEL 배선 옆에서
  세션 PTY 출력 배치마다 `hub.on_output(session_id, byte_len)`을 호출한다. hub는
  Stop 알림이 **실제 방출된** 시각을 세션별로 기억(`resume_watch`)하고,
  `RESUME_GRACE`(3s, 프롬프트 리드로우 무시)가 지난 뒤 `RESUME_WINDOW`(30s) 내에
  누적 출력이 `RESUME_THRESHOLD_BYTES`(8KB) 초과하면 **1회만**:
  1. 그 세션의 stop 소스 알림을 `clear`로 걷어낸다(`notification-cleared` 재사용).
  2. `ActivityKind::Resume` activity-event를 방출한다.
  임계치를 크게 잡는 이유는 키 에코/입력박스 리드로우 오탐 방지. 시각은 hub의 `Clock`
  추상화를 따르며, 상수는 `#[cfg(test)] with_resume_params`로 주입 가능.
  프런트 `sessionBridge.onActivity`는 `resume`을 `applyActivityEvent`로 흘리고,
  `appStore`가 이를 턴 목적상 `tool`과 동일 취급해 working으로 복귀시킨다. 잘못
  열린 턴은 기존 stop/settle(세션 종료) 경로가 그대로 정산하므로 별도 타임아웃은 없다.

### 10.3 앱이 백그라운드일 때 터미널이 열려 있어도 알림

- **창 포커스 추적**: `installWindowFocusTracking`(`ipc/windowFocus.ts`)이
  `getCurrentWindow().onFocusChanged`(초기값 `isFocused()`)를 구독해
  `appStore.windowFocused`를 갱신한다. bootstrap의 브리지 설치 직후 설치.
- **억제 완화**: `pushNotification`은 `activeTerminalAgentId === e.agentId && windowFocused`
  일 때만 억제 → 비포커스면 티커/배지/사운드가 모두 동작.
- **OS 데스크탑 알림**: `tauri-plugin-notification`(Cargo/`lib.rs` 플러그인/capabilities
  `notification:*`/npm `@tauri-apps/plugin-notification`). `sessionBridge.onNotification`이
  `!windowFocused`일 때만 `maybeSendOsNotification`(`ipc/osNotify.ts`, 동적 import)로
  발송한다. 제목=에이전트 이름/ID, 본문=메시지 excerpt. 권한은 최초 발송 전 1회
  확인/요청. 테스트는 플러그인/`@tauri-apps/api/window` 모듈을 모킹.
