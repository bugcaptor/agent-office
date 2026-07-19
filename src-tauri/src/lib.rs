// src-tauri/src/lib.rs
//
// Bootstrap: settings load -> observer server started only when the opt-in
// setting is ON (binds to port 0 so the OS picks a free port; retries once
// on bind failure) -> session manager wiring -> AppState managed ->
// invoke_handler for the renderer-facing commands -> graceful quit on
// RunEvent::ExitRequested (dispose_all -> observer server shutdown).
pub mod api_keys;
mod ipc;
mod markdown;
mod notification;
mod observer;
mod persistence;
pub mod pixellab;
mod session;
mod session_events;
mod shell_export;
#[cfg(unix)]
mod sessiond;
mod state;
mod summarizer;
mod terminal;
mod types;
mod usage;
mod vscode;
mod workdir;

use std::sync::{Arc, RwLock};
use std::time::Duration;

use tauri::{Manager, RunEvent};

use crate::notification::hub::{NotificationHub, SystemClock};
use crate::observer::server::ObserverServerState;
use crate::observer::ObserverRuntime;
use crate::persistence::png_store::{PngStore, MAX_PORTRAIT_BYTES, MAX_SPRITE_BYTES};
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

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        // 네이티브 폴더 선택 다이얼로그(pick_directory) — Rust 측에서만 사용.
        .plugin(tauri_plugin_dialog::init())
        // OS 데스크탑 알림(이슈 #39) — 앱이 백그라운드일 때 프런트가 발송.
        .plugin(tauri_plugin_notification::init())
        .setup(|app| {
            let handle = app.handle().clone();
            let data_dir = app.path().app_data_dir()?;
            install_panic_logger(data_dir.clone());

            let event_store = Arc::new(crate::session_events::store::SessionEventStore::new(
                session_event_root(&data_dir),
            ));
            let tauri_events: Arc<dyn AppEvents> = Arc::new(TauriEvents {
                app: handle.clone(),
            });
            let events: Arc<dyn AppEvents> = Arc::new(
                crate::session_events::recording_events::RecordingAppEvents::new(
                    tauri_events,
                    event_store,
                ),
            );
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
            let observer = Arc::new(
                ObserverRuntime::production(
                    hub.clone(),
                    observer_settings_dir,
                    forwarder_executable_path(),
                )
                .with_claude_session_sink(claude_resume_recorder),
            );

            if settings_cache.read().unwrap().observer_enabled {
                let _ = tauri::async_runtime::block_on(observer_server.ensure(observer.clone()));
            }
            let get_observer_url =
                make_observer_url_getter(settings_cache.clone(), observer_server.clone());

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
                // v2 상시 브로커 모드(opt-in, docs/session-broker-v2-design.md).
                .with_broker_mode(broker_mode),
            );

            let store = ProfileStore::new(data_dir.join("profiles.json"));
            let portrait_store = PngStore::new(data_dir.join("portraits"), MAX_PORTRAIT_BYTES);
            let sprite_store = PngStore::new(data_dir.join("sprites"), MAX_SPRITE_BYTES);
            let session_time_store = crate::persistence::session_time_store::SessionTimeStore::new(
                data_dir.join("session-times.jsonl"),
            );

            app.manage(AppState {
                manager,
                hub,
                observer,
                observer_server,
                store,
                portrait_store,
                sprite_store,
                session_time_store,
                claude_resume_store,
                settings_store,
                settings: settings_cache,
                settings_first_run: std::sync::atomic::AtomicBool::new(settings_first_run),
                session_event_root: session_event_root(&data_dir),
                live_usage: crate::usage::LiveUsageState::new(),
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            ipc::commands::create_session,
            ipc::commands::list_available_shells,
            ipc::commands::dispose_session,
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
            ipc::commands::summarize_text,
            ipc::commands::generate_sprite_image,
            ipc::commands::get_app_settings,
            ipc::commands::set_app_settings,
            ipc::commands::open_in_vscode,
            ipc::commands::open_in_terminal,
            ipc::commands::export_terminal_output,
            markdown::markdown_list_files,
            markdown::markdown_read_file,
            markdown::markdown_write_file,
            workdir::workdir_list_files,
            workdir::workdir_git_status,
            ipc::commands::pick_directory,
            ipc::commands::append_session_turn,
            ipc::commands::load_session_turns,
            ipc::commands::load_session_events,
            ipc::commands::list_claude_resume_sessions,
            ipc::commands::load_usage_snapshot,
        ])
        .build(tauri::generate_context!())
        .expect("failed to build tauri app")
        .run(|app, event| {
            // 앱 종료 -- 모든 PTY kill + settings 정리 + observer 서버 graceful
            // shutdown, 반드시 이 순서로 (dispose_all이 먼저 끝나야 axum
            // shutdown 신호를 보내도 이미 kill된 세션들의 마지막 hook POST가
            // 유실돼도 무해하다 -- 어차피 프로세스가 죽는 중이므로).
            if let RunEvent::ExitRequested { .. } = event {
                let state = app.state::<AppState>();
                state.manager.dispose_all(); // kill + settings cleanup(동기)
                state.observer_server.shutdown();
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
            summarizer_enabled: false,
            summary_provider: SummaryProvider::Claude,
            observer_enabled: true,
            sound_enabled: true,
            sound_volume: 0.5,
            external_terminal: Default::default(),
            external_editor: Default::default(),
            attention_hold_ms: 5000,
            git_status_enabled: true,
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
