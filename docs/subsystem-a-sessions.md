# 서브시스템 A 상세 설계 (Rust / Tauri v2) — 세션 · 알림 · 영속화

상태: 정본 — 2026-07-20 현행화(리팩터 R-1~R-7 반영). 구현 전 산출물(코드 스켈레톤 구 §3·§4, Cargo.toml 사본 구 §7, 태스크 분해 구 §8)은 제거했다 — **구현은 코드가 정본**이고, 이 문서는 현재 구조·계약·결정과 그 근거를 서술한다. 절 번호 §9(요약 결정)·§10(완료 알림)·§11(설정 복구)은 외부(이슈) 참조 보존을 위해 유지.

> 설계: Opus 하위 설계 / 주요 판단: Fable. 원본 Electron 설계는 이력으로만 존재(상태 머신·dedup·배칭·훅 메커니즘의 논리를 1:1 이식). `src/shared/types/`(배럴 `types.ts`)가 TS 단일 소스이고, Rust `types.rs`는 serde로 미러링한다.

---

## 0. 파일 레이아웃 & 동시성 모델

### 0.1 `src-tauri/` 모듈 레이아웃 (2026-07-20 현재)

```
src-tauri/src/
  main.rs                  # 얇은 진입점: --sessiond / --observer-forward / ctl 인자 분기 → run()
  lib.rs                   # run(): Builder + setup + invoke_handler + RunEvent.
                           #   make_pty_factory가 AGENT_OFFICE_SESSION_BROKER=v2 opt-in 판정
  state.rs                 # AppState, AppEvents 트레잇, TauriEvents, SessionRegistry
  types.rs                 # serde 계약 타입 (src/shared/types/ 미러, §1)
  session/
    manager.rs             # SessionManager 코어: 생성(create_with_profile)·설치(install_session)·
                           #   입출력·리사이즈·dispose·exit 처리 (~790줄)
    manager/tests.rs       # 인라인 테스트 분리(R-1)
    manager/real_pty_smoke.rs
    output.rs              # OutputSink(백로그·Channel attach/detach)·spawn_output_pump·snapshot_offset
    output_batcher.rs      # 16ms/64KB 배칭 + UTF-8 경계 캐리 (§3.2)
    handoff_v1.rs          # v1 fd 핸드오프/입양: handoff_all/handoff_one/adopt_detached/adopt_one
    handoff_broker.rs      # v2 브로커 갈래: handoff_all_broker/adopt_detached_broker 등
    pty_factory.rs         # PtyFactory 트레잇 + PortablePtyFactory (+ 테스트 Fake)
    broker_pty.rs          # BrokerPtyFactory (v2 상시 브로커, unix)
    poll_reader.rs         # poll 기반 인터럽트 가능 reader (unix, 핸드오프 전제)
    shells.rs              # 셸 탐지·선택 + PowerShell 래퍼
    zsh_wrapper.rs / bash_wrapper.rs / wrapper_script.rs
                           # 셸별 rc 심 + CommandWrapperSpec 기반 claude()/pi() 래퍼 렌더
    pi_extension.rs        # Pi 확장 파일 배포 (docs/pi-support-design.md)
    env_capture.rs         # 로그인 셸 env 캡처 (봇 모드 #58)
  notification/
    hub.rs (+ hub/)        # NotificationHub: dedup/큐/clear + hold(§10.4)/resume-watch(§10.2)
  observer/                # 훅 파이프라인 (구 notification/hook_server·hook_settings의 후신)
    mod.rs                 # ObserverAdapter 트레잇 + ObserverRuntime(ingest→hub)
    server.rs              # axum 로컬 HTTP 서버 (POST /hook, provider/agent 쿼리 라우팅)
    claude.rs / codex.rs   # 어댑터: 세션 설정 파일·이벤트 매핑·restore_session_artifact
    event.rs               # ObserverEvent·CommandWrapperSpec·메시지 추출/절단
    forwarder.rs           # --observer-forward 훅 중계 (포트 스테일 완화, 이슈 #30)
    hook_command.rs        # 훅 커맨드 문자열 빌더
    claude_resume_recorder.rs  # native 세션 ID 캡처 (docs/claude-session-resume-design.md)
  sessiond/                # 세션 존속 데몬 (unix). v1 테이블 + v2 브로커 겸용
    daemon.rs (+ daemon/)  # handle_connection → 오피코드별 핸들러 13개로 분해(R-5)
    client.rs / protocol.rs
  session_events/          # 분석용 시계열 store/reader (archive/session-event-timeseries + analytics 문서)
  persistence/             # profile/settings/diary/work_log/session_time/claude_resume/png 스토어
  ipc/
    commands.rs            # 얇은 허브 — 도메인 서브모듈 pub(crate) 재수출만
    commands/{session,persistence,media,settings,bot,usage,misc}.rs  # 커맨드 본문(R-2)
    commands/tests.rs
  workdir/                 # git 러너·파서·diff·리스팅·커맨드 (R-3, subsystem-c §10이 소비)
    {git_runner,status,diff,listing,model,commands,mod}.rs
  bot/                     # 봇 모드 (docs/bot-mode-design.md)
  control/                 # CLI 제어 서버·클라이언트 (docs/cli-control-design.md)
  usage/                   # 구독 사용량 (docs/usage-design.md)
  pixellab/ summarizer/    # 스프라이트 생성·요약 외부 API 클라이언트
  markdown.rs shell_export.rs terminal.rs vscode.rs api_keys.rs  # 단일 파일 유틸
```

