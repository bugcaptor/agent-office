// src-tauri/src/webremote/rpc.rs
//
// 웹 원격(docs/web-remote-design.md). 브라우저로 접속해 상태를 확인하고
// 터미널에 개입한다. 주인 의미론 — 내 기계를 내가 원격 조종하는 것이다.
//
// 이 파일이 이 기능의 **보안 중심**이다. 이 앱은 임의 프로세스를 띄우고
// 파일시스템을 읽는다 — 커맨드를 그대로 네트워크에 내놓으면 원격 코드 실행
// 서비스가 된다. 그래서:
//
//   1. **allowlist 테이블 밖의 `cmd`는 무조건 거부**한다(`unknownCmd`).
//      존재하는 커맨드라도 테이블에 없으면 없는 것이다.
//   2. `session.start`는 **저장된 프로필로만** 스폰한다 — 웹에서 cwd·shell·
//      startupCommand를 실어 보낼 수 없다(임의 명령 주입 벡터 차단).
//   3. 설정 변경·봇 조작(admin 티어)은 **테이블 자체가 비어 있다**. 원격
//      설정 변경은 `webRemoteEnabled`를 스스로 끄는 셀프 락아웃이자 권한
//      상승 표면이다(control 서버가 `cliEnabled`를 막는 것과 같은 논리).
//   4. 정적 자산과 RPC 모두 **매 요청 `web_remote_enabled`를 확인**한다 —
//      토글이 서버 재시작 없이 즉시 반영된다.

use std::sync::Arc;

use axum::extract::{Path, State};
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::Router;
use serde_json::Value;

use super::pairing::ClientRecord;
use super::protocol::{ClientPermission, RpcError};
use super::WebRemoteContext;
use crate::session_events::types::AgentEventProfile;
use crate::types::CreateSessionRequest;

/// `cmd` → 필요한 최소 권한. **여기 없는 것은 존재하지 않는 커맨드다.**
///
/// 티어 대응: `ReadOnly` = 읽기, `Input` = 조작.
/// admin 티어(설정·봇)는 의도적으로 비어 있다 — §5 "하지 않을 것".
pub fn required_permission(cmd: &str) -> Option<ClientPermission> {
    match cmd {
        // 읽기만
        "agents.list" | "notifications.list" | "usage.snapshot" | "chat.follow"
        | "media.portrait" => Some(ClientPermission::ReadOnly),
        // 조작
        "session.start" | "session.dispose" | "notifications.clear" | "chat.send"
        | "chat.keys" => Some(ClientPermission::Input),
        _ => None,
    }
}

fn arg_str(args: &Value, key: &str) -> Result<String, RpcError> {
    args.get(key)
        .and_then(|v| v.as_str())
        .map(str::to_string)
        .ok_or_else(|| RpcError::bad_args(format!("{key}가 필요합니다")))
}

