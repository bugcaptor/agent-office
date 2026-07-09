// src-tauri/src/lib.rs
//
// Bootstrap: settings load -> hook server started only when the opt-in
// setting is ON (binds to port 0 so the OS picks a free port; retries once
// on bind failure) -> session manager wiring -> AppState managed ->
// invoke_handler for the renderer-facing commands -> graceful quit on
// RunEvent::ExitRequested (dispose_all -> hook server shutdown).
pub mod api_keys;
mod claude_cli;
mod ipc;
mod notification;
mod persistence;
pub mod pixellab;
mod session;
mod state;
mod types;
mod vscode;

use std::sync::{Arc, RwLock};
use std::time::Duration;

use tauri::{Manager, RunEvent};

use crate::notification::hook_server;
use crate::notification::hook_settings::HookSettingsWriter;
use crate::notification::hub::{NotificationHub, SystemClock};
use crate::persistence::png_store::{PngStore, MAX_PORTRAIT_BYTES, MAX_SPRITE_BYTES};
use crate::persistence::profile_store::ProfileStore;
use crate::persistence::settings_store::{AppSettings, SettingsStore};
use crate::session::manager::SessionManager;
use crate::session::pty_factory::PortablePtyFactory;
use crate::state::*;

/// 세션에 넘길 훅 포트 getter를 만든다. 캐시(`settings_cache`)가 그 순간
/// OFF면 서버가 이미 떠 있어(`hook_port`가 Some) 있어도 None을 돌려줘 새
/// 세션에 훅 배선(--settings·env·ZDOTDIR)을 주입하지 않는다 -- 실행 중
/// ON→OFF 전환(`set_app_settings`)이 다음 세션부터 실제로 반영되는 지점.
/// `#[cfg(test)]` 아래에서 이 함수를 직접 단위 테스트한다.
fn make_hook_port_getter(
    settings_cache: Arc<RwLock<AppSettings>>,
    hook_port: Arc<RwLock<Option<u16>>>,
) -> Arc<dyn Fn() -> Option<u16> + Send + Sync> {
    Arc::new(move || {
        if settings_cache.read().unwrap().claude_hooks_enabled {
            *hook_port.read().unwrap()
        } else {
            None
        }
    })
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .setup(|app| {
            let handle = app.handle().clone();
            let events: Arc<dyn AppEvents> = Arc::new(TauriEvents {
                app: handle.clone(),
            });
            let registry = Arc::new(SessionRegistry::new());
            let hub = Arc::new(NotificationHub::new(
                registry.clone(),
                events.clone(),
                Arc::new(SystemClock),
                Duration::from_millis(3000), // dedup 3s
            ));

            let data_dir = app.path().app_data_dir()?; // (기존 위치에서 앞으로 이동)
            let settings_store = SettingsStore::new(data_dir.join("settings.json"));
            let (settings, settings_first_run) = settings_store.load();
            // AppState가 갖는 캐시와 동일한 Arc를 훅 포트 getter 생성 전에
            // 만든다 -- 아래 get_port 클로저가 이 Arc를 clone해 쥐고 있어야
            // set_app_settings의 실행 중 설정 변경(특히 ON→OFF)이 새 세션의
            // 훅 배선 여부에 즉시 반영된다(getter가 그때그때 최신 캐시를 읽음).
            let settings_cache = Arc::new(std::sync::RwLock::new(settings));

            // 훅 서버는 opt-in: 설정 ON일 때만 기동. OFF면 포트 None — 세션들은
            // 훅 배선 없이 뜬다. 실행 중 ON 전환은 set_app_settings의 지연 기동.
            let hook_port: Arc<std::sync::RwLock<Option<u16>>> = Arc::new(std::sync::RwLock::new(None));
            let (shutdown_tx, server_handle) = if settings_cache.read().unwrap().claude_hooks_enabled {
                let (port, tx, handle) = tauri::async_runtime::block_on(
                    hook_server::serve_with_retry(|rx| hook_server::serve(hub.clone(), rx)),
                )?;
                *hook_port.write().unwrap() = Some(port);
                (Some(tx), Some(handle))
            } else {
                (None, None)
            };

            let temp = app.path().temp_dir()?.join("agent-office").join("hooks");
            let hook_writer = HookSettingsWriter::new(temp);
            let get_port = make_hook_port_getter(settings_cache.clone(), hook_port.clone());

            let manager = Arc::new(SessionManager::new(
                Arc::new(PortablePtyFactory),
                hook_writer,
                registry.clone(),
                events.clone(),
                hub.clone(),
                get_port,
            ));

            let store = ProfileStore::new(data_dir.join("profiles.json"));
            let portrait_store = PngStore::new(data_dir.join("portraits"), MAX_PORTRAIT_BYTES);
            let sprite_store = PngStore::new(data_dir.join("sprites"), MAX_SPRITE_BYTES);

            app.manage(AppState {
                manager,
                hub,
                store,
                portrait_store,
                sprite_store,
                settings_store,
                settings: settings_cache,
                settings_first_run: std::sync::atomic::AtomicBool::new(settings_first_run),
                hook_port,
                hook_shutdown: std::sync::Mutex::new(shutdown_tx),
                server_handle: std::sync::Mutex::new(server_handle),
            });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            ipc::commands::create_session,
            ipc::commands::list_available_shells,
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
        ])
        .build(tauri::generate_context!())
        .expect("failed to build tauri app")
        .run(|app, event| {
            // 앱 종료 -- 모든 PTY kill + settings 정리 + hook 서버 graceful
            // shutdown, 반드시 이 순서로 (dispose_all이 먼저 끝나야 axum
            // shutdown 신호를 보내도 이미 kill된 세션들의 마지막 hook POST가
            // 유실돼도 무해하다 -- 어차피 프로세스가 죽는 중이므로).
            if let RunEvent::ExitRequested { .. } = event {
                let state = app.state::<AppState>();
                state.manager.dispose_all(); // kill + settings cleanup(동기)

                // `.take()`의 결과를 먼저 별도 바인딩으로 떼어낸다: `if let` scrutinee에서
                // 바로 `.lock().unwrap().take()`를 호출하면 MutexGuard 임시값의 수명이
                // if-let 블록 끝까지 연장되어, 블록을 벗어나며 `state`(빌림의 원본)보다
                // 늦게 drop되려다 borrowck에 걸린다(E0597).
                let shutdown_tx = state.hook_shutdown.lock().unwrap().take();
                if let Some(tx) = shutdown_tx {
                    let _ = tx.send(()); // axum graceful shutdown 트리거
                }
                // 서버 task의 JoinHandle을 떼어내 detach한다: graceful shutdown은
                // 신호(oneshot)만으로 트리거되고 완료를 기다릴 필요가 없다(곧 프로세스가
                // 종료됨). 필드에서 꺼내는 것 자체가 "AppState 소멸 시 정확히
                // 한 번 소비" 계약을 지킨다 -- join하지 않고 버려도 axum task는 자체
                // 스폰된 상태라 프로세스 종료를 막지 않는다.
                let _ = state.server_handle.lock().unwrap().take();
                // wait 스레드가 Disposed 확정 후 OS가 자식 reap. 프로세스 종료는 정상 진행.
            }
        });
}