프런트 계약 쪽은 `src/shared/types/{common,session,notification,bot,profile,diary,usage,settings,markdown,git,api}.ts`로 도메인 분할된 배럴(`shared/types.ts`는 재수출만, R-6)이고, IPC 커맨드명 상수는 `src/shared/ipc.ts`, 어댑터는 `src/renderer/ipc/tauriApi.ts`다.

### 0.2 동시성 모델

Rust/Tauri는 **OS 스레드 + tokio 태스크 혼합**이다. `portable-pty`의 리더는 **블로킹 I/O**(`Read`)라 async로 감쌀 수 없으므로 세션당 전용 스레드가 필요하다.

세션 1개당 자원:

| 자원 | 종류 | 역할 |
|---|---|---|
| **reader thread** | `std::thread` (블로킹) | master PTY에서 raw 바이트를 blocking `read` → `tokio::sync::mpsc::UnboundedSender<ReaderMsg>`로 전달. EOF면 `ReaderMsg::Eof` 후 종료. unix에서는 핸드오프를 위해 poll 기반(`poll_reader.rs`, shutdown pipe로 인터럽트 가능). |
| **output pump task** | `tokio::task` | 위 채널을 수신, `OutputBatcher` 소유. 16ms 데드라인(`sleep_until`) + 64KB 상한으로 코얼레싱, `FlushSink`(Channel)로 방출. BEL(0x07) 감지 시 `hub.on_bell`, 배치마다 `hub.on_output`(§10.2). |
| **wait thread** | `std::thread` (블로킹) | `child.wait()`(블로킹) → `ExitOutcome`. `kill_requested`로 intentional 판정, `Exited`/`Disposed` 전이 이벤트 방출. |
| **PTY writer** | `Mutex<Box<dyn Write + Send>>` | 커맨드 스레드에서 짧게 락 잡고 stdin 주입. |

전역 자원:

- **observer 훅 서버**: tokio 태스크 1개(axum, `observer/server.rs`). `AppHandle`을 통해 어느 스레드에서든 이벤트 emit(Send+Sync).
- **AppState**: Tauri `Manager::manage`로 등록, 커맨드는 `tauri::State<'_, AppState>`로 접근. **핫 패스(PTY 출력)는 전역 락을 절대 잡지 않는다** — 각 세션의 출력은 자기 `OutputSink`(Channel)로 직접 흐른다.
- **SessionRegistry**: `sid → (agentId, state)`의 `RwLock<HashMap>`. SessionManager가 쓰고 NotificationHub/observer가 읽어 **순환 의존을 끊는다**(훅 라우팅 `resolve_agent`).

```
[PTY master] ──read(blocking)──> reader thread ──mpsc::Data(bytes)──> output pump task
                                                                        │  OutputBatcher(16ms/64KB, seq, utf8 carry)
                                                                        ├─ 0x07 감지 → hub.on_bell() / on_output
                                                                        └─ FlushSink → tauri::ipc::Channel<OutputChunk> → [webview]
[PTY child] ──wait(blocking)──> wait thread ── AppEvents.session_state() ──emit "session-state"──> [webview]
[claude/codex/pi 훅] ──POST /hook──> axum task ── ObserverRuntime.ingest ── hub ──emit "notification-new"/"activity-event"──> [webview]
```

---

## 1. Rust 계약 타입 — `types.rs` ↔ `src/shared/types/`

타입 정의 자체는 코드가 정본(`src-tauri/src/types.rs` 1,179줄, workdir 계열은 `workdir/model.rs`). 여기엔 **양쪽이 어긋나면 안 되는 매핑 규칙**만 적는다.

