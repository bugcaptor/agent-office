// src-tauri/src/lib.rs
//
// Bootstrap: settings load -> observer server started only when the opt-in
// setting is ON (binds to port 0 so the OS picks a free port; retries once
// on bind failure) -> session manager wiring -> AppState managed ->
// invoke_handler for the renderer-facing commands -> graceful quit on
// RunEvent::ExitRequested (dispose_all -> observer server shutdown).
// 에이전트 CLI 데이터 루트(CLAUDE_CONFIG_DIR/CODEX_HOME) 결정 규칙 한 곳.
pub mod agent_paths;
pub mod api_keys;
mod bot;
// codex CLI 내장 이미지 생성으로 캐릭터 초상/스프라이트 원본을 만든다(kbm #2fa).
pub mod codex_imagegen;
mod control;
/// 웹 원격(docs/web-remote-design.md) — tailnet의 브라우저가 이 앱의 세션을
/// 출력/입력만 중계받아 보고 조작한다.
pub mod webremote;
// Everything(es.exe) 백엔드(이슈 #67) -- markdown.rs 전용 옵트인 스캔 경로.
mod file_index;
// markdown.rs/workdir::list_workdir_files가 공유하는 병렬 스캔 워커.
mod file_scan;
// 작업 폴더를 OS 파일 탐색기(Finder/탐색기)로 여는 런처.
mod file_manager;
// UI 언어 해석(AppSettings.language → Lang). AI 프롬프트를 만드는 모듈들이
// 공유한다 — 번역 카탈로그는 프런트에만 있다.
pub mod i18n;
// pub: contract 테스트(src-tauri/tests/contract_fixtures.rs)가
// `agent_office_lib::ipc::commands::settings::GetAppSettingsResult`에 닿아야 한다.
// 로직 변경 없음 — 가시성만 승격.
pub mod ipc;
mod markdown;
mod notification;
mod observer;
// pub: contract 테스트가 `agent_office_lib::persistence::settings_store::AppSettings`에
// 닿아야 한다. 로직 변경 없음 — 가시성만 승격.
pub mod persistence;
mod power;
// git/es.exe 등 단발 서브프로세스 실행기(spawn+타임아웃+stdout 수집) 공용 구현.
mod proc_runner;
mod session;
// pub: contract 테스트가 `agent_office_lib::session_events::types::SessionEventRecord`에
// 닿아야 한다. 로직 변경 없음 — 가시성만 승격.
pub mod session_events;
// pub: 세션 로그 전사 필터/저장소를 contract·통합 테스트가 직접 부른다.
pub mod session_log;
mod shell_export;
#[cfg(unix)]
mod sessiond;
mod state;
mod summarizer;
/// 동료 대화(docs/agent-talk-design.md) — 캐릭터끼리 앱을 거쳐 주고받는 메시지.
pub mod talk;
mod terminal;
// 확인 요청 대사 TTS(리라이트+ElevenLabs 합성). 키는 웹뷰에 노출하지 않는다.
// pub: 커맨드 시그니처(`tts_speak`)가 이 모듈의 와이어 타입을 쓴다.
pub mod tts;
// pub: contract 테스트(src-tauri/tests/contract_fixtures.rs)가 이 모듈의 wire
// 타입(SessionStateEvent 등)에 닿아야 한다. 로직 변경 없음 — 가시성만 승격.
pub mod types;
// pub: contract 테스트가 `agent_office_lib::usage::{UsageSnapshot, ...}`에 닿아야
// 한다. 로직 변경 없음 — 가시성만 승격.
pub mod usage;
mod vscode;
// pub: contract 테스트가 `agent_office_lib::workdir::{GitStatusResult, ...}`에
// 닿아야 한다(model.rs가 이미 `pub use model::*`로 재수출). 로직 변경 없음 —
// 가시성만 승격.
pub mod workdir;

use std::sync::{Arc, RwLock};
use std::time::Duration;

use tauri::{Emitter, Manager, RunEvent};
use tauri_plugin_window_state::{AppHandleExt as _, StateFlags};

use crate::notification::hub::{NotificationHub, SystemClock};
use crate::observer::server::ObserverServerState;
use crate::observer::ObserverRuntime;
use crate::persistence::png_store::{
    PngStore, MAX_MINIMI_BYTES, MAX_PORTRAIT_BYTES, MAX_SPRITE_BYTES,
};
use crate::persistence::profile_store::ProfileStore;
use crate::persistence::settings_store::{AppSettings, SettingsStore};
use crate::session::manager::SessionManager;
use crate::session::pty_factory::PortablePtyFactory;
use crate::state::*;

pub fn maybe_run_observer_forwarder<I, S>(args: I) -> Option<i32>
where
    I: IntoIterator<Item = S>,
    S: AsRef<std::ffi::OsStr>,
{
    let mut args = args.into_iter();
    let _program = args.next();
    let mode = args.next()?.as_ref().to_os_string();
    if mode.as_os_str() != std::ffi::OsStr::new("--observer-forward") {
        return None;
    }
    let provider = args.next()?.as_ref().to_os_string();
    // event는 claude만 동반한다(예: `--observer-forward claude Stop`).
    let event = args.next().map(|arg| arg.as_ref().to_os_string());
    // 잉여 인자가 있으면 알 수 없는 호출로 보고 forwarder를 타지 않는다.
    if args.next().is_some() {
        return None;
    }
    match provider.to_str() {
        // codex는 이벤트명을 body의 hook_event_name에서 얻으므로 인자로 받지 않는다.
        Some("codex") if event.is_none() => {
            Some(observer::forwarder::run_forwarder("codex", None))
        }
        Some("claude") => {
            // 이벤트가 있으면 유효한 유니코드여야 한다(비유니코드/파싱 실패는 무시).
            let event = match &event {
                Some(event) => match event.to_str() {
                    Some(event) => Some(event),
                    None => return None,
                },
                None => None,
            };
            Some(observer::forwarder::run_forwarder("claude", event))
        }
        _ => None,
    }
}

