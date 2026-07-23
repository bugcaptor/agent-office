// src-tauri/src/ipc/commands/misc.rs
//
// Grab-bag of commands that don't fit another domain: dock/taskbar badge,
// opening the agent's working folder in an external app, exporting terminal
// output, and the native folder-picker dialog.

use tauri::{AppHandle, Emitter, Manager, State};

use crate::state::AppState;

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

/// 마스코트 창(이슈 #72)을 보이거나 숨긴다. 창은 tauri.conf.json에서
/// `visible:false`로 항상 만들어지고, 표시 여부는 main 창의 mascotBridge가
/// 이 커맨드로만 제어한다(런타임 create/destroy 없음 — 라이프사이클이 단순하고
/// capability의 window 매칭이 정적으로 유지된다). 창이 없으면 조용히 no-op.
///
/// `focus:false` 설정 덕에 show()가 포커스를 훔치지 않는다 — 사용자가 다른 앱에
/// 타이핑하는 중에 마스코트가 떠도 입력이 끊기지 않는다.
#[tauri::command(rename_all = "camelCase")]
pub async fn set_mascot_visible(app: AppHandle, visible: bool) -> Result<(), String> {
    if let Some(win) = app.get_webview_window("mascot") {
        if visible {
            win.show().map_err(|e| e.to_string())?;
        } else {
            win.hide().map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

/// 마스코트 클릭(이슈 #72): main 창을 앞으로 끌어올린 뒤 해당 에이전트의
/// 터미널을 열라고 main에 알린다. 포커스/표시는 Rust가 수행하므로 마스코트
/// 창에는 창 조작 권한을 주지 않아도 된다(권한 표면 최소화).
///
/// 최소화 상태에서도 복구돼야 하므로 show + unminimize + set_focus 3연타.
/// 이벤트는 `emit_to("main", ...)`으로 보내 마스코트 자신이 되받지 않게 한다.
#[tauri::command(rename_all = "camelCase")]
pub async fn mascot_activate(app: AppHandle, agent_id: String) -> Result<(), String> {
    if let Some(main) = app.get_webview_window("main") {
        let _ = main.show();
        let _ = main.unminimize();
        let _ = main.set_focus();
    }
    app.emit_to(
        "main",
        "mascot-open-terminal",
        serde_json::json!({ "agentId": agent_id }),
    )
    .map_err(|e| e.to_string())
}

/// 에이전트 작업 폴더를 Visual Studio Code로 연다. `path`는 렌더러가
/// 프로필의 `cwd`를 그대로 전달한다(미설정 시 메뉴가 비활성화되므로 폴백
/// 없음). 시작 폴더 UI가 `~/dev/foo`류 입력을 허용하므로 세션 생성과
/// 동일한 틸드 확장을 거친다. 구현/OS별 실행 전략은 `crate::vscode` 참조.
#[tauri::command(rename_all = "camelCase")]
pub async fn open_in_vscode(path: String) -> Result<(), String> {
    crate::vscode::open_dir_in_vscode(&crate::session::manager::expand_tilde(path))
}

/// 에이전트 작업 폴더를 외부 터미널 앱으로 연다. 전달/확장 규칙은
/// `open_in_vscode`와 동일. 어떤 앱을 쓸지는 앱 설정 `externalTerminal`
/// (macOS 전용 — Terminal.app/iTerm)을 따른다. 구현/OS별 실행 전략은
/// `crate::terminal` 참조.
#[tauri::command(rename_all = "camelCase")]
pub async fn open_in_terminal(
    app_state: State<'_, AppState>,
    path: String,
) -> Result<(), String> {
    let prefer_iterm = matches!(
        app_state.settings.read().unwrap().external_terminal,
        crate::persistence::settings_store::ExternalTerminal::Iterm
    );
    crate::terminal::open_dir_in_terminal(
        &crate::session::manager::expand_tilde(path),
        prefer_iterm,
    )
}

/// 이슈 #42: 셸 출력(터미널 버퍼 plain text)을 임시 .txt 파일로 쓰고 사용자가
/// 설정한 외부 에디터로 연다. `content`는 렌더러(TerminalRegistry.getPlainText)가
/// 추출한 현재 화면(스크롤백 포함), `agent_name`은 파일명에 쓸 표시 이름이다.
/// 어떤 에디터를 쓸지는 앱 설정 `externalEditor`(system/vscode)를 따른다.
/// 성공 시 쓴 파일의 절대 경로 문자열을 돌려준다. 구현은 `crate::shell_export`.
#[tauri::command(rename_all = "camelCase")]
pub async fn export_terminal_output(
    app_state: State<'_, AppState>,
    agent_name: String,
    content: String,
) -> Result<String, String> {
    // 파일명 충돌 없이 매번 새 파일 -- 초 단위 timestamp를 파일명에 넣는다.
    let timestamp = chrono::Local::now().format("%Y%m%d-%H%M%S").to_string();
    let file = crate::shell_export::write_export_file(&agent_name, &content, &timestamp)?;

    // 설정 read 가드는 에디터 프로세스(블로킹 status 대기)를 실행하기 전에
    // 드롭한다 -- 실행이 길어져도 설정 락을 쥐고 있지 않도록.
    let use_vscode = {
        let guard = app_state.settings.read().unwrap();
        matches!(
            guard.external_editor,
            crate::persistence::settings_store::ExternalEditor::Vscode
        )
    };
    crate::shell_export::open_file_in_editor(&file, use_vscode)?;
    Ok(file.to_string_lossy().into_owned())
}

/// 네이티브 폴더 선택 다이얼로그를 띄운다. 사용자가 고른 절대 경로,
/// 취소 시 None. `initial_dir`이 (틸드 확장 후) 실존 디렉터리면 거기서
/// 시작한다 — 아니면 OS 기본 위치. 다이얼로그 표시의 메인 스레드 디스패치는
/// tauri-plugin-dialog가 처리하므로 async 커맨드 스레드에서 안전하다.
#[tauri::command(rename_all = "camelCase")]
pub async fn pick_directory(
    app: tauri::AppHandle,
    initial_dir: Option<String>,
) -> Result<Option<String>, String> {
    use tauri_plugin_dialog::DialogExt;

    let mut builder = app.dialog().file();
    if let Some(dir) = initial_dir {
        let expanded = crate::session::manager::expand_tilde(dir);
        if std::path::Path::new(&expanded).is_dir() {
            builder = builder.set_directory(expanded);
        }
    }

    // 콜백 → oneshot 브리지: blocking_pick_folder는 async 런타임 스레드를
    // 다이얼로그가 닫힐 때까지 점유하므로 쓰지 않는다.
    let (tx, rx) = tokio::sync::oneshot::channel();
    builder.pick_folder(move |folder| {
        let _ = tx.send(folder);
    });
    let picked = rx
        .await
        .map_err(|_| "폴더 선택 다이얼로그가 응답 없이 종료되었습니다".to_string())?;
    match picked {
        None => Ok(None),
        Some(fp) => Ok(Some(
            fp.into_path()
                .map_err(|e| e.to_string())?
                .to_string_lossy()
                .into_owned(),
        )),
    }
}