- 구조체는 `#[serde(rename_all = "camelCase")]` — Rust `snake_case` → TS `camelCase`.
- enum은 `#[serde(rename_all = "lowercase")]` — PascalCase variant → TS 소문자 문자열 값 (`SessionState = 'starting'|'running'|'exited'|'disposed'`, `NotificationSource = 'hook'|'stop'|'bell'` 등).
- epoch ms는 `u64`(TS `number`). `Option<T>`는 `T | undefined`이며 `skip_serializing_if`로 생략.
- **TS(`src/shared/types/`)가 정본이고 Rust는 미러** — 수동 동기화이므로 필드 추가 시 양쪽 + `shared/__tests__/contract.test.ts` 왕복 픽스처를 함께 갱신한다. (컴파일러가 어긋남을 못 잡는 구조적 부채 — 자동 안전망은 REBUILD-PLAN R-9 결정 항목.)
- 뒤에 추가된 계약 필드는 `#[serde(default)]`로 additive하게만 늘린다(브로커 프로토콜과 동일 원칙).

핵심 계약 타입(대표): `SessionState`/`SessionStateEvent`/`SessionExitInfo`, `NotificationSource`/`NotificationEvent`/`NotificationClearedEvent`, `ActivityKind`/`ActivityEvent`, `CreateSessionRequest`/`CreateSessionResult`, `OutputChunk{sessionId, agentId, data, frames, seq}`, `AgentProfile`(+`bot`)/`PersistedState`, `AppSettings`, `BotConfig`/`BotStatus`, `UsageSnapshot`.

- `NotificationEvent.id`는 **NotificationHub가 uuid로 발급**(렌더러 재발급 금지 — clear 동기화 때문).
- `SessionExitInfo.signal`은 항상 None(portable-pty가 크로스플랫폼 ExitStatus에서 시그널을 분리 노출하지 않음). 단 브로커 세션은 데몬이 waitpid하므로 exit code가 살아난다(broker-v2 문서).
- `listNotifications`는 **배열**(`NotificationEvent[]`)을 반환한다 — 스냅샷 래퍼 타입은 IPC 표면에 없다.
- sessionId는 **런타임 전용**(SessionManager/SessionRegistry 메모리) — 디스크에 저장하지 않는다.

---

## 2. 렌더러 경계 매핑 — Tauri 커맨드 & 이벤트 (정확한 문자열)

**모든 커맨드는 `#[tauri::command(rename_all = "camelCase")]`** 로 선언해 JS는 `{ agentId, ... }` 카멜케이스로 인자를 넘긴다. `State<'_, AppState>` 파라미터명은 전부 `app_state`다 — `save_state`의 페이로드 파라미터 `state: PersistedState`와 이름이 충돌하면 Tauri의 JS 인자 키 ↔ Rust 파라미터명 매핑이 조용히 깨지기 때문.

### 2.1 코어 세션·알림 커맨드 (초기 설계의 동결 표면, 전부 현존)

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

### 2.2 전체 커맨드 표면 (도메인 파일별, 2026-07-20 현재 총 54개)

이름은 코드가 정본 — `lib.rs`의 `generate_handler!`와 `src/shared/ipc.ts`가 접점.

| 파일 | 커맨드 |
|---|---|
| `ipc/commands/session.rs` | 2.1의 세션·알림 8개(`create_session`~`clear_notifications`, `subscribe_output`/`unsubscribe_output` 포함) + `list_available_shells`, `handoff_supported`, `session_broker_mode`, `upload_session_snapshots`, `handoff_sessions`, `adopt_detached_sessions` |
| `ipc/commands/persistence.rs` | `load_state`, `save_state`, `append_session_turn`, `load_session_turns`, `append_diary_entry`, `load_diary`, `save_work_log`, `load_work_logs`, `load_session_events`, `list_claude_resume_sessions` |
| `ipc/commands/media.rs` | `save_portrait`, `load_portrait`, `delete_portrait`, `save_sprite`, `load_sprite`, `delete_sprite`, `summarize_text`, `generate_sprite_image` |
| `ipc/commands/settings.rs` | `get_app_settings`, `set_app_settings`, `control_status`, `control_approve`, `control_revoke` (+ GUI/CLI 공유 `apply_settings_effects`) |
| `ipc/commands/bot.rs` | `bot_start`, `bot_stop`, `bot_status` |
| `ipc/commands/usage.rs` | `load_usage_snapshot` (+ 순수 함수 `resolve_usage_roots`) |
| `ipc/commands/misc.rs` | `set_badge_count`, `open_in_vscode`, `open_in_terminal`, `export_terminal_output`, `pick_directory` |
| `workdir/commands.rs` | `workdir_list_files`, `workdir_git_status`, `workdir_diff_file`, `workdir_file_history`, `workdir_diff_commit`, `workdir_commit_files`, `workdir_repo_log`, `workdir_difftool` |

### 2.3 이벤트 (emit/listen) — 정확한 이름

