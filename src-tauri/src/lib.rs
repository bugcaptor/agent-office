// src-tauri/src/lib.rs
//
// Bootstrap: settings load -> observer server started only when the opt-in
// setting is ON (binds to port 0 so the OS picks a free port; retries once
// on bind failure) -> session manager wiring -> AppState managed ->
// invoke_handler for the renderer-facing commands -> graceful quit on
// RunEvent::ExitRequested (dispose_all -> observer server shutdown).
pub mod api_keys;
mod ipc;
mod notification;
mod observer;
mod persistence;
pub mod pixellab;
mod session;
mod session_events;
#[cfg(unix)]
mod sessiond;
mod state;
mod summarizer;
mod terminal;
mod types;
mod vscode;

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
    let provider = args.next()?.as_ref().to_os_string();
    if args.next().is_some() {
        return None;
    }
    if mode.as_os_str() == std::ffi::OsStr::new("--observer-forward")
        && provider.as_os_str() == std::ffi::OsStr::new("codex")
    {
        Some(observer::forwarder::run_codex_forwarder())
    } else {
        None
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

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        // 네이티브 폴더 선택 다이얼로그(pick_directory) — Rust 측에서만 사용.
        .plugin(tauri_plugin_dialog::init())
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
            // AppState가 갖는 캐시와 동일한 Arc를 observer URL getter 생성 전에
            // 만든다 -- 아래 getter가 이 Arc를 clone해 쥐고 있어야
            // set_app_settings의 실행 중 설정 변경(특히 ON→OFF)이 새 세션의
            // 훅 배선 여부에 즉시 반영된다(getter가 그때그때 최신 캐시를 읽음).
            let settings_cache = Arc::new(std::sync::RwLock::new(settings));

            let observer_server = Arc::new(ObserverServerState::default());
            // §핵심 5: 세션 재시작(입양) 후 훅이 스폰 시점의 죽은 포트를 치는
            // 문제 완화 -- forwarder가 읽는 <app_data_dir>/observer-port의 근거.
            observer_server.set_app_data_dir(data_dir.clone());
            let observer_temp = app
                .path()
                .temp_dir()
                .unwrap_or_else(|error| {
                    eprintln!("observer temp directory unavailable, using OS temp: {error}");
                    std::env::temp_dir()
                })
                .join("agent-office")
                .join("observer")
                .join("claude");
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
                    observer_temp,
                    std::env::current_exe().unwrap_or_default(),
                )
                .with_claude_session_sink(claude_resume_recorder),
            );

            if settings_cache.read().unwrap().observer_enabled {
                let _ = tauri::async_runtime::block_on(observer_server.ensure(observer.clone()));
            }
            let get_observer_url =
                make_observer_url_getter(settings_cache.clone(), observer_server.clone());

            let manager = Arc::new(
                SessionManager::new(
                    Arc::new(PortablePtyFactory),
                    observer.clone(),
                    registry.clone(),
                    events.clone(),
                    hub.clone(),
                    get_observer_url,
                )
                // 세션 핸드오프(unix 전용, docs/session-handoff-design.md) 소켓/로그
                // 경로와 AGENT_OFFICE_APP_DATA env 주입(§핵심 5)의 근거.
                .with_app_data_dir(data_dir.clone()),
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
            ipc::commands::pick_directory,
            ipc::commands::append_session_turn,
            ipc::commands::load_session_turns,
            ipc::commands::load_session_events,
            ipc::commands::list_claude_resume_sessions,
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