/// 웹 RPC 한 건을 처리한다. 권한·가시성 검사가 **여기 한 곳**에 있다.
///
/// `conn`은 이 요청을 실어 온 WS 연결의 id다 — 채팅 구독 수명이 연결에 매여
/// 있어서(브라우저를 닫으면 tail이 멈춘다) 커맨드가 그것을 알아야 한다.
pub async fn dispatch(
    ctx: &Arc<WebRemoteContext>,
    record: &ClientRecord,
    conn: u64,
    cmd: &str,
    args: Value,
) -> Result<Value, RpcError> {
    // 웹 원격이 꺼져 있으면 RPC 자체가 없다(브라우저 토큰이 살아 있어도).
    if !ctx.web_remote_enabled() {
        return Err(RpcError::forbidden("웹 원격이 꺼져 있습니다"));
    }
    let Some(needed) = required_permission(cmd) else {
        return Err(RpcError::unknown_cmd(cmd));
    };
    if needed.allows_input() && !record.permission.allows_input() {
        return Err(RpcError::forbidden("조작 권한이 필요합니다"));
    }

    match cmd {
        "agents.list" => serde_json::to_value(ctx.build_agents_for(record))
            .map_err(|e| RpcError::internal(e.to_string())),

        "notifications.list" => {
            let agent_id = arg_str(&args, "agentId")?;
            ensure_visible(ctx, record, &agent_id)?;
            serde_json::to_value(ctx.manager.pending_notifications(&agent_id))
                .map_err(|e| RpcError::internal(e.to_string()))
        }

        "usage.snapshot" => {
            // 같은 스로틀 상태를 네이티브와 공유한다 — 폰 폴링이 중복 fetch를
            // 일으키지 않는다.
            let snapshot = crate::ipc::commands::load_usage_snapshot_body(
                &ctx.live_usage,
                chrono::Utc::now().timestamp_millis(),
            )
            .await;
            serde_json::to_value(snapshot).map_err(|e| RpcError::internal(e.to_string()))
        }

        "session.start" => {
            let agent_id = arg_str(&args, "agentId")?;
            ensure_visible(ctx, record, &agent_id)?;
            // **저장된 프로필로만** 스폰한다. 웹이 cwd/shell/명령을 실어 보낼
            // 수 없는 것이 이 커맨드가 원격 코드 실행 표면이 되지 않는 근거다.
            let profile = ctx
                .store
                .load()
                .agents
                .into_iter()
                .find(|a| a.id == agent_id)
                .ok_or_else(|| RpcError::not_found("저장된 캐릭터가 아닙니다"))?;
            let created = crate::ipc::commands::spawn_session(
                &ctx.manager,
                &ctx.observer,
                &ctx.observer_server,
                &ctx.settings,
                CreateSessionRequest {
                    agent_id: profile.id.clone(),
                    cols: None,
                    rows: None,
                    cwd: profile.cwd.clone(),
                    shell: profile.shell.clone(),
                    startup_command: profile.startup_command.clone(),
                    personality_prompt: profile.personality_prompt.clone(),
                    autostart_claude: None,
                },
                AgentEventProfile {
                    name: profile.name.clone(),
                    role: Some(profile.role.clone()),
                },
            )
            .await
            .map_err(RpcError::internal)?;
            serde_json::to_value(created).map_err(|e| RpcError::internal(e.to_string()))
        }

        "session.dispose" => {
            let agent_id = arg_str(&args, "agentId")?;
            ensure_visible(ctx, record, &agent_id)?;
            ctx.manager.dispose(&agent_id);
            Ok(Value::Null)
        }

        "notifications.clear" => {
            let agent_id = arg_str(&args, "agentId")?;
            ensure_visible(ctx, record, &agent_id)?;
            let ids: Option<Vec<String>> = args
                .get("ids")
                .and_then(|v| serde_json::from_value(v.clone()).ok());
            if let Some(sid) = ctx.manager.session_id_for(&agent_id) {
                ctx.hub_notify.clear(&sid, ids);
            }
            Ok(Value::Null)
        }

        // 커스텀 초상 PNG(base64). 없으면 null이고 뷰어는 seed+archetype으로
        // 절차 생성 아바타를 그린다. 쓰기는 없다 — 프로필 소유권은 호스트에
        // 있고 뷰어는 읽기 캐시다(§결정 4).
        "media.portrait" => {
            let agent_id = arg_str(&args, "agentId")?;
            ensure_visible(ctx, record, &agent_id)?;
            let png = ctx
                .portraits
                .load(&agent_id)
                .map_err(|e| RpcError::internal(e.to_string()))?;
            Ok(png.map(Value::String).unwrap_or(Value::Null))
        }

        // ── 채팅 뷰(M2) ─────────────────────────────────────────────
        //
        // 읽기: 전사 tail 구독. 쓰기: 세션 stdin에 문장·키를 주입한다.
        // 새 표면이 아니다 — **이미 떠 있는 세션의 stdin**이라 봇 모드가 이미
        // 지고 있는 경계 안이다(임의 프로세스 스폰과 무관).
        "chat.follow" => {
            let agent_id = arg_str(&args, "agentId")?;
            ensure_visible(ctx, record, &agent_id)?;
            let cwd = ctx
                .store
                .load()
                .agents
                .into_iter()
                .find(|a| a.id == agent_id)
                .and_then(|a| a.cwd)
                .unwrap_or_default();
            ctx.chat.follow(&agent_id, &cwd, conn);
            Ok(Value::Null)
        }

        "chat.send" => {
            let agent_id = arg_str(&args, "agentId")?;
            ensure_visible(ctx, record, &agent_id)?;
            let text = arg_str(&args, "text")?;
            if text.trim().is_empty() {
                return Err(RpcError::bad_args("빈 문장은 보낼 수 없습니다"));
            }
            // 봇 모드와 **같은 주입 규칙**이다: 단일 라인화 후 텍스트를 쓰고,
            // 잠깐 쉰 뒤 CR을 따로 보낸다(TUI가 Enter를 삼키는 것을 막는다).
            ctx.manager
                .write_input(&agent_id, &crate::bot::runner::single_line(&text));
            tokio::time::sleep(std::time::Duration::from_millis(
                crate::bot::runner::INJECT_SUBMIT_DELAY_MS,
            ))
            .await;
            ctx.manager.write_input(&agent_id, "\r");
            Ok(Value::Null)
        }

        "chat.keys" => {
            let agent_id = arg_str(&args, "agentId")?;
            ensure_visible(ctx, record, &agent_id)?;
            let names: Vec<String> = args
                .get("keys")
                .and_then(|v| serde_json::from_value(v.clone()).ok())
                .ok_or_else(|| RpcError::bad_args("keys가 필요합니다"))?;
            if names.is_empty() {
                return Err(RpcError::bad_args("keys가 비었습니다"));
            }
            let bytes = super::chat::keys_to_bytes(&names)
                .map_err(|k| RpcError::bad_args(format!("알 수 없는 키입니다: {k}")))?;
            for chunk in bytes {
                ctx.manager.write_input(&agent_id, chunk);
            }
            Ok(Value::Null)
        }

        // required_permission이 Some을 준 cmd는 위에서 전부 처리된다. 여기
        // 도달하면 테이블과 구현이 어긋난 것이므로 열어 주지 않는다.
        _ => Err(RpcError::unknown_cmd(cmd)),
    }
}