| 구독 | 전송 방식 | 이름 | 페이로드 |
|---|---|---|---|
| `onData(agentId, cb)` | **`tauri::ipc::Channel<OutputChunk>`** | (커맨드 인자로 전달) | `OutputChunk` |
| `onSessionState` | 이벤트 emit | `"session-state"` | `SessionStateEvent` |
| `onNotification` | 이벤트 emit | `"notification-new"` | `NotificationEvent` |
| `onNotificationCleared` | 이벤트 emit | `"notification-cleared"` | `NotificationClearedEvent` |
| `onActivity` | 이벤트 emit | `"activity-event"` | `ActivityEvent` (prompt/tool/sub-*/resume) |

### 2.4 PTY 출력 전송 방식 결정 — Channel 채택 (근거)

**결론: PTY 출력(최다 트래픽)은 Tauri v2 `Channel<OutputChunk>`, 나머지 저빈도 신호는 전역 이벤트.**

- **Channel 채택 근거**: (1) Channel은 특정 스트림 전용 fast-path로 **순서 보장** + JSON 브로드캐스트/전역 리스너 깨움 오버헤드 회피. (2) `OutputBatcher`가 이미 16ms/64KB로 빈도를 세션당 ~60/s로 낮췄고, Channel은 그 위에서 백프레셔에 유리. (3) `onData(agentId, cb)`는 구독 모델이지만, Channel도 `subscribe_output(agentId, channel)` 커맨드 + 반환 unsubscribe(`unsubscribe_output`)로 자연스럽게 감싼다.
- **다중 구독자/조기 출력 처리**: 같은 agentId에 `onData`를 여러 번 부를 수 있다. 백엔드는 **agentId당 Channel 하나**만 두고, **어댑터(`tauriApi.ts`)가 콜백 Set으로 팬아웃**하며 refcount가 0이 되면 unsubscribe한다. Channel 등록 이전 출력은 백엔드 `OutputSink`가 소량(256청크) **백로그**에 담았다가 attach 시 순서 유지로 드레인 → 조기 출력 유실 방지. 데몬 링버퍼(512KB)·입양 리플레이도 같은 백로그 경로로 수렴한다.
- **이벤트를 안 쓰는 이유**: 전역 이벤트는 broadcast + JS측 필터가 필요하고 고빈도에서 낭비. 단, 세션 상태/알림/activity는 **여러 리스너**(오피스 씬 + 티커 + 배지 + 타임라인)가 듣는 저빈도 신호라 이벤트가 적합.
- 이 전송 방식은 `AgentOfficeApi`에 노출되지 않으므로, 추후 교체해도 계약 무변경. 어댑터의 `wrapListen`은 listen Promise 해소 전에 unsubscribe돼도 누수 없게 처리한다.

---

## 3. 핵심 컴포넌트 (구현은 코드가 정본 — 여기엔 불변식·결정만)

구 §3의 구현 전 코드 스켈레톤(~950줄)은 제거했다. 각 컴포넌트의 유지해야 할 계약·불변식:

### 3.1 PtyFactory — `session/pty_factory.rs`

- **부작용 경계 트레잇**: `PtyFactory::spawn(PtySpawnOptions) -> SpawnedPty { reader, writer, control(resize/kill), waiter }`. SessionManager는 이 트레잇만 알고, 테스트는 Fake를 주입한다. 이 이음새 덕에 v2 브로커도 `BrokerPtyFactory` 교체 한 겹으로 들어갔다(broker-v2 문서).
- `SpawnedPty`에는 핸드오프용 `reader_interrupt`/`handoff`(v1, unix)와 `broker_owned`(v2 혼합 상황 분류) 필드가 추가돼 있다 — 의미는 각 핸드오프 문서 참조.
- slave는 spawn 직후 닫는다. `clone_killer()`로 wait 스레드가 child를 소유해도 별도 kill 가능.
- 스폰 셸은 **로그인 인터랙티브 셸**(`$SHELL -l -i`, Windows powershell). 주입 env: `AGENT_OFFICE_SESSION`(항상), `AGENT_OFFICE_HOOK_URL`·`AGENT_OFFICE_SETTINGS`(훅 on일 때), `AGENT_OFFICE_PI_EXT`(훅 on), `TERM=xterm-256color`. 셸 rc 심(`zsh_wrapper`/`bash_wrapper`/`shells.rs`)이 `claude()`/`pi()` 래퍼를 정의해 `--settings`/`-e`를 투명 주입한다(`wrapper_script.rs`의 `CommandWrapperSpec` 렌더).

### 3.2 OutputBatcher — `session/output_batcher.rs`

- 순수 배칭 로직(타이밍은 pump가 소유): `MAX_BYTES = 64KB`, `WINDOW_MS = 16`(≈60fps), 세션별 `seq` 단조 증가, `frames` 카운트.
- **UTF-8 경계 캐리**: 배치 경계에서 코드포인트가 쪼개지면 불완전 꼬리를 다음 배치로 이월(`valid_utf8_prefix`) — Rust 고유 리스크 처리.
- `flush_final`이 EOF/dispose 시 잔여를 강제 방출.

