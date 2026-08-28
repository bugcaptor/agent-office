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
/// 포커스를 절대 훔치지 않는다: `focus:false`는 창 생성 시점만 막고, show()는
/// 내부적으로 makeKeyAndOrderFront(tao window.rs `set_visible`)라 그것만으로는
/// 부족하다. 그래서 `focusable:false`를 함께 켠다 — tao가 `canBecomeKeyWindow`/
/// `canBecomeMainWindow`를 NO로 덮으므로(TaoWindow 클래스) show()도 클릭도
/// 마스코트를 key 창으로 만들지 못한다. 사용자가 터미널에 타이핑하는 중에
/// 마스코트가 뜨거나 마스코트를 드래그해도 입력 포커스가 끊기지 않는다.
/// 드래그(`startDragging` → tao `drag_window`)는 performWindowDragWithEvent라
/// key 창을 요구하지 않고, 클릭 히트는 `acceptFirstMouse:true`가 보장한다.
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

/// 마스코트 신호등(docs/mascot-lights-design.md §5.3) — 창 크기(칸 수·방향·
/// 스프라이트 유무에 따라 계산된 값)와 하단중앙 앵커로 환산된 위치를 한 번에
/// 적용한다. `width`/`height`/`x`/`y`는 전부 **물리 px**(렌더러가 dpr로 환산해
/// 넘긴다). 창이 없으면 조용히 no-op.
///
/// `set_resizable(true)` → `set_size` → `set_position` → `set_resizable(false)`
/// 순으로 감싸는 이유: tauri.conf.json에서 마스코트 창은 `resizable:false`로
/// 만들어지는데(사용자가 테두리를 끌어 늘리지 못하게), Windows에서는
/// non-resizable 창에 대해 **프로그램이 거는** `set_size` 호출도 무시되는
/// 사례가 보고돼 있다(§10). 매 호출마다 잠깐 resizable을 켰다 다시 끄면
/// 사용자 조작 여지는 그대로 막으면서 프로그램 리사이즈만 허용할 수 있다.
#[tauri::command(rename_all = "camelCase")]
pub async fn set_mascot_layout(
    app: AppHandle,
    width: u32,
    height: u32,
    x: i32,
    y: i32,
) -> Result<(), String> {
    if let Some(win) = app.get_webview_window("mascot") {
        win.set_resizable(true).map_err(|e| e.to_string())?;
        // set_size/set_position이 실패해도 set_resizable(false)는 반드시
        // 실행한다 — `?`로 조기 반환하면 그 뒤로 사용자가 창 테두리를 끌어
        // 늘릴 수 있게 된 채 다음 성공 호출 전까지 남는다(리뷰 C3). 결과를
        // 변수로 받아 두고 resizable 복원 다음에 반환한다.
        let result = win
            .set_size(tauri::PhysicalSize::new(width, height))
            .and_then(|_| win.set_position(tauri::PhysicalPosition::new(x, y)));
        win.set_resizable(false).map_err(|e| e.to_string())?;
        result.map_err(|e| e.to_string())?;
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

/// 작업 폴더를 OS 파일 탐색기(macOS Finder / Windows 탐색기 / Linux 기본 파일
/// 관리자)로 연다. 전달/확장 규칙은 `open_in_vscode`와 동일. 구현/OS별 실행
/// 전략은 `crate::file_manager` 참조.
#[tauri::command(rename_all = "camelCase")]
pub async fn open_in_file_manager(path: String) -> Result<(), String> {
    crate::file_manager::open_dir_in_file_manager(&crate::session::manager::expand_tilde(path))
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

/// 캐릭터 번들(이슈 #77) 내보내기: 네이티브 저장 다이얼로그를 띄워 사용자가 고른
/// 경로에 UTF-8 텍스트(자기완결형 JSON)를 쓴다. 저장한 절대 경로, 취소 시 None.
/// `default_name`은 다이얼로그 초기 파일명. 콜백→oneshot 브리지는 `pick_directory`와
/// 동일한 이유(블로킹 API가 런타임 스레드를 점유)로 쓴다.
#[tauri::command(rename_all = "camelCase")]
pub async fn export_character_file(
    app: tauri::AppHandle,
    default_name: String,
    content: String,
) -> Result<Option<String>, String> {
    use tauri_plugin_dialog::DialogExt;

    let (tx, rx) = tokio::sync::oneshot::channel();
    app.dialog()
        .file()
        .set_file_name(default_name)
        .add_filter("Agent Office 캐릭터", &["json"])
        .save_file(move |path| {
            let _ = tx.send(path);
        });
    let picked = rx
        .await
        .map_err(|_| "저장 다이얼로그가 응답 없이 종료되었습니다".to_string())?;
    match picked {
        None => Ok(None),
        Some(fp) => {
            let path = fp.into_path().map_err(|e| e.to_string())?;
            std::fs::write(&path, content.as_bytes()).map_err(|e| e.to_string())?;
            Ok(Some(path.to_string_lossy().into_owned()))
        }
    }
}

/// 캐릭터 번들(이슈 #77) 가져오기: 네이티브 열기 다이얼로그로 파일 하나를 고르게
/// 하고 UTF-8 텍스트로 읽어 그대로 돌려준다(파싱/검증은 프런트가 수행). 취소 시 None.
#[tauri::command(rename_all = "camelCase")]
pub async fn import_character_file(app: tauri::AppHandle) -> Result<Option<String>, String> {
    use tauri_plugin_dialog::DialogExt;

    let (tx, rx) = tokio::sync::oneshot::channel();
    app.dialog()
        .file()
        .add_filter("Agent Office 캐릭터", &["json"])
        .pick_file(move |path| {
            let _ = tx.send(path);
        });
    let picked = rx
        .await
        .map_err(|_| "열기 다이얼로그가 응답 없이 종료되었습니다".to_string())?;
    match picked {
        None => Ok(None),
        Some(fp) => {
            let path = fp.into_path().map_err(|e| e.to_string())?;
            let text = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
            Ok(Some(text))
        }
    }
}

/// 저장 다이얼로그에서 고른 경로의 확장자로 쓸 본문을 고른다(이슈 #65).
/// `.json`(대소문자 무관)만 JSON, 나머지(확장자 없음 포함)는 Markdown —
/// 사용자가 확장자를 지우거나 임의로 바꿔도 사람이 읽는 쪽으로 떨어진다.
/// 순수 함수라 다이얼로그 없이 단위 테스트한다.
pub fn diary_content_for<'a>(path: &std::path::Path, markdown: &'a str, json: &'a str) -> &'a str {
    let is_json = path
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("json"));
    if is_json {
        json
    } else {
        markdown
    }
}

/// 캐릭터 일기(이슈 #65) 내보내기: 네이티브 저장 다이얼로그로 위치를 고르게 하고,
/// 고른 파일의 확장자에 따라 Markdown/JSON 본문 중 하나를 쓴다(둘 다 미리 만들어
/// 넘겨받는다 — 다이얼로그가 닫힌 뒤에는 렌더러에 되물을 수 없으므로). 저장한
/// 절대 경로, 취소 시 None. 콜백→oneshot 브리지는 `export_character_file`과 동일.
#[tauri::command(rename_all = "camelCase")]
pub async fn export_diary_file(
    app: tauri::AppHandle,
    default_name: String,
    markdown: String,
    json: String,
) -> Result<Option<String>, String> {
    use tauri_plugin_dialog::DialogExt;

    let (tx, rx) = tokio::sync::oneshot::channel();
    app.dialog()
        .file()
        .set_file_name(default_name)
        .add_filter("Markdown", &["md"])
        .add_filter("JSON", &["json"])
        .save_file(move |path| {
            let _ = tx.send(path);
        });
    let picked = rx
        .await
        .map_err(|_| "저장 다이얼로그가 응답 없이 종료되었습니다".to_string())?;
    match picked {
        None => Ok(None),
        Some(fp) => {
            let path = fp.into_path().map_err(|e| e.to_string())?;
            let content = diary_content_for(&path, &markdown, &json);
            std::fs::write(&path, content.as_bytes()).map_err(|e| e.to_string())?;
            Ok(Some(path.to_string_lossy().into_owned()))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn json_extension_picks_json_others_pick_markdown() {
        assert_eq!(diary_content_for(Path::new("/t/a.json"), "MD", "JS"), "JS");
        assert_eq!(diary_content_for(Path::new("/t/a.JSON"), "MD", "JS"), "JS");
        assert_eq!(diary_content_for(Path::new("/t/a.md"), "MD", "JS"), "MD");
        assert_eq!(diary_content_for(Path::new("/t/a.txt"), "MD", "JS"), "MD");
        // 확장자를 지운 경우도 사람이 읽는 Markdown으로 떨어진다.
        assert_eq!(diary_content_for(Path::new("/t/일기"), "MD", "JS"), "MD");
    }

    #[test]
    fn export_writes_picked_content_and_roundtrips() {
        // 다이얼로그 없이 커맨드의 쓰기 부분만 검증한다(경로는 테스트가 고정).
        let dir = std::env::temp_dir().join(format!("diary-export-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let md = "# 컴파일러의 일기\n\n## 2026-07-25 14:30\n\n한글 본문\n";
        let js = "{\"kind\":\"agent-office.diary\"}";

        let p_md = dir.join("일기.md");
        std::fs::write(&p_md, diary_content_for(&p_md, md, js).as_bytes()).unwrap();
        assert_eq!(std::fs::read_to_string(&p_md).unwrap(), md);

        let p_js = dir.join("일기.json");
        std::fs::write(&p_js, diary_content_for(&p_js, md, js).as_bytes()).unwrap();
        assert_eq!(std::fs::read_to_string(&p_js).unwrap(), js);

        let _ = std::fs::remove_dir_all(&dir);
    }
}