/// `--sessiond <socket_path>` 분기(unix 전용, docs/session-handoff-design.md
/// §아키텍처) -- 앱이 종료 시 세션을 넘길 데몬으로 자기 자신을 재실행할 때의
/// 진입점. `maybe_run_observer_forwarder`와 같은 패턴: 인자를 보고 데몬
/// 모드가 아니면 `None`을 돌려줘 `main.rs`가 평범한 `run()`으로 진행하게 한다.
#[cfg(unix)]
pub fn maybe_run_sessiond<I, S>(args: I) -> Option<i32>
where
    I: IntoIterator<Item = S>,
    S: AsRef<std::ffi::OsStr>,
{
    let mut args = args.into_iter();
    let _program = args.next();
    let mode = args.next()?.as_ref().to_os_string();
    if mode.as_os_str() != std::ffi::OsStr::new("--sessiond") {
        return None;
    }
    let socket_path = args.next()?.as_ref().to_os_string();
    if args.next().is_some() {
        return None;
    }
    Some(sessiond::daemon::run_daemon(std::path::PathBuf::from(
        socket_path,
    )))
}

/// Windows/기타: 세션 핸드오프는 unix 전용 기능이라 데몬 모드 자체가 없다.
#[cfg(not(unix))]
pub fn maybe_run_sessiond<I, S>(_args: I) -> Option<i32>
where
    I: IntoIterator<Item = S>,
    S: AsRef<std::ffi::OsStr>,
{
    None
}

/// `ctl <명령> …` 분기(이슈 #55, docs/cli-control-design.md) — 실행 중인 앱의
/// control 서버에 요청 1건을 보내는 단명 클라이언트. `maybe_run_observer_forwarder`
/// 와 같은 패턴: 첫 인자가 `ctl`이 아니면 `None`을 돌려줘 `main.rs`가 평범한
/// `run()`(GUI)으로 진행하게 한다. GUI 런타임에 도달하지 않으므로 두 번째
/// 사무실/서버가 뜨지 않는다.
pub fn maybe_run_cli<I, S>(args: I) -> Option<i32>
where
    I: IntoIterator<Item = S>,
    S: AsRef<std::ffi::OsStr>,
{
    let mut args = args.into_iter();
    let _program = args.next();
    let mode = args.next()?.as_ref().to_os_string();
    if mode.as_os_str() != std::ffi::OsStr::new("ctl") {
        return None;
    }
    // 나머지 토큰을 String으로. 비유니코드 인자는 lossy 변환(경로/텍스트가
    // 대부분이라 실무상 무해 — 필요하면 --app-data 등으로 명시 지정).
    let rest: Vec<String> = args
        .map(|a| a.as_ref().to_string_lossy().into_owned())
        .collect();
    Some(control::client::run(rest))
}

/// 훅이 forwarder로 재실행할 자기 자신의 경로. Linux AppImage에서는
/// `current_exe()`가 실행마다 바뀌는 `/tmp/.mount_*` 마운트 안을 가리켜, 세션
/// 핸드오프 후 앱을 재시작하면 훅 설정에 박힌 forwarder 경로 자체가 스테일해진다
/// (포트 스테일과 같은 §핵심 5 시나리오, PR #32 리뷰 지적). AppImage 런타임이
/// 주는 `$APPIMAGE`(원본 .AppImage의 안정 경로)를 우선한다 — AppImage는 인자를
/// 내부 바이너리로 그대로 전달하므로 `--observer-forward` 분기가 동일하게 동작한다.
fn forwarder_executable_path() -> std::path::PathBuf {
    if let Some(appimage) = std::env::var_os("APPIMAGE") {
        let path = std::path::PathBuf::from(appimage);
        // forwarder_shell_command가 절대 경로를 요구한다 — 이상한 값이면 무시.
        if path.is_absolute() {
            return path;
        }
    }
    std::env::current_exe().unwrap_or_default()
}

/// Returns the live observer endpoint only when the latest settings snapshot
/// enables observation and a server is currently installed.
/// `#[cfg(test)]` 아래에서 이 함수를 직접 단위 테스트한다.
fn make_observer_url_getter(
    settings: Arc<RwLock<AppSettings>>,
    server: Arc<ObserverServerState>,
) -> Arc<dyn Fn() -> Option<String> + Send + Sync> {
    Arc::new(move || {
        settings
            .read()
            .unwrap()
            .observer_enabled
            .then(|| server.current_url())
            .flatten()
    })
}

/// 패닉 관측성: Finder에서 실행된 .app은 stderr가 어디에도 남지 않아
/// 백그라운드 스레드/tokio 태스크의 패닉이 흔적 없이 사라진다(2026-07-11
/// "터미널 영구 고착" 사고의 원인 규명 실패 지점). 기본 훅(stderr 출력)을
/// 유지하면서 <app_data>/panic.log에 위치·메시지·백트레이스를 append한다.
fn install_panic_logger(data_dir: std::path::PathBuf) {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis())
            .unwrap_or(0);
        let thread = std::thread::current()
            .name()
            .unwrap_or("<unnamed>")
            .to_string();
        let backtrace = std::backtrace::Backtrace::force_capture();
        let entry = format!("=== panic @{ts}ms thread={thread}\n{info}\n{backtrace}\n\n");
        let _ = std::fs::create_dir_all(&data_dir);
        use std::io::Write as _;
        if let Ok(mut f) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(data_dir.join("panic.log"))
        {
            let _ = f.write_all(entry.as_bytes());
        }
        previous(info); // 기본 stderr 출력도 유지(dev 실행 시 즉시 보임)
    }));
}

fn session_event_root(data_dir: &std::path::Path) -> std::path::PathBuf {
    data_dir.join("session-events").join("v1")
}

/// 세션 브로커 v2(docs/session-broker-v2-design.md)의 PtyFactory 주입 결정.
/// `AGENT_OFFICE_SESSION_BROKER=v2` + unix일 때만 `BrokerPtyFactory`(데몬이
/// 스폰부터 PTY 소유)를 쓰고, 아니면 기존 `PortablePtyFactory`(프로세스 내
/// 직접 스폰). 반환값 `.1`은 broker 모드 여부 -- SessionManager의 앱 쪽 의미
/// 분기(handoff/adopt/스냅샷 업로드)에 그대로 넘긴다. 기본 off라 v1 경로가
/// 손대지 않은 채 보존된다.
fn make_pty_factory(
    data_dir: &std::path::Path,
) -> (Arc<dyn crate::session::pty_factory::PtyFactory>, bool) {
    let opt_in = std::env::var("AGENT_OFFICE_SESSION_BROKER")
        .map(|v| v == "v2")
        .unwrap_or(false);
    #[cfg(unix)]
    if opt_in {
        let fallback: Arc<dyn crate::session::pty_factory::PtyFactory> =
            Arc::new(PortablePtyFactory);
        return (
            Arc::new(crate::session::broker_pty::BrokerPtyFactory::new(data_dir, fallback)),
            true,
        );
    }
    let _ = (data_dir, opt_in);
    (Arc::new(PortablePtyFactory), false)
}