### 3.3 SessionManager — `session/manager.rs` (+ output/handoff_v1/handoff_broker)

- 상태 머신: `Starting → Running → Exited | Disposed`. 전이는 이벤트 `"session-state"`로 방출, intentional 판정은 `kill_requested: AtomicBool`.
- **1 에이전트 = 1 세션 불변식**: `create`가 기존 Starting/Running 세션을 재사용(E9).
- **`install_session` 추출**: spawn 이후 배선(세션 등록, reader/pump/wait 3스레드, Running CAS)을 create와 입양(adopt)이 공유한다 — 핸드오프 v1 때 만든 구조가 v2에서도 그대로 재사용됨.
- 핸드오프/입양 분기는 `broker_mode` + 세션별 `broker_owned`로 가르고, 갈래 본문은 `handoff_v1.rs`/`handoff_broker.rs`로 파일 격리(R-4) — v1 제거(R-8) 시 파일 삭제로 끝나게.
- `handed_off` 세션은 `on_exit`/`dispose`/`dispose_all`에서 스킵(kill·cleanup·상태이벤트 금지).

### 3.4 NotificationHub — `notification/hub.rs`

- dedup: `sha1_smol(session|source|message.trim())` 키 + 3s 윈도우(윈도우 슬라이드), 세션별 큐, 부분/전체 clear, `Clock` 트레잇 주입으로 결정론 테스트.
- Hook 소스 hold(질문 알림 지연)·Stop 후 resume-watch(진행중 복귀)는 §10이 정본.
- `purge_session`으로 죽은 세션 알림 정리. 죽은 세션의 훅은 `registry.resolve_agent` 실패로 조기 폐기(E8).

### 3.5 observer 훅 파이프라인 — `observer/`