#[cfg(test)]
mod tests {
    use super::*;

    // 회귀 테스트: 실행 중 훅 opt-in을 ON→OFF로 바꾸면(캐시만 갱신, 서버는
    // 유지) getter가 즉시 None을 돌려줘야 새 세션에 훅 배선이 주입되지
    // 않는다. 이 테스트가 없던 이전 구현은 hook_port만 읽는 getter였고,
    // 서버가 한 번 뜨고 나면 캐시를 OFF로 바꿔도 계속 Some(port)를 돌려줬다.
    #[test]
    fn hook_port_getter_reflects_live_settings_cache_off_after_server_started() {
        let hook_port: Arc<RwLock<Option<u16>>> = Arc::new(RwLock::new(Some(4321)));
        let settings_cache = Arc::new(RwLock::new(AppSettings {
            version: 1,
            claude_cli_enabled: false,
            claude_hooks_enabled: true,
        }));
        let get_port = make_hook_port_getter(settings_cache.clone(), hook_port.clone());

        // ON + 서버 기동됨(hook_port = Some) -> 포트를 그대로 돌려준다.
        assert_eq!(get_port(), Some(4321));

        // 실행 중 OFF로 전환 -- 서버는 여전히 살아 있어 hook_port는 Some인
        // 채지만, getter는 캐시를 봐야 하므로 None을 돌려줘야 한다.
        settings_cache.write().unwrap().claude_hooks_enabled = false;
        assert_eq!(get_port(), None);
        assert!(
            hook_port.read().unwrap().is_some(),
            "OFF 전환이 서버 자체를 내리지는 않는다 -- hook_port는 그대로 Some"
        );

        // 다시 ON으로 전환하면 즉시 포트를 돌려준다(서버 재기동 없이 재사용).
        settings_cache.write().unwrap().claude_hooks_enabled = true;
        assert_eq!(get_port(), Some(4321));
    }
}