/// 창 상태 저장/복원 대상 플래그. **SIZE | POSITION | MAXIMIZED만** 쓴다:
///
/// - `VISIBLE`: main은 tauri.conf.json에서 항상 `visible: true`라 복원할 사용자
///   상태가 없다. 반대로 이 플래그가 켜지면 저장 시 `is_visible()`이 기록되는데,
///   macOS에서 앱을 숨기거나(⌘H) 최소화한 채 종료하면 `visible: false`가 남고
///   복원 경로가 `show()`/`set_focus()`를 건너뛴다. 지금은 설정이 창을 띄우므로
///   무해하지만, 창 표시 여부를 저장 파일에 결부시킬 이유가 없어 아예 뺀다.
/// - `FULLSCREEN`: 풀스크린으로 종료했다고 다음 실행도 풀스크린으로 강제하는 건
///   원하는 동작이 아니고, macOS에서는 크기/위치 복원과 겹쳐 어긋난다.
/// - `DECORATIONS`: main의 장식은 앱이 바꾸지 않으므로 저장할 상태가 없다.
///
/// 플러그인 등록과 종료 시 명시 저장이 **같은 플래그**를 써야 저장 파일의
/// 필드 구성이 어긋나지 않는다.
const WINDOW_STATE_FLAGS: StateFlags = StateFlags::SIZE
    .union(StateFlags::POSITION)
    .union(StateFlags::MAXIMIZED);

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        // main 창의 크기·위치·최대화 상태를 종료 시 저장하고 재실행 시 복원한다.
        // 마스코트 창(이슈 #72)은 자체 위치 복원 로직(src/renderer/mascot/position.ts,
        // localStorage + 모니터 유효성 검사)을 갖고 있어 플러그인이 건드리면 두
        // 주체가 좌표를 다투므로 denylist로 제외한다.
        .plugin(
            tauri_plugin_window_state::Builder::new()
                .with_state_flags(WINDOW_STATE_FLAGS)
                .with_denylist(&["mascot"])
                .build(),
        )
        // 네이티브 폴더 선택 다이얼로그(pick_directory) — Rust 측에서만 사용.
        .plugin(tauri_plugin_dialog::init())
        // OS 데스크탑 알림(이슈 #39) — 앱이 백그라운드일 때 프런트가 발송.
        .plugin(tauri_plugin_notification::init())
        .setup(|app| {
            let handle = app.handle().clone();
            let data_dir = app.path().app_data_dir()?;
            install_panic_logger(data_dir.clone());

            // 로그인 셸 env 1회 캡처(#58). 봇 시작·요약기 스폰 직전에도 부르지만
            // (멱등) 부팅에서 미리 해 둬야 세션 로그의 JSONL 전사 소스와 사용량
            // 조회가 `CLAUDE_CONFIG_DIR`/`CODEX_HOME` 오버라이드를 본다. 로그인
            // 셸을 돌리는 블로킹 호출이라 부팅을 막지 않게 백그라운드로 뺀다.
            std::thread::Builder::new()
                .name("env-capture".into())
                .spawn(crate::session::env_capture::ensure_captured)
                .ok();

            let event_store = Arc::new(crate::session_events::store::SessionEventStore::new(
                session_event_root(&data_dir),
            ));
            let tauri_events: Arc<dyn AppEvents> = Arc::new(TauriEvents {
                app: handle.clone(),
            });
            // 봇 주입 표식(kbm #2j8): 기록기와 봇 러너가 같은 Arc를 쥐어야
            // "주입 직전 arm → 다음 prompt가 소비"가 성립한다.
            let bot_arms = Arc::new(crate::state::BotPromptArms::new());
            let recording_events: Arc<dyn AppEvents> = Arc::new(
                crate::session_events::recording_events::RecordingAppEvents::new(
                    tauri_events,
                    event_store,
                    bot_arms.clone(),
                ),
            );
            // 웹 원격: 허브를 이벤트 배선보다 먼저 만들어, 붙어 있는
            // 캐릭터의 앱 이벤트가 `AppEvents` 단일 관문에서 그대로 미러되게 한다.
            let web_remote_hub = crate::webremote::host::WebRemoteHub::new();
            let web_remote_events: Arc<dyn AppEvents> =
                Arc::new(crate::webremote::host::WebRemoteEvents::new(web_remote_hub.clone()));
            let events: Arc<dyn AppEvents> = Arc::new(crate::state::CompositeEvents::new(
                recording_events,
                web_remote_events,
            ));
            let registry = Arc::new(SessionRegistry::new());
            let hub = Arc::new(NotificationHub::new(
                registry.clone(),
                events.clone(),
                Arc::new(SystemClock),
                Duration::from_millis(3000), // dedup 3s
            ));

            let settings_store = SettingsStore::new(data_dir.join("settings.json"));
            let (settings, settings_first_run) = settings_store.load();
            // 이슈 #41: 오토모드 질문 알림 홀드 시간을 설정에서 주입한다.
            hub.set_hold_duration(Duration::from_millis(settings.attention_hold_ms));
            hub.set_lang(crate::i18n::ui_lang(&settings));
            // 500ms 간격 단일 스위퍼로 만료된 보류 알림을 방출한다(훅별 타이머 없이).
            {
                let hub = hub.clone();
                tauri::async_runtime::spawn(async move {
                    let mut ticker = tokio::time::interval(Duration::from_millis(500));
                    loop {
                        ticker.tick().await;
                        hub.flush_expired();
                    }
                });
            }
            // 이슈 #42: 셸 출력 내보내기 임시 .txt 누적 방지 -- 부팅 시 1회,
            // 7일보다 오래된 파일을 백그라운드로 청소한다(부팅 블로킹 금지).
            std::thread::spawn(|| {
                shell_export::gc_old_exports(std::time::Duration::from_secs(7 * 24 * 3600))
            });
            // AppState가 갖는 캐시와 동일한 Arc를 observer URL getter 생성 전에
            // 만든다 -- 아래 getter가 이 Arc를 clone해 쥐고 있어야
            // set_app_settings의 실행 중 설정 변경(특히 ON→OFF)이 새 세션의
            // 훅 배선 여부에 즉시 반영된다(getter가 그때그때 최신 캐시를 읽음).
            let settings_cache = Arc::new(std::sync::RwLock::new(settings));

            let observer_server = Arc::new(ObserverServerState::default());
            // §핵심 5: 세션 재시작(입양) 후 훅이 스폰 시점의 죽은 포트를 치는
            // 문제 완화 -- forwarder가 읽는 <app_data_dir>/observer-port의 근거.
            observer_server.set_app_data_dir(data_dir.clone());
            // 이슈 #40: Claude 훅 설정 파일을 OS temp가 아니라 app_data의 안정
            // 경로에 둔다. OS temp는 앱이 꺼진 사이 시스템 청소로 사라질 수 있어,
            // 셸 env(`AGENT_OFFICE_SETTINGS`)가 가리키는 파일이 없어져 `claude
            // --settings <없는 파일>`이 하드 실패했다. app_data는 앱 수명주기가
            // 소유하며 입양 시 복구(restore_session_artifacts)로 재작성된다.
            let observer_settings_dir = data_dir.join("observer").join("claude");
            // 더블-크래시 등으로 정리 못 한 설정 아티팩트가 app_data에 영구화되지
            // 않도록 부트 시 1회 백그라운드로 30일 초과분을 청소한다(살아 있는
            // 세션은 매 입양마다 재작성돼 mtime이 갱신되므로 안전).
            {
                let dir = observer_settings_dir.clone();
                std::thread::spawn(move || {
                    crate::observer::claude::gc_stale_settings(
                        &dir,
                        Duration::from_secs(30 * 24 * 3600),
                    );
                });
            }
            // Claude 리줌 캡처(docs/claude-session-resume-design.md): 스토어 →
            // 레코더(sink) → observer runtime 순으로 배선. sink는 builder로 주입해
            // production() 시그니처를 건드리지 않는다.
            let claude_resume_store = Arc::new(
                crate::persistence::claude_resume_store::ClaudeResumeStore::new(
                    data_dir.join("claude-resume.json"),
                ),
            );
            let claude_resume_recorder =
                Arc::new(crate::observer::claude_resume_recorder::ClaudeResumeRecorder::new(
                    registry.clone(),
                    claude_resume_store.clone(),
                ));
            // 동료 대화(docs/agent-talk-design.md): 허브 + 스킬 자산 + 배달 워커.
            // 스킬(로컬 플러그인)은 토글과 무관하게 만들어 둔다 — 대화를 켠 뒤
            // 앱을 다시 띄우지 않아도 다음 세션부터 바로 붙게.
            let talk = Arc::new(crate::talk::TalkHub::default());
            talk.set_log_dir(data_dir.join("talks"));
            talk.set_events(events.clone());
            {
                let snapshot = settings_cache.read().unwrap();
                talk.set_config(crate::talk::TalkConfig {
                    max_turns: snapshot.talk_max_turns.max(1),
                    idle_quiet_ms: snapshot.talk_idle_quiet_ms,
                    lang: crate::i18n::ui_lang(&snapshot),
                });
                talk.set_enabled(snapshot.talk_enabled);
            }
            let talk_exe = forwarder_executable_path();
            let talk_cli = crate::talk::skill::shim_path();
            if let Err(error) = crate::talk::skill::ensure_assets(&data_dir, &talk_exe) {
                eprintln!("agent-office: talk skill assets failed: {error}");
            }
            // 세션마다 부르는 두 갈래: (1) 훅 설정에 얹을 조각(관찰 ON),
            // (2) talk 전용 설정 파일(관찰 OFF). 둘 다 대화가 꺼져 있으면 None.
            let talk_fragment = {
                let settings = settings_cache.clone();
                let data_dir = data_dir.clone();
                let exe = talk_exe.clone();
                Arc::new(move || {
                    if !settings.read().unwrap().talk_enabled {
                        return None;
                    }
                    let shim = crate::talk::skill::ensure_assets(&data_dir, &exe)
                        .unwrap_or_else(|_| crate::talk::skill::shim_path());
                    Some(crate::talk::skill::settings_fragment(&data_dir, &shim))
                }) as Arc<dyn Fn() -> Option<serde_json::Value> + Send + Sync>
            };
            let talk_wiring_provider = {
                let settings = settings_cache.clone();
                let data_dir = data_dir.clone();
                let settings_dir = observer_settings_dir.clone();
                let exe = talk_exe.clone();
                Arc::new(move |session_id: &str, has_settings: bool| {
                    if !settings.read().unwrap().talk_enabled {
                        return None;
                    }
                    let shim = crate::talk::skill::ensure_assets(&data_dir, &exe).ok()?;
                    // 훅 설정이 이미 있으면 권한 조각은 그쪽에 합쳐져 있다.
                    let settings_path = if has_settings {
                        None
                    } else {
                        crate::talk::skill::write_talk_only_settings(
                            &settings_dir,
                            session_id,
                            &data_dir,
                            &shim,
                        )
                        .map_err(|error| {
                            eprintln!("agent-office: talk settings write failed: {error}");
                        })
                        .ok()
                    };
                    Some(crate::session::manager::TalkWiring {
                        plugin_dir: crate::talk::skill::plugin_dir(&data_dir),
                        settings_path,
                    })
                })
                    as Arc<
                        dyn Fn(&str, bool) -> Option<crate::session::manager::TalkWiring>
                            + Send
                            + Sync,
                    >
            };

            let observer = Arc::new(
                ObserverRuntime::production_with(
                    hub.clone(),
                    observer_settings_dir.clone(),
                    forwarder_executable_path(),
                    Some(talk_fragment),
                )
                .with_claude_session_sink(claude_resume_recorder),
            );

            if settings_cache.read().unwrap().observer_enabled {
                let _ = tauri::async_runtime::block_on(observer_server.ensure(observer.clone()));
            }
            let get_observer_url =
                make_observer_url_getter(settings_cache.clone(), observer_server.clone());

            // 세션 로그(docs/session-log-design.md): 저장소 루트 + 설정 토글을
            // 미러하는 원자 플래그. 플래그는 set_app_settings가 갱신한다.
            let session_log_root = crate::session_log::store::root_for(&data_dir);
            let session_log_enabled = Arc::new(std::sync::atomic::AtomicBool::new(
                settings_cache.read().unwrap().session_log_enabled,
            ));
            crate::session_log::spawn_gc(session_log_root.clone());

            let (pty_factory, broker_mode) = make_pty_factory(&data_dir);
            let manager = Arc::new(
                SessionManager::new(
                    pty_factory,
                    observer.clone(),
                    registry.clone(),
                    events.clone(),
                    hub.clone(),
                    get_observer_url,
                )
                // 세션 핸드오프(unix 전용, docs/session-handoff-design.md) 소켓/로그
                // 경로와 AGENT_OFFICE_APP_DATA env 주입(§핵심 5)의 근거.
                .with_app_data_dir(data_dir.clone())
                // 동료 대화 스킬 배선(--plugin-dir + 권한 조각).
                .with_talk_wiring(talk_wiring_provider)
                // v2 상시 브로커 모드(opt-in, docs/session-broker-v2-design.md).
                .with_broker_mode(broker_mode)
                // 터미널 전사 상시 기록(30일·2GB 자율 보존).
                .with_session_log(session_log_root.clone(), session_log_enabled.clone())
                // 대체 화면(Claude Code)에서 도는 대화는 PTY로 안 보인다 --
                // 리줌 스토어의 세션 ID로 Claude 자체 JSONL 전사를 찾아 붙인다.
                .with_agent_session_lookup(claude_resume_store.clone()),
            );

            // 외부(논리) 세션 끊김 감지: attach를 요청한 셸이 사라지면 5초 안에
            // 캐릭터에서 뗀다. 셸의 EXIT trap은 사용자 trap을 덮어쓸 위험이 있어
            // 앱 쪽 `kill(pid, 0)` 폴링으로 감지한다(비unix는 no-op).
            {
                let manager = manager.clone();
                tauri::async_runtime::spawn(async move {
                    let mut ticker = tokio::time::interval(Duration::from_secs(5));
                    loop {
                        ticker.tick().await;
                        manager.sweep_externals();
                    }
                });
            }

            let store = ProfileStore::new(data_dir.join("profiles.json"));
            let portrait_store = Arc::new(PngStore::new(data_dir.join("portraits"), MAX_PORTRAIT_BYTES));
            let sprite_store = PngStore::new(data_dir.join("sprites"), MAX_SPRITE_BYTES);
            let minimi_store = PngStore::new(data_dir.join("minimis"), MAX_MINIMI_BYTES);
            let session_time_store = crate::persistence::session_time_store::SessionTimeStore::new(
                data_dir.join("session-times.jsonl"),
            );
            let diary_store =
                crate::persistence::diary_store::DiaryStore::new(data_dir.join("diaries"));
            let work_log_store =
                crate::persistence::work_log_store::WorkLogStore::new(data_dir.join("worklogs"));
            // 이 달의 우수사원: 문서 1개(`awards/awards.json`)와 초상 스냅샷
            // 하위 폴더(`awards/portraits/`)를 루트 하나 아래 함께 둔다.
            let awards_store =
                crate::persistence::awards_store::AwardsStore::new(data_dir.join("awards"));
            // 포스트잇 메모(#79): 캐릭터별 하위 폴더(`memos/<agentId>/`)를 갖는다.
            let memo_store =
                crate::persistence::memo_store::MemoStore::new(data_dir.join("memos"));
            // 확인 요청 대사 TTS: 키 파일(0600)과 mp3 캐시는 app_data 하위에 둔다.
            let tts = Arc::new(crate::tts::TtsState::new(&data_dir));

            // CLI 제어(#55): control 서버 상태 + 핸들러가 쥘 앱 상태 클론. 필요한
            // Arc/스토어만 복제해 ControlContext에 담는다(AppState는 Tauri가
            // 소유해 Arc로 꺼낼 수 없으므로). cli_enabled ON일 때만 지금 기동해
            // control-port를 기록한다(토큰=승인은 별도, 기본 미승인).
            let control_server = Arc::new(crate::control::ControlServerState::default());
            control_server.set_app_data_dir(data_dir.clone());
            let control_ctx = Arc::new(crate::control::ControlContext {
                manager: manager.clone(),
                observer: observer.clone(),
                observer_server: observer_server.clone(),
                hub: hub.clone(),
                registry: registry.clone(),
                store: store.clone(),
                settings: settings_cache.clone(),
                settings_store: settings_store.clone(),
                talk: talk.clone(),
                app_data_dir: data_dir.clone(),
                tmux_probe: crate::control::tmux::system_probe(),
            });
            if settings_cache.read().unwrap().cli_enabled {
                let _ = tauri::async_runtime::block_on(control_server.ensure(control_ctx.clone()));
            }

            // 웹 원격(docs/web-remote-design.md) — 별도 리스너/Router.
            // control 서버와 같은 2단계 옵트인이라 web_hosting_enabled가 켜져 있어도
            // 페어링 승인 전에는 모든 요청이 401이다.
            // 사용량 스로틀 상태는 네이티브 커맨드와 웹 RPC가 공유한다
            // (폰 폴링이 중복 fetch를 일으키지 않게).
            let live_usage = Arc::new(crate::usage::LiveUsageState::new());
            let web_remote_server = Arc::new(crate::webremote::WebRemoteServerState::default());
            let host_name = crate::webremote::local_host_name();
            let web_remote_ctx = Arc::new(crate::webremote::WebRemoteContext::new(
                crate::webremote::WebRemoteContextDeps {
                    manager: manager.clone(),
                    registry: registry.clone(),
                    store: store.clone(),
                    settings: settings_cache.clone(),
                    hub: web_remote_hub.clone(),
                    app_data_dir: data_dir.clone(),
                    host_name,
                    hub_notify: hub.clone(),
                    observer: observer.clone(),
                    observer_server: observer_server.clone(),
                    live_usage: live_usage.clone(),
                    portraits: portrait_store.clone(),
                },
            ));
            {
                // 채팅 뷰(M2)의 전사 소스 — 세션 로그와 **같은 파서·같은 위치
                // 탐색**을 쓰되 tailer는 별개다(읽기 전용이라 간섭이 없다).
                let lookup = claude_resume_store.clone();
                web_remote_ctx
                    .chat
                    .set_source_factory(Arc::new(move |_agent_id: &str, _cwd: &str| {
                        let mut sources: Vec<
                            Box<dyn crate::session_log::agent_transcript::TranscriptSource>,
                        > = Vec::new();
                        sources.extend(crate::session_log::agent_transcript::claude::source(
                            lookup.clone(),
                        ));
                        sources.extend(crate::session_log::agent_transcript::codex::source());
                        sources
                    }));
            }
            {
                // 브라우저가 처음 붙을 때 호스트 렌더러에 화면 직렬화를 요청하는 다리.
                let handle = handle.clone();
                web_remote_hub.snapshots.set_emitter(Arc::new(move |agent_id, request_id| {
                    let _ = handle.emit(
                        "web-remote-snapshot-request",
                        serde_json::json!({ "agentId": agent_id, "requestId": request_id }),
                    );
                }));
            }
            {
                // 페어링 요청이 오면 승인 다이얼로그를 띄운다.
                let handle = handle.clone();
                web_remote_ctx.set_pair_notify(Arc::new(move |pending| {
                    let _ = handle.emit(
                        "web-remote-pair-request",
                        serde_json::json!({
                            "pairingId": pending.pairing_id,
                            "code": pending.code,
                            "clientName": pending.client_name,
                            // 코드 수명 — 승인 다이얼로그가 자동 소멸에 쓴다.
                            "expiresInMs": pending.remaining_ms(),
                        }),
                    );
                }));
            }
            {
                let (needs_server, port) = {
                    let s = settings_cache.read().unwrap();
                    (s.web_remote_enabled, s.web_remote_port)
                };
                if needs_server {
                    let _ = tauri::async_runtime::block_on(
                        web_remote_server.ensure(web_remote_ctx.clone(), port),
                    );
                }
            }

            // 캐릭터 봇 모드(#57): 탭별 폴링 태스크 소유자 + 태스크가 쥘 상태 클론.
            // 봇 모드 자체는 런타임 상태(탭에서 켜야 시작)라 여기선 아무 태스크도
            // 띄우지 않는다 — start는 렌더러 bot_start 커맨드가 트리거한다.
            let bot_runtime = Arc::new(crate::bot::BotRuntime::default());
            let bot_ctx = Arc::new(crate::bot::runner::BotContext {
                manager: manager.clone(),
                store: store.clone(),
                state_store: crate::bot::state_store::BotStateStore::new(
                    data_dir.join("bot-state.json"),
                ),
                state_lock: Arc::new(std::sync::Mutex::new(())),
                bot_arms: bot_arms.clone(),
            });

            // 작업 중 잠자기 방지(#68): 웨이크락 소유자 + lease 만료 감시 태스크.
            // 렌더러가 set_keep_awake로 lease(180s)를 갱신하고, 이 태스크가 30초
            // 간격으로 tick해 렌더러가 크래시/행으로 통지를 멈추면 강제 해제한다.
            let wake_lock = Arc::new(crate::power::WakeLock::new());
            {
                let wake_lock = wake_lock.clone();
                tauri::async_runtime::spawn(async move {
                    loop {
                        tokio::time::sleep(std::time::Duration::from_secs(30)).await;
                        wake_lock.tick();
                    }
                });
            }

            // 배달 워커: 수신자가 한가해지면 큐의 메시지를 PTY에 주입한다.
            crate::talk::spawn_worker(
                talk.clone(),
                manager.clone(),
                talk_cli.to_string_lossy().into_owned(),
                {
                    let store = store.clone();
                    Arc::new(move |agent_id: &str| {
                        store
                            .load()
                            .agents
                            .into_iter()
                            .find(|a| a.id == agent_id)
                            .map(|a| a.role)
                    })
                },
            );

            app.manage(AppState {
                manager,
                hub,
                observer,
                observer_server,
                store,
                portrait_store,
                sprite_store,
                minimi_store,
                session_time_store,
                diary_store,
                work_log_store,
                awards_store,
                memo_store,
                claude_resume_store,
                settings_store,
                settings: settings_cache,
                settings_first_run: std::sync::atomic::AtomicBool::new(settings_first_run),
                session_event_root: session_event_root(&data_dir),
                session_log_root,
                session_log_enabled,
                live_usage: live_usage.clone(),
                control_server,
                control_ctx,
                web_remote_server,
                web_remote_ctx,
                bot_runtime,
                bot_ctx,
                wake_lock,
                tts,
                talk,
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            ipc::commands::create_session,
            ipc::commands::list_available_shells,
            ipc::commands::dispose_session,
            ipc::commands::detach_external_session,
            ipc::commands::handoff_supported,
            ipc::commands::handoff_sessions,
            ipc::commands::adopt_detached_sessions,
            ipc::commands::session_broker_mode,
            ipc::commands::upload_session_snapshots,
            ipc::commands::write_input,
            ipc::commands::resize_session,
            ipc::commands::subscribe_output,
            ipc::commands::unsubscribe_output,
            ipc::commands::list_notifications,
            ipc::commands::clear_notifications,
            ipc::commands::load_state,
            ipc::commands::save_state,
            ipc::commands::set_badge_count,
            ipc::commands::save_portrait,
            ipc::commands::load_portrait,
            ipc::commands::delete_portrait,
            ipc::commands::save_sprite,
            ipc::commands::load_sprite,
            ipc::commands::delete_sprite,
            ipc::commands::save_minimi,
            ipc::commands::load_minimi,
            ipc::commands::delete_minimi,
            ipc::commands::summarize_text,
            ipc::commands::list_provider_models,
            ipc::commands::codex_image_status,
            ipc::commands::generate_codex_image,
            ipc::commands::get_app_settings,
            ipc::commands::set_app_settings,
            ipc::commands::set_keep_awake,
            ipc::commands::set_mascot_visible,
            ipc::commands::mascot_activate,
            ipc::commands::set_mascot_layout,
            ipc::commands::tts_speak,
            ipc::commands::tts_list_voices,
            ipc::commands::tts_key_status,
            ipc::commands::tts_set_keys,
            ipc::commands::control_status,
            ipc::commands::control_approve,
            ipc::commands::control_revoke,
            // 웹 원격 — 호스트 역할
            ipc::commands::web_remote_status,
            ipc::commands::web_remote_pair_approve,
            ipc::commands::web_remote_pair_reject,
            ipc::commands::web_remote_revoke,
            ipc::commands::web_remote_set_permission,
            ipc::commands::web_remote_submit_snapshot,
            // tailscale serve 대행 — 웹 원격 HTTPS(포트 47443)
            ipc::commands::tailscale_serve_status,
            ipc::commands::tailscale_serve_enable,
            ipc::commands::tailscale_serve_disable,
                        ipc::commands::bot_start,
            ipc::commands::bot_stop,
            ipc::commands::bot_status,
            ipc::commands::open_in_vscode,
            ipc::commands::open_in_terminal,
            ipc::commands::open_in_file_manager,
            ipc::commands::export_terminal_output,
            markdown::markdown_list_files,
            markdown::markdown_read_file,
            markdown::markdown_write_file,
            workdir::workdir_list_files,
            workdir::workdir_search_files,
            workdir::workdir_git_status,
            workdir::workdir_git_branch,
            workdir::workdir_diff_file,
            workdir::workdir_file_history,
            workdir::workdir_diff_commit,
            workdir::workdir_commit_files,
            workdir::workdir_repo_log,
            workdir::workdir_git_cancel,
            workdir::workdir_difftool,
            ipc::commands::pick_directory,
            ipc::commands::export_character_file,
            ipc::commands::import_character_file,
            ipc::commands::export_diary_file,
            ipc::commands::append_session_turn,
            ipc::commands::load_session_turns,
            ipc::commands::append_diary_entry,
            ipc::commands::load_diary,
            ipc::commands::save_work_log,
            ipc::commands::load_work_logs,
            ipc::commands::talk_status,
            ipc::commands::list_talk_log_dates,
            ipc::commands::read_talk_log,
            ipc::commands::load_awards,
            ipc::commands::finalize_award,
            ipc::commands::append_award_speech,
            ipc::commands::load_award_portrait,
            ipc::commands::load_memo,
            ipc::commands::save_memo,
            ipc::commands::archive_memo_sheet,
            ipc::commands::list_memo_archive,
            ipc::commands::read_memo_sheet,
            ipc::commands::delete_memos,
            ipc::commands::load_session_events,
            ipc::commands::list_session_logs,
            ipc::commands::open_session_log,
            ipc::commands::generate_study_material,
            ipc::commands::list_claude_resume_sessions,
            ipc::commands::load_usage_snapshot,
        ])
        .build(tauri::generate_context!())
        .expect("failed to build tauri app")
        .run(|app, event| {
            // 마스코트 창(이슈 #72)이 살아 있으면 main을 닫아도 앱이 죽지 않는다 --
            // Tauri는 **모든** 창이 닫혀야 ExitRequested를 발화하는데, quitGuard가
            // main에 destroy()를 쓰므로 close 훅으로는 잡히지 않는다. main이
            // 파괴되는 순간 마스코트도 함께 파괴해 유령 창/좀비 프로세스를 막는다.
            if let RunEvent::WindowEvent {
                label,
                event: tauri::WindowEvent::Destroyed,
                ..
            } = &event
            {
                if label == "main" {
                    if let Some(mascot) = app.get_webview_window("mascot") {
                        let _ = mascot.destroy();
                    }
                }
            }
            // 앱 종료 -- 모든 PTY kill + settings 정리 + observer 서버 graceful
            // shutdown, 반드시 이 순서로 (dispose_all이 먼저 끝나야 axum
            // shutdown 신호를 보내도 이미 kill된 세션들의 마지막 hook POST가
            // 유실돼도 무해하다 -- 어차피 프로세스가 죽는 중이므로).
            if let RunEvent::ExitRequested { .. } = event {
                // 창 상태 저장을 **가장 먼저** — 종료 경로가 둘이라서다.
                // ① X 클릭: CloseRequested가 발화해 플러그인이 살아 있는 창을
                //    읽어 캐시를 갱신한다(quitGuard가 preventDefault해도 Rust
                //    리스너는 이미 돌았다).
                // ② quitGuard 확인 후 `destroy()`(ConfirmQuitDialog.tsx): 이땐
                //    CloseRequested가 재발화하지 않는다. 크기/위치는 Moved/Resized
                //    리스너가 실시간으로 캐시에 넣어 두므로, 여기서 그 캐시를
                //    디스크에 확정 기록하는 게 이 호출의 역할이다.
                // ⌘Q처럼 CloseRequested 없이 들어온 경로에서 창이 아직 살아
                // 있다면 실제 창을 읽어 최대화 여부까지 갱신한다(창이 이미
                // 파괴됐으면 플러그인이 그 창을 건너뛰고 캐시만 쓴다).
                // 실패는 로그만 — 종료를 막지 않는다.
                if let Err(e) = app.save_window_state(WINDOW_STATE_FLAGS) {
                    eprintln!("agent-office: 창 상태 저장 실패: {e}");
                }
                let state = app.state::<AppState>();
                state.manager.dispose_all(); // kill + settings cleanup(동기)
                state.observer_server.shutdown();
                state.control_server.shutdown(); // CLI 제어 서버 정지 + control-port 정리(#55)
                state.web_remote_server.shutdown(); // 웹 원격 수신 서버 정지
                state.bot_runtime.stop_all(); // 봇 폴링 태스크 정지(#57)
                state.wake_lock.deactivate(); // 잠자기 방지 해제(#68) — OS가 자동 회수도 하지만 이중 안전장치.
                // wait 스레드가 Disposed 확정 후 OS가 자식 reap. 프로세스 종료는 정상 진행.
            }
        });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::persistence::settings_store::SummaryProvider;

    #[test]
    fn session_event_root_is_versioned_under_app_data() {
        let root = session_event_root(std::path::Path::new("/app-data"));
        assert_eq!(root, std::path::Path::new("/app-data/session-events/v1"));
    }

    // APPIMAGE는 프로세스 전역 env — 병렬 테스트 경합 방지용 직렬화 락
    // (observer/forwarder.rs의 ENV_LOCK과 동일 관례).
    static APPIMAGE_ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    #[test]
    fn forwarder_executable_prefers_absolute_appimage_path() {
        let _guard = APPIMAGE_ENV_LOCK.lock().unwrap();
        let previous = std::env::var_os("APPIMAGE");

        std::env::set_var("APPIMAGE", "/opt/apps/agent-office.AppImage");
        assert_eq!(
            forwarder_executable_path(),
            std::path::PathBuf::from("/opt/apps/agent-office.AppImage"),
        );

        // 상대 경로 APPIMAGE는 무시하고 current_exe로 폴백한다
        // (forwarder_shell_command가 절대 경로를 요구).
        std::env::set_var("APPIMAGE", "relative.AppImage");
        assert_eq!(
            forwarder_executable_path(),
            std::env::current_exe().unwrap_or_default(),
        );

        match previous {
            Some(value) => std::env::set_var("APPIMAGE", value),
            None => std::env::remove_var("APPIMAGE"),
        }
    }

    #[test]
    fn forwarder_executable_without_appimage_uses_current_exe() {
        let _guard = APPIMAGE_ENV_LOCK.lock().unwrap();
        let previous = std::env::var_os("APPIMAGE");
        std::env::remove_var("APPIMAGE");

        assert_eq!(
            forwarder_executable_path(),
            std::env::current_exe().unwrap_or_default(),
        );

        if let Some(value) = previous {
            std::env::set_var("APPIMAGE", value);
        }
    }

    // forwarder를 실제로 기동하는 Some 분기(codex, claude[+event])는 세션 env에
    // 의존하므로 여기서는 "forwarder 모드 아님"을 정확히 판별하는 None 분기만 본다.
    #[test]
    fn maybe_run_observer_forwarder_rejects_non_forwarder_invocations() {
        // --observer-forward가 아니거나 provider가 없으면 None.
        assert_eq!(maybe_run_observer_forwarder(["agent-office"]), None);
        assert_eq!(
            maybe_run_observer_forwarder(["agent-office", "--observer-forward"]),
            None,
        );
        assert_eq!(
            maybe_run_observer_forwarder(["agent-office", "--sessiond", "codex"]),
            None,
        );
        // 알 수 없는 provider는 None.
        assert_eq!(
            maybe_run_observer_forwarder(["agent-office", "--observer-forward", "unknown"]),
            None,
        );
        // codex는 이벤트 인자를 받지 않는다(잉여 인자 → None).
        assert_eq!(
            maybe_run_observer_forwarder(["agent-office", "--observer-forward", "codex", "Stop"]),
            None,
        );
        // claude라도 이벤트가 2개 이상이면 None.
        assert_eq!(
            maybe_run_observer_forwarder([
                "agent-office",
                "--observer-forward",
                "claude",
                "Stop",
                "extra",
            ]),
            None,
        );
    }

    #[tokio::test]
    async fn observer_url_getter_reflects_live_settings_cache_after_server_started() {
        let settings_cache = Arc::new(RwLock::new(AppSettings {
            version: 1,
            language: "system".to_string(),
            summarizer_enabled: false,
            summary_provider: SummaryProvider::Claude,
            summary_models: Default::default(),
            diary_enabled: false,
            observer_enabled: true,
            typing_sound_enabled: true,
            notify_sound_enabled: true,
            sound_volume: 0.5,
            external_terminal: Default::default(),
            external_editor: Default::default(),
            attention_hold_ms: 5000,
            git_status_enabled: true,
            workdir_show_ignored: false,
            file_index_backend: Default::default(),
            cli_enabled: false,
            keep_awake_enabled: false,
            session_log_enabled: true,
            mascot_enabled: false,
            mascot_lights_mode: Default::default(),
            mascot_lights_vertical: false,
            mascot_lights_projects: Vec::new(),
            tts_enabled: false,
            tts_rewrite_model_anthropic: "claude-haiku-4-5".to_string(),
            tts_rewrite_model_openrouter: "openai/gpt-5.4-mini".to_string(),
            tts_rewrite_provider: Default::default(),
            web_remote_bind: Default::default(),
            web_remote_port: crate::webremote::protocol::DEFAULT_WEB_REMOTE_PORT,
            web_remote_enabled: false,
            talk_enabled: false,
            talk_max_turns: crate::talk::DEFAULT_MAX_TURNS,
            talk_idle_quiet_ms: crate::talk::DEFAULT_IDLE_QUIET_MS,
        }));
        let registry = Arc::new(SessionRegistry::new());
        let events: Arc<dyn AppEvents> = Arc::new(crate::state::fake::RecordingEvents::default());
        let hub = Arc::new(NotificationHub::new(
            registry,
            events,
            Arc::new(SystemClock),
            Duration::from_millis(3_000),
        ));
        let observer = Arc::new(observer::ObserverRuntime::new(hub, vec![]));
        let server = Arc::new(observer::server::ObserverServerState::default());
        assert!(server.ensure(observer).await.is_some());
        let expected_url = server.current_url();
        let get_url = make_observer_url_getter(settings_cache.clone(), server.clone());

        assert_eq!(get_url(), expected_url);

        settings_cache.write().unwrap().observer_enabled = false;
        assert_eq!(get_url(), None);
        assert_eq!(server.current_url(), expected_url);

        settings_cache.write().unwrap().observer_enabled = true;
        assert_eq!(get_url(), expected_url);
        server.shutdown();
    }

    // maybe_run_sessiond의 실제 데몬 기동(Some 분기)은 daemon.rs/client.rs가
    // run_daemon(_inner)를 직접 구동해 검증한다 -- 여기서는 인자 파싱이
    // "데몬 모드 아님"을 정확히 판별하는지(None 분기)만 확인한다.
    // maybe_run_cli: 첫 인자가 `ctl`이 아니면 None(=GUI로 진행). `ctl help`는
    // 네트워크·파일 접근 없이 usage를 찍고 Some(0)을 돌려주는 안전한 분기라
    // 여기서 라우팅만 확인한다(실 요청 경로는 client.rs 단위 테스트가 담당).
    #[test]
    fn maybe_run_cli_routes_only_the_ctl_subcommand() {
        assert_eq!(maybe_run_cli(["agent-office"]), None);
        assert_eq!(maybe_run_cli(["agent-office", "--observer-forward"]), None);
        assert_eq!(maybe_run_cli(["agent-office", "--sessiond", "/tmp/x.sock"]), None);
        assert_eq!(maybe_run_cli(["agent-office", "ctl", "help"]), Some(0));
        assert_eq!(maybe_run_cli(["agent-office", "ctl"]), Some(0));
    }

    #[cfg(unix)]
    #[test]
    fn maybe_run_sessiond_returns_none_for_non_daemon_invocations() {
        assert_eq!(maybe_run_sessiond(["agent-office"]), None);
        assert_eq!(maybe_run_sessiond(["agent-office", "--observer-forward"]), None);
        assert_eq!(maybe_run_sessiond(["agent-office", "--sessiond"]), None);
        assert_eq!(
            maybe_run_sessiond([
                "agent-office",
                "--sessiond",
                "/tmp/x.sock",
                "extra",
            ]),
            None
        );
    }
}