- 초기 설계의 `hook_server.rs`(수신)·`hook_settings.rs`(설정 파일)는 **어댑터 구조로 일반화**됐다: `ObserverAdapter` 트레잇(claude/codex 구현) + `ObserverRuntime`(ingest → hub), Pi는 `ingest_pi_source` 직행 갈래(pi-support 문서 §0.5).
- 훅 커맨드는 curl 직결이 아니라 **앱 바이너리 forwarder**(`--observer-forward`) 경유 — 포트 스테일 완화(이슈 #30, handoff 문서 §5). 훅 실패는 항상 비차단(claude 흐름에 영향 0), BEL 폴백 상시.
- Claude 설정 파일은 `<app_data>/observer/claude/`(§11에서 OS temp로부터 이동), 세션별 생성·정리. 입양 시 멱등 복구(§11).
- 훅 서버 기동은 `serve_with_retry`로 캡슐화 — 시도마다 새 oneshot 쌍을 만들고 **성공한 시도의 shutdown sender만** AppState에 저장한다. (sender drop도 shutdown 신호로 취급되므로, 재시도 분기에서 만든 tx를 버리면 방금 띄운 서버가 즉사하는 배선 버그가 있다 — 초기 스케치에서 실제로 발견해 캡슐화로 해소한 함정.)

### 3.6 부트스트랩·종료 — `lib.rs`

- setup 순서: 훅 서버(포트 확보) → ObserverRuntime → SessionManager(팩토리는 `make_pty_factory`가 broker opt-in 판정) → 스토어들 → `app.manage(AppState)`. 500ms 스위퍼가 `hub.flush_expired`(§10.4)를 돌린다.
- graceful quit: `RunEvent::ExitRequested`에서 `dispose_all()`(handed_off 스킵) + 훅 서버 shutdown 전송. "유지하고 종료"는 프런트가 `handoff_sessions` 완료 후 destroy(handoff 문서).

---

## 5. 엣지 케이스 E1~E9 (Rust 매핑)

| # | 상황 | Rust 처리 |
|---|------|-----------|
| **E1** | 세션 프로세스 예기치 않은 종료 | wait 스레드 `waiter.wait()` 반환 → `kill_requested=false` → `on_exit`가 `SessionState::Exited` 전이 이벤트(`session-state`) 방출. 레코드는 sessions/registry에 **유지**(재기동/진단). `hub.purge_session`으로 미해결 알림 정리. |
| **E2** | claude만 종료, 쉘 생존 | PTY 자체는 살아있어 `read`/`wait` 미종료 → 상태 전이 없음(`Running` 유지). Stop hook이 이미 알림 발생시켰을 수 있음. 사용자는 프롬프트에서 재실행(래퍼가 설정 파일 부재 시 §11 가드로 강등 실행). |
| **E3** | 앱 quit | `RunEvent::ExitRequested`에서 `manager.dispose_all()`(각 세션 `kill_requested=true` + `control.kill()` + settings cleanup) + 훅 서버 graceful shutdown. 각 세션은 wait 스레드에서 `Disposed` 확정. handed_off 세션은 스킵. |
| **E4** | hook 서버 포트 충돌 | `TcpListener::bind(("127.0.0.1", 0))` → OS가 빈 포트 할당(정적 충돌 없음). 바인딩 실패 시 `serve_with_retry`가 1회 재시도(§3.5의 sender 함정 참조). 실제 포트는 `get_hook_port` 클로저로 create 시점에 읽어 env/설정에 주입. |
| **E5** | 동일 세션 다중 알림/dedup | 큐는 `Mutex<HashMap<SessionId, Vec<..>>>`로 순서 유지. `last_seen`에서 동일 `dedupKey`가 3s 내 재도착하면 억제(윈도우 슬라이드). 서로 다른 메시지는 별도 항목. Hook 소스의 기록 시점 예외는 §10.4. |
| **E6** | PTY 대량 출력 백프레셔 | reader thread → mpsc → output pump의 `OutputBatcher`가 16ms 데드라인 + 64KB 상한 코얼레싱. `seq` 단조 증가로 순서 검증. `flush_final`이 EOF/dispose 시 잔여 강제 방출. UTF-8 경계 캐리로 코드포인트 분할 방지. |
| **E7** | hook 전송 실패/서버 다운 | forwarder/curl은 짧은 타임아웃 + 실패 무시(항상 exit 0) → claude 흐름 비차단. 알림만 누락, BEL 폴백이 부분 방어. forwarder는 포트 파일 재시도(handoff 문서 §5). |
| **E8** | 죽은 세션으로 온 hook | `registry.resolve_agent(sid)`가 `None`(Disposed는 registry에서 제거됨) → ingest 조기 반환·폐기. |
| **E9** | 에이전트당 중복 생성 | `create`가 sessions 맵에서 기존 `Running/Starting` 조회 시 재사용, 새 PTY 미생성(1 에이전트 1 세션). |

추가 Rust 고유 리스크:
- **UTF-8 분할** — E6에서 처리(위).
- **kill 후 wait 경합** — `clone_killer()`로 wait 스레드가 child 소유해도 별도 kill 가능. intentional 판정은 `AtomicBool`.
- **Channel 미등록 조기 출력** — `OutputSink` 백로그(256청크)로 방어, attach 시 순서 유지 드레인.

---

## 6. 테스트 가능성 (이음새)

테스트 본문은 코드가 정본: `session/manager/{tests,real_pty_smoke}.rs`, `ipc/commands/tests.rs`, `notification/hub/`, `sessiond/daemon/tests.rs` 등(R-1로 인라인 테스트 전부 파일 분리). 유지해야 할 것은 **주입 이음새 설계**다:

| 컴포넌트 | 이음새 | 페이크/주입 |
|---|---|---|
| `SessionManager` | `trait PtyFactory` | Fake — 인메모리 파이프 리더, 기록용 writer, 테스트가 firing하는 exit 채널. |
| 상태/알림 방출 | `trait AppEvents` | `RecordingEvents`(수집) — Tauri 앱 없이 단위 테스트. |
| `NotificationHub` | `trait Clock` | `FakeClock`(base `Instant` + atomic ms 오프셋 `advance()`), dedup/hold 결정론. |
| `OutputBatcher` | `trait FlushSink` | RecordingSink, 타이머 없이 push/flush 직접 호출. |
| observer 어댑터 | `trait ObserverAdapter`/`ClaudeSessionSink` | 어댑터 단위 매핑 테스트·페이크 sink. |
| 훅 서버 | 통합 | 실제 `serve`(포트 0) + reqwest POST. |
| 스토어 | 경로 주입 | tempdir. |

프런트 어댑터(`tauriApi.ts`)는 vitest로 invoke/Channel 모킹 — onData 팬아웃/unsubscribe refcount, wrapListen pre-resolution unsubscribe. 실행: `npx vitest run --dir src` / `cargo test --manifest-path src-tauri/Cargo.toml`.

---

## 9. 요약 결정 사항

- **동시성**: 세션당 reader thread(블로킹) → tokio mpsc → output pump task(배칭/BEL/Channel) + wait thread(블로킹). 전역 락은 핫 패스에서 회피, `SessionRegistry`(RwLock)로 hook 라우팅.
- **PTY 출력 전송**: **Tauri v2 `Channel<OutputChunk>`**(순서 보장·전역 브로드캐스트 회피), 어댑터가 agentId당 팬아웃 + 백엔드 백로그로 조기 출력 방어. 상태/알림/activity는 이벤트(`session-state`/`notification-new`/`notification-cleared`/`activity-event`).
- **PTY = 로그인 인터랙티브 쉘**(`$SHELL -l -i`; Windows powershell). 기본값은 빈 로그인 쉘 — 스폰 env에 노출되는 `AGENT_OFFICE_SETTINGS` 경로로 사용자가 직접 `claude`를 실행하고, 셸 래퍼가 `--settings`를 투명 주입한다.
- **hook**: per-session 설정 파일(`<app_data>/observer/claude/`, §11) → forwarder 경유 POST → `axum` 127.0.0.1 랜덤 포트. **BEL(0x07) 폴백** 상시. 어댑터 구조(observer/)로 claude/codex/pi 공용.
- **dedup**: `sha1_smol(session|source|message)` + 3s 윈도우, 세션별 큐(부분/전체 clear), `Clock` 주입. Hook 홀드·resume 복귀는 §10.
- **백프레셔**: 16ms/64KB 코얼레싱 + `seq`, **UTF-8 경계 캐리**(Rust 고유), exit/dispose 시 `flush_final`.
- **테스트**: 모든 부작용(pty/http/fs/event/clock)을 트레잇 뒤로. 인라인 테스트는 서브모듈 파일로 분리(R-1).
- **영속화**: `PersistedState`(agents+version) → app data dir JSON, sessionId는 런타임 전용 미저장. **quit**: `RunEvent::ExitRequested`에서 PTY kill + settings cleanup + axum graceful shutdown(핸드오프 시 스킵).
- **세션 존속**: v1 fd-핸드오프(기본) + v2 상시 브로커(opt-in) 공존 — 각각 `docs/session-handoff-design.md`/`docs/session-broker-v2-design.md`가 정본.

---

## 10. 완료 알림 고도화 (이슈 #39)

> 이 절이 hub 알림 동작의 정본이다. observer 어댑터 리팩터를 반영한다.

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

### 10.4 오토모드를 감안한 질문 알림 지연 (이슈 #41)

오토모드에서는 에이전트의 질문(`Notification` 훅 → `ObserverEvent::Attention` →
`NotificationSource::Hook`)이 자동 승인되는데도 느낌표 알림이 즉시 떠버린다. hub는
Hook 소스 알림을 `hold_duration`(설정 `attentionHoldMs`, 기본 5초) 만큼 보류했다가,
그 사이 세션이 계속 일한다는 신호가 오면 조용히 폐기하고, 신호가 없으면 그때 방출한다.
`hold_duration == 0`이면 홀드를 끄고 현행대로 즉시 방출한다(기본 hub도 0으로 시작해
기존 동작을 보존).

- **보류**: `ingest`가 dedup 통과 후, `source == Hook`이고 `hold_duration > 0`이면
  큐/이벤트 대신 `held`(세션당 최대 1개)에 넣는다. 같은 세션에 이미 held가 있으면 같은
  `dedup_key`는 무시(원래 타이머 유지), 다른 키는 교체(새 질문이 이전 질문을 대체).
- **dedup 기록 시점**: Hook 소스는 §10.2의 Stop과 달리 ingest가 아니라 **실제 방출
  시점**(즉시 방출이든 flush든)에만 `last_seen`을 남긴다 — 홀드가 폐기된 질문이 dedup
  윈도우 안에 다시 와도 알림이 나가야 하기 때문. Stop/Bell은 종전대로 ingest 시 기록.
- **방출**: 단일 스위퍼(`lib.rs`가 500ms 간격으로 `flush_expired` 호출)가 `held_at`
  기준 만료된 항목을 방출한다. 방출 시점에 세션이 registry에서 사라졌으면(사망) 조용히
  폐기. 최대 500ms 지터는 5초 홀드에서 허용.
- **폐기(취소) 신호 — 모두 조용히, 이벤트 없음**:
  1. `Prompt`/`Tool`/`SubStart` activity(프롬프트 제출·도구 사용·서브에이전트 시작) —
     `SubStop`/`SubCount`/`Resume`은 취소하지 않는다.
  2. `Stop` observer(자동답변 후 턴 종료) — `running` 값과 무관하게 폐기해 질문+완료
     이중 알림을 막는다.
  3. `on_output` 출력 폭주 — §10.2 `resume_watch` 로직 **앞에서**, `HOLD_OUTPUT_GRACE`
     (1s, 질문 UI 렌더링 구간)가 지난 뒤 누적 출력이 `HOLD_OUTPUT_THRESHOLD_BYTES`(8KB)를
     넘으면 폐기. resume_watch 경로는 그대로 유지된다.
  4. 세션 전체 `clear(None)`(터미널 열림)·`purge_session`. 부분 `clear(Some(ids))`는
     held를 건드리지 않는다.
- **설정**: `AppSettings.attentionHoldMs`(serde default 5000). `lib.rs`는 설정 로드 직후,
  `set_app_settings`는 변경 시 `hub.set_hold_duration`으로 반영한다. 설정 UI는
  `SettingsDialog`의 숫자 입력(초 단위, 0~60 clamp, 내부 ms 변환).

## 11. 이어받은 셸에서 Claude 설정 파일 복구 (이슈 #40)

**증상**: 세션 이어하기(핸드오프/브로커 입양)로 앱을 재시작한 뒤, 터미널에서 수동으로
`claude`를 나갔다가 같은 셸에서 다시 `claude`를 치면 `--settings`가 가리키는 임시 설정
파일을 못 찾아 실패.

**근본원인**: Claude 훅 설정 파일이 `<OS_temp>/agent-office/observer/claude/<sessionId>.settings.json`
(OS 임시 디렉터리)에 있었다. 셸의 `AGENT_OFFICE_SETTINGS` env는 스폰 시점 경로로 **영구
고정**되는데, OS temp는 앱이 꺼진 사이 시스템 청소로 사라질 수 있고 복구 경로가 없었다.
핸드오프/입양 자체는 파일을 보존(`daemon.rs`의 Adopt는 삭제 안 함)하지만, 앱-off 창의
temp 청소·강제 삭제를 되돌릴 수단이 없어 그 셸의 `claude` 재실행이 영구 실패했다.

**해결 (B: app_data 안정 경로 + A: 입양 시 복구 + 래퍼 가드)**:

- **경로 이동** — `lib.rs`: settings_dir을 OS temp가 아니라 `<app_data>/observer/claude/`로
  둔다(앱 수명주기가 소유). `ObserverRuntime::production` 시그니처 불변.
- **멱등 복구** — `observer/claude.rs`: 파일 쓰기를 `write_settings_file(path)`로 추출하고,
  `ObserverAdapter::restore_session_artifact(path)`(트레이트 default = no-op) 구현. 파일명이
  `.settings.json`으로 끝나는 경로만 claude 소관이라 판별해 **존재 여부와 무관하게 재작성**
  (낡은 forwarder 경로도 함께 갱신). `ObserverRuntime::restore_session_artifacts(session_id,
  cleanup_paths)`가 순회·계측 로그. 내용은 세션 무관(sessionId·포트 미포함)이라 같은 함수를
  스폰/복구가 공유. 쓰기는 temp+rename 원자적.
- **입양 시점 호출** — v1 `adopt_one`(`session/handoff_v1.rs`)은 `adopted.cleanup_paths`로,
  v2 브로커 `adopt_one_broker`(`session/handoff_broker.rs`)는 `AttachOk`에 additive로 실린
  `cleanup_paths`로 복구를 호출.
  `cleanup_paths`가 비면(observer OFF 세션·codex-only) no-op이라 자연히 건너뛴다.
- **프로토콜 additive** — `sessiond/protocol.rs`의 `AttachOk`에 `#[serde(default)] cleanup_paths`
  추가(데몬이 삭제 소유권 유지, 복구용 경로만 앱에 반환). `client.rs`의 `AttachedMeta`에도
  동반. **삭제 소유권 계약은 불변** — 삭제는 여전히 ObserverPlanGuard(스폰 실패)·dispose·
  on_exit(둘 다 handed_off 스킵)·데몬 자식종료/Kill 5곳만.
- **래퍼 파일-부재 가드** — `CommandWrapperSpec.skip_prefix_if_env_file_missing: Option<String>`
  (env 이름). 렌더된 셸 함수는 prefix를 붙이기 전에 그 env가 가리키는 파일 존재를 확인해,
  없으면 경고 후 `--settings` 없이 원본 `claude`를 실행한다(하드 실패 대신 **비관찰 강등**).
  앱-off 창에서 데몬조차 없어 복구할 수 없을 때의 실행 보장(이슈 열린질문의 답). posix는
  `if [ ! -f "${ENV}" ]`, powershell은 `Test-Path -LiteralPath`. prefix가 비면 무의미해 미방출.
- **GC** — `gc_stale_settings(dir, max_age)`를 부트 시 백그라운드 1회 실행(30일 초과
  `*.settings.json` 청소). app_data로 옮기며 더블-크래시 잔존물이 영구화되는 것 방지. 살아
  있는 세션은 매 입양마다 재작성돼 mtime이 갱신되므로 안전.

**보장 범위**: 앱 재실행·입양이 일어나면 파일이 복구된다. 앱이 완전히 꺼진(데몬도 없는)
창에서 분리된 셸의 `claude`는 "관찰"이 아니라 "실행"까지만 보장(래퍼 가드가 비관찰로
강등). 일반 종료·"모두 종료" 뒤에는 세션 아티팩트가 남지 않고, observer OFF 세션에는
설정 파일/래퍼가 생기지 않는다.