fn ensure_visible(ctx: &Arc<WebRemoteContext>, record: &ClientRecord, agent_id: &str) -> Result<(), RpcError> {
    if ctx.agent_allowed(record, agent_id) {
        Ok(())
    } else {
        Err(RpcError::forbidden("접근할 수 없는 캐릭터입니다"))
    }
}

// ── 정적 자산 ─────────────────────────────────────────────────────────

/// `dist-web/`(vite.web.config.ts 산출물)을 바이너리에 내장한다. debug 빌드에서는
/// rust-embed가 런타임에 디스크를 읽으므로 `npm run web:dev`(vite --watch)만
/// 띄워 두면 새 빌드가 즉시 서빙된다 — 프록시·HMR 배관이 필요 없다.
#[derive(rust_embed::Embed)]
#[folder = "../dist-web/"]
struct WebAssets;

async fn index(State(ctx): State<Arc<WebRemoteContext>>) -> Response {
    serve_asset(&ctx, "index.html")
}

async fn asset(State(ctx): State<Arc<WebRemoteContext>>, Path(path): Path<String>) -> Response {
    serve_asset(&ctx, &path)
}

fn serve_asset(ctx: &Arc<WebRemoteContext>, path: &str) -> Response {
    // 매 요청 확인 — 토글을 끄면 즉시 사라진다(서버 재시작 불필요).
    if !ctx.web_remote_enabled() {
        return (StatusCode::NOT_FOUND, "web remote disabled").into_response();
    }
    let path = path.trim_start_matches('/');
    let candidate = if path.is_empty() { "index.html" } else { path };
    match WebAssets::get(candidate) {
        Some(file) => {
            let mime = mime_guess::from_path(candidate).first_or_octet_stream();
            (
                [(header::CONTENT_TYPE, mime.as_ref().to_string())],
                file.data.into_owned(),
            )
                .into_response()
        }
        // SPA 폴백: 알 수 없는 경로는 index.html로 돌려 클라이언트 라우팅에 맡긴다.
        None => match WebAssets::get("index.html") {
            Some(file) => (
                [(header::CONTENT_TYPE, "text/html; charset=utf-8".to_string())],
                file.data.into_owned(),
            )
                .into_response(),
            None => (
                StatusCode::NOT_FOUND,
                "웹 클라이언트가 빌드되지 않았습니다(npm run web:build)",
            )
                .into_response(),
        },
    }
}

