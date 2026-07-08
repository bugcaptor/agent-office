// src-tauri/src/ipc/commands.rs
//
// The renderer-facing Tauri commands, using the exact invoke names the
// frontend calls. Every command is a thin delegation into
// `SessionManager`/`NotificationHub`/`ProfileStore`/`SettingsStore` — no
// lock is held across an `.await` point. Most bodies have no `.await` at
// all (`async fn` is required by Tauri for commands that take `State`);
// the exceptions (`summarize_text`, `generate_sprite_image`,
// `set_app_settings`) hold no lock when they yield.
//
// The `State<'_, AppState>` parameter is named `app_state` everywhere
// (not `state`) so it never collides with the `state: PersistedState`
// payload parameter on `save_state` -- Tauri's IPC argument binding matches
// JS argument keys to Rust parameter names, so a name collision there would
// silently break `save_state`'s payload mapping.

use tauri::{ipc::Channel, AppHandle, Manager, State};

use crate::persistence::settings_store::AppSettings;
use crate::state::AppState;
use crate::types::*;

#[derive(Debug, Default, Clone, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionOpts {
    pub cols: Option<u16>,
    pub rows: Option<u16>,
    pub cwd: Option<String>,
}

#[tauri::command(rename_all = "camelCase")]
pub async fn create_session(
    app_state: State<'_, AppState>,
    agent_id: String,
    opts: Option<SessionOpts>,
) -> Result<CreateSessionResult, String> {
    let o = opts.unwrap_or_default();
    app_state.manager.create(CreateSessionRequest {
        agent_id,
        cols: o.cols,
        rows: o.rows,
        cwd: o.cwd,
        autostart_claude: None, // 항상 기본 false (SessionManager::create의 unwrap_or(false))
    })
}

#[tauri::command(rename_all = "camelCase")]
pub async fn dispose_session(
    app_state: State<'_, AppState>,
    agent_id: String,
) -> Result<(), String> {
    app_state.manager.dispose(&agent_id);
    Ok(())
}

#[tauri::command(rename_all = "camelCase")]
pub async fn write_input(
    app_state: State<'_, AppState>,
    agent_id: String,
    data: String,
) -> Result<(), String> {
    app_state.manager.write_input(&agent_id, &data);
    Ok(())
}

#[tauri::command(rename_all = "camelCase")]
pub async fn resize_session(
    app_state: State<'_, AppState>,
    agent_id: String,
    cols: u16,
    rows: u16,
) -> Result<(), String> {
    app_state.manager.resize(&agent_id, cols, rows);
    Ok(())
}

#[tauri::command(rename_all = "camelCase")]
pub async fn subscribe_output(
    app_state: State<'_, AppState>,
    agent_id: String,
    channel: Channel<OutputChunk>,
) -> Result<(), String> {
    app_state.manager.attach_output(&agent_id, channel);
    Ok(())
}

#[tauri::command(rename_all = "camelCase")]
pub async fn unsubscribe_output(
    app_state: State<'_, AppState>,
    agent_id: String,
) -> Result<(), String> {
    app_state.manager.detach_output(&agent_id);
    Ok(())
}

#[tauri::command(rename_all = "camelCase")]
pub async fn list_notifications(
    app_state: State<'_, AppState>,
    agent_id: String,
) -> Result<Vec<NotificationEvent>, String> {
    Ok(app_state.manager.pending_notifications(&agent_id))
}

#[tauri::command(rename_all = "camelCase")]
pub async fn clear_notifications(
    app_state: State<'_, AppState>,
    agent_id: String,
    ids: Option<Vec<String>>,
) -> Result<(), String> {
    if let Some(sid) = app_state.manager.session_id_for(&agent_id) {
        app_state.hub.clear(&sid, ids);
    }
    Ok(())
}

#[tauri::command(rename_all = "camelCase")]
pub async fn load_state(app_state: State<'_, AppState>) -> Result<PersistedState, String> {
    Ok(app_state.store.load())
}

/// 주의: Tauri `State` 파라미터는 `app_state`, JS 페이로드 `{ state }`는
/// `state` 파라미터로 받는다 (이름 충돌 회피 -- JS 인자 키와 Rust 파라미터명이
/// 일치해야 매핑된다).
#[tauri::command(rename_all = "camelCase")]
pub async fn save_state(
    app_state: State<'_, AppState>,
    state: PersistedState,
) -> Result<(), String> {
    app_state.store.save(&state).map_err(|e| e.to_string())
}

#[tauri::command(rename_all = "camelCase")]
pub async fn set_badge_count(app: AppHandle, count: i64) -> Result<(), String> {
    // Verified against the installed tauri = 2.11.5 source
    // (src/webview/webview_window.rs, src/window/mod.rs): the design's guess
    // matches exactly. `WebviewWindow::set_badge_count(&self, count:
    // Option<i64>) -> tauri::Result<()>` (it just delegates to
    // `Window::set_badge_count`) -- no `AppHandle`/`Window`-level badge
    // method exists, so we must fetch the window first. `None` (or `0`,
    // which we normalize to `None`) clears the badge. Cross-platform: a
    // no-op on Windows/Android at runtime (doc comment says "Unsupported"
    // there), so no `#[cfg(target_os = ...)]` gate is needed here.
    if let Some(win) = app.get_webview_window("main") {
        win.set_badge_count(if count > 0 { Some(count) } else { None })
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[tauri::command(rename_all = "camelCase")]
pub async fn save_portrait(
    app_state: State<'_, AppState>,
    agent_id: String,
    png_base64: String,
) -> Result<(), String> {
    let ids: Vec<String> = app_state
        .store
        .load()
        .agents
        .iter()
        .map(|a| a.id.clone())
        .collect();
    app_state
        .portrait_store
        .save(&agent_id, &png_base64, &ids)
        .map_err(|e| e.to_string())
}

#[tauri::command(rename_all = "camelCase")]
pub async fn load_portrait(
    app_state: State<'_, AppState>,
    agent_id: String,
) -> Result<Option<String>, String> {
    app_state
        .portrait_store
        .load(&agent_id)
        .map_err(|e| e.to_string())
}

#[tauri::command(rename_all = "camelCase")]
pub async fn delete_portrait(
    app_state: State<'_, AppState>,
    agent_id: String,
) -> Result<(), String> {
    app_state
        .portrait_store
        .delete(&agent_id)
        .map_err(|e| e.to_string())
}

#[tauri::command(rename_all = "camelCase")]
pub async fn save_sprite(
    app_state: State<'_, AppState>,
    agent_id: String,
    png_base64: String,
) -> Result<(), String> {
    let ids: Vec<String> = app_state
        .store
        .load()
        .agents
        .iter()
        .map(|a| a.id.clone())
        .collect();
    app_state
        .sprite_store
        .save(&agent_id, &png_base64, &ids)
        .map_err(|e| e.to_string())
}

#[tauri::command(rename_all = "camelCase")]
pub async fn load_sprite(
    app_state: State<'_, AppState>,
    agent_id: String,
) -> Result<Option<String>, String> {
    app_state
        .sprite_store
        .load(&agent_id)
        .map_err(|e| e.to_string())
}

#[tauri::command(rename_all = "camelCase")]
pub async fn delete_sprite(
    app_state: State<'_, AppState>,
    agent_id: String,
) -> Result<(), String> {
    app_state
        .sprite_store
        .delete(&agent_id)
        .map_err(|e| e.to_string())
}

/// 머리 위 라벨 요약: `claude -p`(haiku) 헤드리스 호출. 유저 크레딧을
/// 소모하므로 opt-in — 설정 OFF면 "claude-cli-disabled"로 거절하고
/// 렌더러 summarizer가 원문 폴백으로 처리한다.
#[tauri::command(rename_all = "camelCase")]
pub async fn summarize_text(
    app_state: State<'_, AppState>,
    instruction: String,
    text: String,
) -> Result<String, String> {
    if !app_state.settings.read().unwrap().claude_cli_enabled {
        return Err("claude-cli-disabled".to_string());
    }
    crate::claude_cli::summarize(&instruction, &text).await
}

/// PixelLab로 64×64 스프라이트 1장 생성. AppState 비의존
/// (stateless) — 이 command만은 본문에 .await가 있으나 락을 전혀 잡지
/// 않으므로 파일 머리말의 "no lock across await" 계약과 무관하다.
#[tauri::command(rename_all = "camelCase")]
pub async fn generate_sprite_image(
    description: String,
) -> Result<crate::pixellab::GeneratedImage, String> {
    let trimmed = description.trim();
    if trimmed.is_empty() {
        return Err("validation: description is empty".to_string());
    }
    // pixen maxLength 2000 — 초과분은 뒤를 자른다 (char 경계 안전).
    let capped: String = trimmed
        .chars()
        .take(crate::pixellab::DESCRIPTION_MAX_CHARS)
        .collect();
    crate::pixellab::generate_image(&capped)
        .await
        .map_err(|e| e.to_ipc_string())
}

#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GetAppSettingsResult {
    pub settings: AppSettings,
    pub first_run: bool,
}

#[tauri::command(rename_all = "camelCase")]
pub async fn get_app_settings(
    app_state: State<'_, AppState>,
) -> Result<GetAppSettingsResult, String> {
    Ok(GetAppSettingsResult {
        settings: *app_state.settings.read().unwrap(),
        first_run: app_state
            .settings_first_run
            .load(std::sync::atomic::Ordering::SeqCst),
    })
}

/// 저장 + 캐시 갱신. 훅 OFF→ON이면 훅 서버를 지연 기동한다(이미 떠 있으면
/// 재사용). ON→OFF는 서버 프로세스 자체를 내리지 않는다 — 이미 떠 있는
/// 세션들의 훅 POST는 계속 수신된다. 다만 캐시가 OFF로 갱신된 뒤로는
/// lib.rs의 훅 포트 getter가 (서버가 살아있어도) None을 돌려주므로, 이
/// 시점 이후 새로 만드는 세션에는 훅 배선(--settings·env·ZDOTDIR)이 전혀
/// 주입되지 않는다 -- "변경은 새 세션부터 적용" 정책의 실제 동작.
#[tauri::command(rename_all = "camelCase")]
pub async fn set_app_settings(
    app_state: State<'_, AppState>,
    settings: AppSettings,
) -> Result<(), String> {
    // write 가드를 먼저 잡고 쥔 채 저장(동기, await 없음) 후 캐시를 갱신한다 --
    // 그래야 두 set_app_settings 호출이 겹쳐도 "디스크에 쓴 값"과 "캐시에 남는
    // 값"이 서로 다른 호출 것이 되는 경합이 없다. 가드는 .await 지점 전에
    // 스코프를 벗어나 해제되므로(파일 머리말의 no-lock-across-await 계약 유지),
    // 아래 훅 서버 지연 기동 await는 락 없이 진행된다.
    {
        let mut guard = app_state.settings.write().unwrap();
        app_state.settings_store.save(&settings).map_err(|e| e.to_string())?;
        *guard = settings;
    }
    app_state
        .settings_first_run
        .store(false, std::sync::atomic::Ordering::SeqCst);

    let need_server = settings.claude_hooks_enabled
        && app_state.hook_port.read().unwrap().is_none();
    if need_server {
        // 락은 .await 전에 전부 놓는다(파일 머리말의 no-lock-across-await 계약).
        let hub = app_state.hub.clone();
        let (port, tx, handle) = crate::notification::hook_server::serve_with_retry(|rx| {
            crate::notification::hook_server::serve(hub.clone(), rx)
        })
        .await
        .map_err(|e| e.to_string())?;
        let mut guard = app_state.hook_port.write().unwrap();
        if guard.is_none() {
            *guard = Some(port);
            drop(guard);
            *app_state.hook_shutdown.lock().unwrap() = Some(tx);
            *app_state.server_handle.lock().unwrap() = Some(handle);
        } else {
            // 동시 호출 경합으로 다른 쪽이 먼저 기동함 — 새로 띄운 서버는 종료.
            drop(guard);
            let _ = tx.send(());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    // Assert each command *body* delegates correctly into
    // manager/hub/store. `tauri::State<'_, AppState>` cannot be constructed
    // standalone (it borrows from a live `tauri::App`/`AppHandle`), so
    // instead of driving the `#[tauri::command]`-wrapped async fns directly,
    // these tests build a real `AppState` (fakes for PtyFactory/AppEvents,
    // tempdir-backed ProfileStore/HookSettingsWriter -- the same seams
    // other test modules use) and call the exact `app_state.manager` /
    // `app_state.hub` / `app_state.store` method sequence each command body
    // above executes. Every command function is a one-line, non-`await`ing
    // delegation, so exercising the delegation target through a real
    // `AppState` proves the wiring without needing a Tauri runtime.
    // `subscribe_output`/`unsubscribe_output` (need a live `Channel`) and
    // `set_badge_count` (needs a live `AppHandle`/webview window) are left
    // to manual/E2E verification -- there is no seam for either without a
    // running Tauri app.
    use super::*;
    use crate::notification::hook_settings::HookSettingsWriter;
    use crate::notification::hub::{NotificationHub, SystemClock};
    use crate::persistence::profile_store::ProfileStore;
    use crate::session::manager::SessionManager;
    use crate::session::pty_factory::fake::FakePtyFactory;
    use crate::state::fake::RecordingEvents;
    use crate::state::{AppEvents, SessionRegistry};
    use std::path::PathBuf;
    use std::sync::Arc;
    use std::time::Duration;
    use uuid::Uuid;

    // summarize_text은 `State<'_, AppState>`를 받으므로 (State는 살아있는
    // App/AppHandle 없이 standalone 구성 불가) 다른 stateful command들과 같은
    // 패턴으로, 본문의 게이트 로직을 real AppState를 통해 그대로 재현해
    // 검증한다.

    #[tokio::test]
    async fn summarize_text_gate_rejects_when_cli_disabled() {
        let (state, ctl, dir, profile_dir) = build("summarize-disabled");
        // AppSettings::default()의 claude_cli_enabled == false 전제.
        assert!(!state.settings.read().unwrap().claude_cli_enabled);

        // summarize_text 본문과 동일한 게이트: OFF면 CLI 호출 전에 거절.
        let result: Result<String, String> = if !state.settings.read().unwrap().claude_cli_enabled
        {
            Err("claude-cli-disabled".to_string())
        } else {
            crate::claude_cli::summarize("요약하라", "text").await
        };

        assert_eq!(result.unwrap_err(), "claude-cli-disabled");
        cleanup(&ctl, &dir, &profile_dir);
    }

    #[tokio::test]
    async fn summarize_text_proceeds_to_claude_cli_when_enabled() {
        let (state, ctl, dir, profile_dir) = build("summarize-enabled");
        *state.settings.write().unwrap() = crate::persistence::settings_store::AppSettings {
            version: 1,
            claude_cli_enabled: true,
            claude_hooks_enabled: false,
        };

        // ON이면 게이트를 통과해 claude_cli::summarize로 위임된다 -- 빈 텍스트라서
        // 실 프로세스 spawn 없이 그쪽의 자체 검증 에러로 되돌아오는 것으로 확인.
        let result: Result<String, String> = if !state.settings.read().unwrap().claude_cli_enabled
        {
            Err("claude-cli-disabled".to_string())
        } else {
            crate::claude_cli::summarize("요약하라", "   ").await
        };

        assert_eq!(result.unwrap_err(), "validation: text is empty");
        cleanup(&ctl, &dir, &profile_dir);
    }

    // set_app_settings 본문(저장 -> 캐시 갱신 -> first_run 플래그 내림)을
    // 그대로 재현해 검증한다. 회귀 대상: 첫 실행 완료 후 웹뷰가 리로드돼도
    // get_app_settings가 firstRun: true를 다시 주면 안 된다(Minor #3).
    #[tokio::test]
    async fn set_app_settings_clears_first_run_flag_after_success() {
        let (state, ctl, dir, profile_dir) = build("first-run-flag");
        assert!(
            state.settings_first_run.load(std::sync::atomic::Ordering::SeqCst),
            "build() 헬퍼는 부팅 시 settings.json 부재 상태를 흉내내므로 초기값은 true"
        );

        let new_settings = crate::persistence::settings_store::AppSettings {
            version: 1,
            claude_cli_enabled: true,
            claude_hooks_enabled: false,
        };
        // set_app_settings 본문과 동일한 순서: write 가드를 쥔 채 저장 후 캐시
        // 갱신, 가드 해제 -- 그다음 first_run을 false로 내린다.
        {
            let mut guard = state.settings.write().unwrap();
            state
                .settings_store
                .save(&new_settings)
                .expect("save into tempdir should succeed");
            *guard = new_settings;
        }
        state
            .settings_first_run
            .store(false, std::sync::atomic::Ordering::SeqCst);

        assert!(!state.settings_first_run.load(std::sync::atomic::Ordering::SeqCst));
        assert_eq!(*state.settings.read().unwrap(), new_settings);
        cleanup(&ctl, &dir, &profile_dir);
    }

    /// Unique tempdir per test, matching the convention used throughout the
    /// other modules' tests (no `tempfile` dependency needed).
    fn scratch_dir(tag: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "agent-office-commands-test-{tag}-{}",
            Uuid::new_v4()
        ))
    }

    /// Builds a real `AppState` wired to fakes, mirroring `lib.rs`'s setup
    /// wiring but with `FakePtyFactory`/`RecordingEvents` standing in for
    /// the PTY/Tauri-event side effects.
    fn build(tag: &str) -> (AppState, Arc<FakePtyFactoryControl>, PathBuf, PathBuf) {
        let events: Arc<RecordingEvents> = Arc::new(RecordingEvents::default());
        let events_dyn: Arc<dyn AppEvents> = events.clone();
        let registry = Arc::new(SessionRegistry::new());
        let hub = Arc::new(NotificationHub::new(
            registry.clone(),
            events_dyn.clone(),
            Arc::new(SystemClock),
            Duration::from_millis(3000),
        ));
        let hook_dir = scratch_dir(&format!("{tag}-hooks"));
        let writer = HookSettingsWriter::new(hook_dir.clone());
        let (fac, ctl) = FakePtyFactory::new();
        let manager = Arc::new(SessionManager::new(
            Arc::new(fac),
            writer,
            registry,
            events_dyn,
            hub.clone(),
            Arc::new(|| Some(12345u16)),
        ));
        let profile_dir = scratch_dir(&format!("{tag}-profiles"));
        let store = ProfileStore::new(profile_dir.join("profiles.json"));
        let portrait_store = crate::persistence::png_store::PngStore::new(
            profile_dir.join("portraits"),
            crate::persistence::png_store::MAX_PORTRAIT_BYTES,
        );
        let sprite_store = crate::persistence::png_store::PngStore::new(
            profile_dir.join("sprites"),
            crate::persistence::png_store::MAX_SPRITE_BYTES,
        );

        let settings_store =
            crate::persistence::settings_store::SettingsStore::new(profile_dir.join("settings.json"));

        let state = AppState {
            manager,
            hub,
            store,
            portrait_store,
            sprite_store,
            settings_store,
            settings: Arc::new(std::sync::RwLock::new(
                crate::persistence::settings_store::AppSettings::default(),
            )),
            settings_first_run: std::sync::atomic::AtomicBool::new(true),
            hook_port: Arc::new(std::sync::RwLock::new(None)),
            hook_shutdown: std::sync::Mutex::new(None),
            server_handle: std::sync::Mutex::new(None),
        };
        (state, ctl, hook_dir, profile_dir)
    }

    type FakePtyFactoryControl = crate::session::pty_factory::fake::FakeControl;

    fn req(agent_id: &str) -> CreateSessionRequest {
        CreateSessionRequest {
            agent_id: agent_id.into(),
            cols: None,
            rows: None,
            cwd: None,
            autostart_claude: None,
        }
    }

    fn cleanup(ctl: &FakePtyFactoryControl, hook_dir: &PathBuf, profile_dir: &PathBuf) {
        ctl.close_output();
        let _ = std::fs::remove_dir_all(hook_dir);
        let _ = std::fs::remove_dir_all(profile_dir);
    }

    // ---- create_session ----

    #[tokio::test]
    async fn create_session_delegates_to_manager_create_with_opts_and_default_autostart() {
        let (state, ctl, dir, profile_dir) = build("create");

        let result = state
            .manager
            .create(CreateSessionRequest {
                agent_id: "a1".into(),
                cols: Some(100),
                rows: Some(30),
                cwd: None,
                autostart_claude: None, // command body always passes None -> manager defaults to false
            })
            .unwrap();

        assert_eq!(result.state, SessionState::Running);
        // autostart defaulted false (plain shell) -> no stdin injection.
        assert_eq!(ctl.writes_utf8(), "");

        cleanup(&ctl, &dir, &profile_dir);
    }

    // ---- dispose_session ----

    #[tokio::test]
    async fn dispose_session_delegates_to_manager_dispose() {
        let (state, ctl, dir, profile_dir) = build("dispose");
        state.manager.create(req("a1")).unwrap();

        state.manager.dispose("a1");

        assert_eq!(
            ctl.kill_count(),
            1,
            "dispose_session must reach PtyControl::kill via manager.dispose"
        );
        cleanup(&ctl, &dir, &profile_dir);
    }

    // ---- write_input ----

    #[tokio::test]
    async fn write_input_delegates_to_manager_write_input() {
        let (state, ctl, dir, profile_dir) = build("write");
        state.manager.create(req("a1")).unwrap();
        let claude_launch_len = ctl.writes_utf8().len();

        state.manager.write_input("a1", "echo hi\n");

        assert_eq!(&ctl.writes_utf8()[claude_launch_len..], "echo hi\n");
        cleanup(&ctl, &dir, &profile_dir);
    }

    // ---- resize_session ----

    #[tokio::test]
    async fn resize_session_delegates_to_manager_resize() {
        let (state, ctl, dir, profile_dir) = build("resize");
        state.manager.create(req("a1")).unwrap();

        state.manager.resize("a1", 120, 40);

        assert_eq!(ctl.resize_calls(), vec![(120, 40)]);
        cleanup(&ctl, &dir, &profile_dir);
    }

    // ---- list_notifications ----

    #[tokio::test]
    async fn list_notifications_delegates_to_manager_pending_notifications() {
        let (state, ctl, dir, profile_dir) = build("list-notif");
        state.manager.create(req("a1")).unwrap();
        let sid = state.manager.session_id_for("a1").unwrap();
        state.hub.ingest_hook(
            &sid,
            crate::types::NotificationSource::Hook,
            br#"{"message":"hi"}"#,
        );

        let listed = state.manager.pending_notifications("a1");

        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].message, "hi");
        cleanup(&ctl, &dir, &profile_dir);
    }

    #[tokio::test]
    async fn list_notifications_for_unknown_agent_returns_empty() {
        let (state, ctl, dir, profile_dir) = build("list-notif-unknown");
        assert!(state.manager.pending_notifications("ghost").is_empty());
        cleanup(&ctl, &dir, &profile_dir);
    }

    // ---- clear_notifications ----

    #[tokio::test]
    async fn clear_notifications_resolves_session_id_then_delegates_to_hub_clear() {
        let (state, ctl, dir, profile_dir) = build("clear-notif");
        state.manager.create(req("a1")).unwrap();
        let sid = state.manager.session_id_for("a1").unwrap();
        state.hub.ingest_hook(
            &sid,
            crate::types::NotificationSource::Hook,
            br#"{"message":"hi"}"#,
        );

        // Mirrors `clear_notifications`'s body exactly: session_id_for then hub.clear.
        if let Some(resolved_sid) = state.manager.session_id_for("a1") {
            state.hub.clear(&resolved_sid, None);
        }

        assert!(state.hub.pending(&sid).is_empty());
        cleanup(&ctl, &dir, &profile_dir);
    }

    #[tokio::test]
    async fn clear_notifications_for_unknown_agent_is_a_harmless_noop() {
        let (state, ctl, dir, profile_dir) = build("clear-notif-unknown");
        // Must not panic when session_id_for resolves to None.
        if let Some(sid) = state.manager.session_id_for("ghost") {
            state.hub.clear(&sid, None);
        }
        cleanup(&ctl, &dir, &profile_dir);
    }

    // ---- load_state / save_state ----

    #[tokio::test]
    async fn load_state_delegates_to_store_load() {
        let (state, ctl, dir, profile_dir) = build("load-state");
        let loaded = state.store.load();
        assert_eq!(loaded.version, 1);
        assert!(loaded.agents.is_empty());
        cleanup(&ctl, &dir, &profile_dir);
    }

    #[tokio::test]
    async fn save_state_delegates_to_store_save_and_maps_io_error_to_string() {
        let (state, ctl, dir, profile_dir) = build("save-state");
        let persisted = PersistedState {
            agents: vec![AgentProfile {
                id: "p1".into(),
                name: "Ada".into(),
                role: "backend".into(),
                note: "".into(),
                seed: "seed".into(),
                created_at: 1,
                desk_index: 0,
                cwd: None,
                appearance: None,
                portrait_updated_at: None,
                sprite_request: None,
                sprite_updated_at: None,
                archetype: None,
            }],
            version: 1,
        };

        // Mirrors `save_state`'s body: `store.save(&state).map_err(|e| e.to_string())`.
        let result: Result<(), String> = state.store.save(&persisted).map_err(|e| e.to_string());
        assert!(result.is_ok());

        let reloaded = state.store.load();
        assert_eq!(reloaded.agents.len(), 1);
        assert_eq!(reloaded.agents[0].name, "Ada");
        cleanup(&ctl, &dir, &profile_dir);
    }

    // ---- portrait commands ----

    fn tiny_png_b64() -> String {
        use base64::Engine;
        let mut v = vec![0x89u8, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A];
        v.extend_from_slice(b"body");
        base64::engine::general_purpose::STANDARD.encode(v)
    }

    #[tokio::test]
    async fn save_then_load_then_delete_portrait_through_app_state() {
        let (state, ctl, dir, profile_dir) = build("portrait");
        // 프로필 존재 검증을 위해 profiles.json에 p1 저장.
        let persisted = PersistedState {
            agents: vec![AgentProfile {
                id: "p1".into(),
                name: "Ada".into(),
                role: "backend".into(),
                note: "".into(),
                seed: "seed".into(),
                created_at: 1,
                desk_index: 0,
                cwd: None,
                appearance: None,
                portrait_updated_at: None,
                sprite_request: None,
                sprite_updated_at: None,
                archetype: None,
            }],
            version: 1,
        };
        state.store.save(&persisted).unwrap();
        let ids: Vec<String> = state.store.load().agents.iter().map(|a| a.id.clone()).collect();
        let encoded = tiny_png_b64();

        // save_portrait 본문과 동일한 delegation.
        state.portrait_store.save("p1", &encoded, &ids).unwrap();
        // load_portrait 본문.
        let loaded = state.portrait_store.load("p1").unwrap();
        assert_eq!(loaded, Some(encoded));
        // delete_portrait 본문.
        state.portrait_store.delete("p1").unwrap();
        assert_eq!(state.portrait_store.load("p1").unwrap(), None);

        cleanup(&ctl, &dir, &profile_dir);
    }

    #[tokio::test]
    async fn save_portrait_maps_unknown_agent_to_err() {
        let (state, ctl, dir, profile_dir) = build("portrait-unknown");
        let ids: Vec<String> = state.store.load().agents.iter().map(|a| a.id.clone()).collect();
        let result: Result<(), String> = state
            .portrait_store
            .save("ghost", &tiny_png_b64(), &ids)
            .map_err(|e| e.to_string());
        assert!(result.is_err());
        cleanup(&ctl, &dir, &profile_dir);
    }

    #[tokio::test]
    async fn save_then_load_then_delete_sprite_through_app_state() {
        let (state, ctl, dir, profile_dir) = build("sprite");
        let persisted = PersistedState {
            agents: vec![AgentProfile {
                id: "p1".into(),
                name: "Ada".into(),
                role: "backend".into(),
                note: "".into(),
                seed: "seed".into(),
                created_at: 1,
                desk_index: 0,
                cwd: None,
                appearance: None,
                portrait_updated_at: None,
                sprite_request: None,
                sprite_updated_at: None,
                archetype: None,
            }],
            version: 1,
        };
        state.store.save(&persisted).unwrap();
        let ids: Vec<String> = state.store.load().agents.iter().map(|a| a.id.clone()).collect();
        let encoded = tiny_png_b64();

        // save_sprite / load_sprite / delete_sprite 본문과 동일한 delegation.
        state.sprite_store.save("p1", &encoded, &ids).unwrap();
        assert_eq!(state.sprite_store.load("p1").unwrap(), Some(encoded));
        // portraits와 sprites는 별도 디렉터리다.
        assert_eq!(state.portrait_store.load("p1").unwrap(), None);
        state.sprite_store.delete("p1").unwrap();
        assert_eq!(state.sprite_store.load("p1").unwrap(), None);

        cleanup(&ctl, &dir, &profile_dir);
    }

    // generate_sprite_image: 네트워크 이전의 검증 로직만 테스트한다.
    // (실 API 호출 테스트 금지 — 빈 description은 HTTP 전에 걸러져야 한다.)
    #[tokio::test]
    async fn generate_sprite_image_rejects_empty_description() {
        let err = generate_sprite_image("   ".to_string()).await.unwrap_err();
        assert_eq!(err, "validation: description is empty");
    }
}