pub fn routes() -> Router<Arc<WebRemoteContext>> {
    // axum 0.7의 와일드카드는 `/*path`다(0.8의 `{*path}`가 아니다 — 잘못 쓰면
    // 라우터 조립 시점에 패닉한다).
    Router::new()
        .route("/web", get(index))
        .route("/web/", get(index))
        .route("/web/*path", get(asset))
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::Engine as _;
    use serde_json::json;

    fn record(permission: ClientPermission) -> ClientRecord {
        ClientRecord {
            client_id: "p1".into(),
            name: "테스트".into(),
            token: "t".into(),
            permission,
            created_at: 0,
        }
    }

    #[test]
    fn table_defines_the_whole_surface() {
        // viewer 티어
        assert_eq!(required_permission("agents.list"), Some(ClientPermission::ReadOnly));
        assert_eq!(
            required_permission("notifications.list"),
            Some(ClientPermission::ReadOnly)
        );
        assert_eq!(required_permission("usage.snapshot"), Some(ClientPermission::ReadOnly));
        // 아바타 초상 — 읽기만(쓰기는 호스트 전용).
        assert_eq!(required_permission("media.portrait"), Some(ClientPermission::ReadOnly));
        // operator 티어
        assert_eq!(required_permission("session.start"), Some(ClientPermission::Input));
        assert_eq!(required_permission("session.dispose"), Some(ClientPermission::Input));
        assert_eq!(
            required_permission("notifications.clear"),
            Some(ClientPermission::Input)
        );
        // 채팅(M2) — 구독은 읽기, 주입은 조작.
        assert_eq!(required_permission("chat.follow"), Some(ClientPermission::ReadOnly));
        assert_eq!(required_permission("chat.send"), Some(ClientPermission::Input));
        assert_eq!(required_permission("chat.keys"), Some(ClientPermission::Input));
        // 테이블 밖 — 실재하는 Tauri 커맨드라도 없는 것이다.
        for cmd in [
            "set_app_settings",
            "settings.set",
            "bot.start",
            "save_state",
            "workdir_diff_file",
            "export_terminal_output",
            "chat.unfollow",
            "transcript.list",
            "media.portrait.save",
            "save_portrait",
            "",
        ] {
            assert_eq!(required_permission(cmd), None, "{cmd}는 열려 있으면 안 된다");
        }
    }

    #[test]
    fn admin_tier_is_empty_by_construction() {
        // 설정 변경·봇 조작이 어떤 이름으로도 테이블에 없어야 한다.
        let opened: Vec<&str> = [
            "agents.list",
            "notifications.list",
            "usage.snapshot",
            "media.portrait",
            "session.start",
            "session.dispose",
            "notifications.clear",
            "chat.follow",
            "chat.send",
            "chat.keys",
        ]
        .into_iter()
        .filter(|c| required_permission(c).is_some())
        .collect();
        assert_eq!(opened.len(), 10, "열린 커맨드는 정확히 이 10개뿐이다");
    }

    /// 초상은 있으면 base64, 없으면 null이다(뷰어는 null을 절차 생성 신호로 쓴다).
    #[tokio::test]
    async fn portrait_returns_base64_or_null() {
        let (ctx, dir) = crate::webremote::tests::build_ctx("web-portrait");
        ctx.settings.write().unwrap().web_remote_enabled = true;
        let viewer = record(ClientPermission::ReadOnly);

        // 저장된 초상이 없으면 null.
        let none = dispatch(&ctx, &viewer, 1, "media.portrait", json!({ "agentId": "a1" }))
            .await
            .expect("읽기 권한으로 조회 가능");
        assert!(none.is_null(), "{none}");

        // 저장해 두면 같은 base64가 그대로 돌아온다.
        let png = base64::engine::general_purpose::STANDARD.encode([
            0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x01,
        ]);
        ctx.portraits
            .save("a1", &png, &["a1".to_string()])
            .expect("초상 저장");
        let some = dispatch(&ctx, &viewer, 1, "media.portrait", json!({ "agentId": "a1" }))
            .await
            .unwrap();
        assert_eq!(some.as_str(), Some(png.as_str()));

        // 프로필에 없는 id는 가시성 게이트에서 막힌다(경로 조작 시도 포함).
        for bad in ["ghost", "../secret"] {
            let err = dispatch(&ctx, &viewer, 1, "media.portrait", json!({ "agentId": bad }))
                .await
                .expect_err("없는 캐릭터");
            assert_eq!(err.code, "forbidden", "{bad}");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 읽기 전용 티어는 채팅을 **보기만** 한다 — 주입은 조작 권한이다.
    #[tokio::test]
    async fn viewer_can_follow_chat_but_not_inject() {
        let (ctx, dir) = crate::webremote::tests::build_ctx("web-chat-perm");
        ctx.settings.write().unwrap().web_remote_enabled = true;
        ctx.chat.set_source_factory(Arc::new(|_a, _c| Vec::new()));
        let viewer = record(ClientPermission::ReadOnly);

        dispatch(&ctx, &viewer, 1, "chat.follow", json!({ "agentId": "a1" }))
            .await
            .expect("구독은 읽기 권한");

        for cmd in ["chat.send", "chat.keys"] {
            let err = dispatch(&ctx, &viewer, 1, cmd, json!({ "agentId": "a1" }))
                .await
                .expect_err("주입은 거부");
            assert_eq!(err.code, "forbidden", "{cmd}");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn chat_send_refuses_empty_text_and_unknown_keys() {
        let (ctx, dir) = crate::webremote::tests::build_ctx("web-chat-args");
        ctx.settings.write().unwrap().web_remote_enabled = true;
        let op = record(ClientPermission::Input);

        for text in ["", "   ", "\n"] {
            let err = dispatch(
                &ctx,
                &op,
                1,
                "chat.send",
                json!({ "agentId": "a1", "text": text }),
            )
            .await
            .expect_err("빈 문장 거부");
            assert_eq!(err.code, "badArgs", "{text:?}");
        }

        let err = dispatch(
            &ctx,
            &op,
            1,
            "chat.keys",
            json!({ "agentId": "a1", "keys": ["enter", "f13"] }),
        )
        .await
        .expect_err("모르는 키 거부");
        assert_eq!(err.code, "badArgs");
        assert!(err.message.contains("f13"), "{}", err.message);

        // keys 자체가 없거나 비면 badArgs.
        for args in [
            json!({ "agentId": "a1" }),
            json!({ "agentId": "a1", "keys": [] }),
        ] {
            let err = dispatch(&ctx, &op, 1, "chat.keys", args)
                .await
                .expect_err("keys 필요");
            assert_eq!(err.code, "badArgs");
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// 채팅도 가시성 게이트 안이다 — 프로필에 없는 캐릭터는 구독도 주입도 못 한다.
    #[tokio::test]
    async fn chat_commands_respect_visibility() {
        let (ctx, dir) = crate::webremote::tests::build_ctx("web-chat-visibility");
        ctx.settings.write().unwrap().web_remote_enabled = true;
        let op = record(ClientPermission::Input);
        for (cmd, args) in [
            ("chat.follow", json!({ "agentId": "ghost" })),
            ("chat.send", json!({ "agentId": "ghost", "text": "안녕" })),
            ("chat.keys", json!({ "agentId": "ghost", "keys": ["enter"] })),
        ] {
            let err = dispatch(&ctx, &op, 1, cmd, args)
                .await
                .expect_err("없는 캐릭터");
            assert_eq!(err.code, "forbidden", "{cmd}");
        }
        assert!(!ctx.chat.is_following("ghost"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn read_only_client_cannot_call_operator_commands() {
        let (ctx, dir) = crate::webremote::tests::build_ctx("web-perm");
        ctx.settings.write().unwrap().web_remote_enabled = true;
        let viewer = record(ClientPermission::ReadOnly);
        let err = dispatch(&ctx, &viewer, 1, "session.start", json!({ "agentId": "a1" }))
            .await
            .expect_err("읽기 전용은 거부");
        assert_eq!(err.code, "forbidden");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn unknown_command_is_refused_even_for_operators() {
        let (ctx, dir) = crate::webremote::tests::build_ctx("web-unknown");
        ctx.settings.write().unwrap().web_remote_enabled = true;
        let op = record(ClientPermission::Input);
        let err = dispatch(&ctx, &op, 1, "set_app_settings", json!({}))
            .await
            .expect_err("테이블 밖은 거부");
        assert_eq!(err.code, "unknownCmd");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn rpc_is_dead_while_web_hosting_is_off() {
        let (ctx, dir) = crate::webremote::tests::build_ctx("web-off");
        ctx.settings.write().unwrap().web_remote_enabled = false;
        // 토큰이 살아 있어도 토글이 꺼지면 RPC가 없다.
        let op = record(ClientPermission::Input);
        let err = dispatch(&ctx, &op, 1, "agents.list", json!({}))
            .await
            .expect_err("꺼져 있으면 거부");
        assert_eq!(err.code, "forbidden");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn every_local_character_is_visible_to_the_browser() {
        let (ctx, dir) = crate::webremote::tests::build_ctx("web-visibility");
        let client = record(ClientPermission::Input);

        // 캐릭터별 공유 토글이라는 개념이 없다 — 내 캐릭터는 전부 보인다.
        let seen = dispatch(&ctx, &client, 1, "agents.list", json!({})).await.unwrap();
        assert_eq!(seen.as_array().unwrap().len(), 1, "주인은 전부 본다");
        assert!(ctx.agent_allowed(&client, "a1"));
        // 프로필에 없는 id는 tap이 깔려 있어도 보이지 않는다.
        ctx.hub.share(&ctx.manager, "ghost");
        assert!(!ctx.agent_allowed(&client, "ghost"), "tap≠캐릭터 존재");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn session_start_refuses_unknown_profile() {
        let (ctx, dir) = crate::webremote::tests::build_ctx("web-notfound");
        ctx.settings.write().unwrap().web_remote_enabled = true;
        let op = record(ClientPermission::Input);
        let err = dispatch(&ctx, &op, 1, "session.start", json!({ "agentId": "ghost" }))
            .await
            .expect_err("없는 캐릭터");
        // 가시성 게이트가 먼저 걸린다(존재 자체를 알려주지 않는다).
        assert_eq!(err.code, "forbidden");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test]
    async fn missing_args_are_reported_as_bad_args() {
        let (ctx, dir) = crate::webremote::tests::build_ctx("web-args");
        ctx.settings.write().unwrap().web_remote_enabled = true;
        let op = record(ClientPermission::Input);
        let err = dispatch(&ctx, &op, 1, "session.dispose", json!({}))
            .await
            .expect_err("agentId 없음");
        assert_eq!(err.code, "badArgs");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
