// src-tauri/src/control/client.rs
//
// `agent-office ctl …` 서브커맨드(이슈 #55, docs/cli-control-design.md). 같은
// 바이너리를 인자 분기로 실행하되 GUI(`run()`)에 도달하지 않는 단명 클라이언트
// — `observer-forward`/`sessiond`와 동일한 검증된 패턴이다(lib.rs). 실행 중인
// GUI 앱의 control 서버에 붙어 요청 1건을 보내고 stdout에 출력한 뒤 종료한다.
//
// 발견 순서: `--app-data`/`--port`/`--token` 플래그 > `AGENT_OFFICE_APP_DATA`
// env(세션 터미널엔 앱이 자동 주입) > OS별 표준 app_data 경로. app_data에서
// `control-port`/`control-token`을 읽어 `POST http://127.0.0.1:<port>/v1/<cmd>`.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::time::Duration;

use serde_json::{json, Map, Value};

use super::protocol::TOKEN_HEADER;
use super::read_token_at;

const IDENTIFIER: &str = "com.bugcaptor.agent-office";

// 종료 코드 계약(docs/cli-control-design.md §종료 코드).
const EXIT_OK: i32 = 0;
const EXIT_CMD_ERROR: i32 = 1; // 서버가 ok:false로 거절
const EXIT_CONNECT: i32 = 2; // 연결 실패(서버 없음/네트워크)
const EXIT_NO_APP: i32 = 3; // 포트 파일 없음(앱 미실행 또는 CLI 제어 OFF)
const EXIT_NOT_APPROVED: i32 = 4; // 토큰 없음(미승인)
const EXIT_UNAUTHORIZED: i32 = 5; // 401(토큰 무효/취소됨)
const EXIT_NO_REPLY: i32 = 8; // talk ask: 시간 안에 답이 오지 않음
const EXIT_USAGE: i32 = 64; // 잘못된 사용법

/// 파싱된 CLI 호출. `positionals[0]`이 서브커맨드다.
#[derive(Debug, Default, PartialEq)]
struct Parsed {
    json: bool,
    enter: bool,
    kv: BTreeMap<String, String>,
    positionals: Vec<String>,
}

impl Parsed {
    fn sub(&self) -> &str {
        self.positionals.first().map(String::as_str).unwrap_or("")
    }
}

/// 인자 토큰(프로그램/`ctl` 제거 후)을 Parsed로. `--` 이후는 전부 위치인자.
fn parse(args: &[String]) -> Result<Parsed, String> {
    let mut p = Parsed::default();
    let mut only_positional = false;
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        if only_positional {
            p.positionals.push(arg.clone());
            continue;
        }
        match arg.as_str() {
            "--" => only_positional = true,
            "--json" => p.json = true,
            "--enter" => p.enter = true,
            // `--help`/`-h`는 값을 먹는 플래그로 오인되지 않게 help 위치인자로 흡수.
            "--help" | "-h" => p.positionals.push("help".to_string()),
            other if other.starts_with("--") => {
                let rest = &other[2..];
                if let Some((k, v)) = rest.split_once('=') {
                    if k.is_empty() {
                        return Err(format!("빈 플래그 이름: {other}"));
                    }
                    p.kv.insert(k.to_string(), v.to_string());
                } else {
                    let v = it
                        .next()
                        .ok_or_else(|| format!("{other} 값이 누락되었습니다"))?;
                    p.kv.insert(rest.to_string(), v.clone());
                }
            }
            _ => p.positionals.push(arg.clone()),
        }
    }
    Ok(p)
}

/// app_data 경로 결정: 플래그 > env > OS 기본.
fn resolve_app_data(flag: Option<&str>, env: Option<&str>) -> Option<PathBuf> {
    if let Some(f) = flag.filter(|s| !s.is_empty()) {
        return Some(PathBuf::from(f));
    }
    if let Some(e) = env.filter(|s| !s.is_empty()) {
        return Some(PathBuf::from(e));
    }
    default_app_data()
}

/// Tauri v2 `app_data_dir(identifier)`을 런타임 없이 재현한다. 세션 내부에서는
/// `AGENT_OFFICE_APP_DATA`가 있어 이 경로가 필요 없고, 외부 스크립트용 폴백이다.
fn default_app_data() -> Option<PathBuf> {
    #[cfg(target_os = "macos")]
    {
        let home = std::env::var_os("HOME")?;
        Some(
            PathBuf::from(home)
                .join("Library/Application Support")
                .join(IDENTIFIER),
        )
    }
    #[cfg(target_os = "linux")]
    {
        let base = std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .filter(|p| !p.as_os_str().is_empty())
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))?;
        Some(base.join(IDENTIFIER))
    }
    #[cfg(target_os = "windows")]
    {
        let base = std::env::var_os("APPDATA")?;
        Some(PathBuf::from(base).join(IDENTIFIER))
    }
    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    {
        None
    }
}

fn read_port(app_data: &std::path::Path) -> Option<u16> {
    std::fs::read_to_string(app_data.join(super::protocol::PORT_FILE))
        .ok()?
        .trim()
        .parse()
        .ok()
}

/// 문자열 값을 JSON 스칼라로 추론(settings set 용): true/false/정수/실수/문자열.
fn infer_value(s: &str) -> Value {
    match s {
        "true" => Value::Bool(true),
        "false" => Value::Bool(false),
        _ => {
            if let Ok(i) = s.parse::<i64>() {
                json!(i)
            } else if let Ok(f) = s.parse::<f64>() {
                json!(f)
            } else {
                Value::String(s.to_string())
            }
        }
    }
}

/// attach 대상 셸의 PID. `eval "$(agent-office ctl attach X)"`에서 ctl의 부모
/// 프로세스가 곧 그 셸이다(앱은 이 PID의 생존을 5초마다 확인해 정리한다).
/// unix 외에는 확인 수단을 이식하지 않아 None — 그 경우 명시적 detach만 남는다.
fn parent_pid() -> Option<u32> {
    #[cfg(unix)]
    {
        Some(std::os::unix::process::parent_id())
    }
    #[cfg(not(unix))]
    {
        None
    }
}

/// 서브커맨드 → (라우트, 본문 JSON). 순수 함수(테스트 용이).
fn build_request(p: &Parsed) -> Result<(&'static str, Value), String> {
    let pos = &p.positionals;
    let agent = |idx: usize, cmd: &str| -> Result<String, String> {
        pos.get(idx)
            .cloned()
            .ok_or_else(|| format!("{cmd}: agentId가 필요합니다"))
    };
    match p.sub() {
        "ping" => Ok(("/v1/ping", json!({}))),
        "list" => Ok(("/v1/list", json!({}))),
        "create" => {
            let mut o = Map::new();
            o.insert("agentId".into(), Value::String(agent(1, "create")?));
            for (flag, field) in [
                ("cwd", "cwd"),
                ("shell", "shell"),
                ("startup-command", "startupCommand"),
                ("name", "name"),
                ("role", "role"),
            ] {
                if let Some(v) = p.kv.get(flag) {
                    o.insert(field.into(), Value::String(v.clone()));
                }
            }
            for (flag, field) in [("cols", "cols"), ("rows", "rows")] {
                if let Some(v) = p.kv.get(flag) {
                    let n: u16 = v
                        .parse()
                        .map_err(|_| format!("--{flag}는 숫자여야 합니다: {v}"))?;
                    o.insert(field.into(), json!(n));
                }
            }
            Ok(("/v1/create", Value::Object(o)))
        }
        "attach" => {
            // `attach --agent X` 형태도 받는다(문서의 eval 예시가 이 형태다).
            let agent = pos
                .get(1)
                .cloned()
                .or_else(|| p.kv.get("agent").cloned())
                .ok_or("attach: agentId가 필요합니다")?;
            let mut o = Map::new();
            o.insert("agentId".into(), Value::String(agent));
            // command substitution 안에서 ctl의 부모 = attach 대상 셸이다.
            if let Some(pid) = parent_pid() {
                o.insert("pid".into(), json!(pid));
            }
            // 타임라인 표시용 작업 폴더 — 명시 --cwd > 현재 디렉터리.
            let cwd = p.kv.get("cwd").cloned().or_else(|| {
                std::env::current_dir()
                    .ok()
                    .map(|d| d.to_string_lossy().into_owned())
            });
            if let Some(cwd) = cwd {
                o.insert("cwd".into(), Value::String(cwd));
            }
            // `--tmux <세션이름>`: 이 셸이 아니라 **그 tmux 세션**에 붙인다
            // (앱이 자기 PTY로 tmux 클라이언트를 연다). 이때 응답 script는
            // 코멘트뿐이라 eval해도 이 셸은 그대로다.
            if let Some(target) = p.kv.get("tmux") {
                o.insert("tmux".into(), Value::String(target.clone()));
            }
            Ok(("/v1/attach", Value::Object(o)))
        }
        "detach" => {
            let agent = pos
                .get(1)
                .cloned()
                .or_else(|| p.kv.get("agent").cloned())
                .ok_or("detach: agentId가 필요합니다")?;
            Ok(("/v1/detach", json!({ "agentId": agent })))
        }
        "send" => {
            let agent = agent(1, "send")?;
            let text = pos.get(2).ok_or("send: 보낼 텍스트가 필요합니다")?;
            let data = if p.enter {
                format!("{text}\n")
            } else {
                text.clone()
            };
            Ok(("/v1/send", json!({ "agentId": agent, "data": data })))
        }
        "dispose" => Ok(("/v1/dispose", json!({ "agentId": agent(1, "dispose")? }))),
        "notifications" => Ok((
            "/v1/notifications",
            json!({ "agentId": agent(1, "notifications")? }),
        )),
        "clear" => {
            let agent = agent(1, "clear")?;
            let ids: Vec<String> = pos.iter().skip(2).cloned().collect();
            let mut o = Map::new();
            o.insert("agentId".into(), Value::String(agent));
            if !ids.is_empty() {
                o.insert("ids".into(), json!(ids));
            }
            Ok(("/v1/clear", Value::Object(o)))
        }
        "talk" => build_talk_request(p),
        "settings" => match pos.get(1).map(String::as_str) {
            Some("get") => Ok(("/v1/settings/get", json!({}))),
            Some("set") => {
                let mut o = Map::new();
                for pair in pos.iter().skip(2) {
                    let (k, v) = pair
                        .split_once('=')
                        .ok_or_else(|| format!("settings set: key=value 형식이어야 합니다: {pair}"))?;
                    if k.is_empty() {
                        return Err(format!("settings set: 빈 키: {pair}"));
                    }
                    o.insert(k.to_string(), infer_value(v));
                }
                if o.is_empty() {
                    return Err("settings set: 최소 하나의 key=value가 필요합니다".into());
                }
                Ok(("/v1/settings/set", Value::Object(o)))
            }
            _ => Err("settings: get 또는 set 하위 명령이 필요합니다".into()),
        },
        "" => Err("명령이 필요합니다 (help 참고)".into()),
        other => Err(format!("알 수 없는 명령: {other} (help 참고)")),
    }
}


/// `talk` 하위 명령(docs/agent-talk-design.md §3.2). 발신자는 보내지 않는다 —
/// 서버가 세션 헤더로 판정한다.
fn build_talk_request(p: &Parsed) -> Result<(&'static str, Value), String> {
    let pos = &p.positionals;
    let wait_ms = |default_ms: u64| -> Result<u64, String> {
        match p.kv.get("wait") {
            Some(v) => v
                .parse::<u64>()
                .map(|secs| secs.saturating_mul(1000))
                .map_err(|_| format!("--wait는 초 단위 숫자여야 합니다: {v}")),
            None => Ok(default_ms),
        }
    };
    match pos.get(1).map(String::as_str) {
        Some("roster") => Ok(("/v1/talk/roster", json!({}))),
        Some(kind @ ("ask" | "send")) => {
            let to = pos.get(2).ok_or("talk: 상대(캐릭터 id 또는 이름)가 필요합니다")?;
            let text = pos.get(3).ok_or("talk: 보낼 내용이 필요합니다")?;
            // ask는 답까지 기다린다(기본 120초), send는 즉시 돌아온다.
            let default = if kind == "ask" { 120_000 } else { 0 };
            let mut o = Map::new();
            o.insert("to".into(), Value::String(to.clone()));
            o.insert("text".into(), Value::String(text.clone()));
            o.insert("waitMs".into(), json!(wait_ms(default)?));
            if let Some(conv) = p.kv.get("conv") {
                o.insert("convId".into(), Value::String(conv.clone()));
            }
            Ok(("/v1/talk/send", Value::Object(o)))
        }
        Some("reply") => {
            let conv = pos.get(2).ok_or("talk reply: 대화 id가 필요합니다")?;
            let text = pos.get(3).ok_or("talk reply: 답장 내용이 필요합니다")?;
            Ok((
                "/v1/talk/reply",
                json!({ "convId": conv, "text": text, "waitMs": wait_ms(0)? }),
            ))
        }
        Some("inbox") => Ok(("/v1/talk/inbox", json!({ "waitMs": wait_ms(0)? }))),
        Some("end") => {
            let conv = pos.get(2).ok_or("talk end: 대화 id가 필요합니다")?;
            Ok(("/v1/talk/end", json!({ "convId": conv })))
        }
        _ => Err("talk: roster|ask|send|reply|inbox|end 중 하나가 필요합니다".into()),
    }
}

/// 이 요청이 서버에서 최대 얼마나 붙잡힐 수 있는지(롱폴링). HTTP 타임아웃을
/// 그만큼 늘려 준다 — 기본 10초로는 `ask`가 항상 연결 실패로 끝난다.
fn request_wait_ms(body: &Value) -> u64 {
    body.get("waitMs").and_then(Value::as_u64).unwrap_or(0)
}

const USAGE: &str = "\
agent-office ctl — 실행 중인 Agent Office를 조종하는 CLI (이슈 #55)

사용법:
  agent-office ctl <명령> [인자] [옵션]

명령:
  status                          연결/승인 상태를 점검한다(토큰 없어도 동작)
  ping                            서버 연결·인증을 확인한다
  list                            프로필과 실행 중 세션 상태를 나열한다
  create <agentId> [--cwd P] [--shell S] [--startup-command C]
                     [--name N] [--role R] [--cols N] [--rows N]
  attach <agentId> [--cwd P]      이 터미널을 캐릭터에 붙인다(아래 eval 필요)
         [--tmux <세션이름>]      대신 그 tmux 세션에 붙는다(앱 탭이 미러)
  detach <agentId>                외부 터미널 연결을 끊는다
  send <agentId> <text> [--enter] 세션 stdin에 text를 주입(--enter=개행 추가)
  dispose <agentId>               세션을 종료한다
  notifications <agentId>         대기 중 알림을 나열한다
  clear <agentId> [id...]         알림을 지운다(id 없으면 전체)
  talk roster                     말 걸 수 있는 동료를 나열한다
  talk ask <상대> <내용> [--wait 초]   보내고 답을 기다린다(기본 120초)
  talk send <상대> <내용> [--conv id]  보내고 즉시 돌아온다
  talk inbox [--wait 초]          나에게 온 메시지를 가져간다
  talk reply <convId> <내용>      받은 메시지에 답한다
  talk end <convId>               대화를 닫는다
  settings get                    현재 앱 설정을 출력한다
  settings set <key=value>...     설정을 변경한다(cliEnabled 제외)

전역 옵션:
  --json                          응답 data를 JSON으로 출력(기계 파싱용)
  --app-data <경로>               app_data 위치 지정(자동발견 대체)
  --port <포트> / --token <토큰>  포트/토큰 직접 지정

외부 터미널 attach(zsh/bash 전용, fish 미지원):
  eval \"$(agent-office ctl attach 캐릭터ID)\"
  attach는 성공 시 셸 스크립트만 stdout으로 내보낸다(실패하면 아무것도 내보내지
  않으므로 eval이 안전하다). 이 셸이 종료되면 앱이 5초 안에 연결을 정리한다.

tmux 세션 attach(출력 미러링·입력 주입까지 되는 풀 기능):
  agent-office ctl attach 캐릭터ID --tmux 세션이름
  앱이 그 tmux 세션에 클라이언트로 붙어 앱 탭이 미러가 된다. 각 pane의 훅·성격은
  그 pane 안에서 eval \"$(agent-office ctl attach 캐릭터ID)\"로 따로 붙인다.
";

/// `ctl` 진입점 — `ctl` 이후의 인자 토큰을 받아 종료 코드를 돌려준다.
pub fn run(args: Vec<String>) -> i32 {
    let parsed = match parse(&args) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("ctl: {e}");
            return EXIT_USAGE;
        }
    };
    // parse가 --help/-h를 "help" 위치인자로 흡수하므로 여기선 "help"/빈값만 본다.
    if matches!(parsed.sub(), "" | "help") {
        print!("{USAGE}");
        return EXIT_OK;
    }

    let env_app_data = std::env::var("AGENT_OFFICE_APP_DATA").ok();
    let app_data = resolve_app_data(parsed.kv.get("app-data").map(String::as_str), env_app_data.as_deref());

    if parsed.sub() == "status" {
        return run_status(&parsed, app_data);
    }

    let Some(app_data) = app_data else {
        eprintln!("ctl: app_data 경로를 찾을 수 없습니다 — --app-data 로 지정하세요");
        return EXIT_NO_APP;
    };

    let port = match parsed.kv.get("port").and_then(|s| s.parse::<u16>().ok()) {
        Some(p) => p,
        None => match read_port(&app_data) {
            Some(p) => p,
            None => {
                eprintln!(
                    "ctl: 실행 중인 Agent Office를 찾을 수 없습니다 \
                     (앱이 실행 중이고 설정에서 CLI 제어가 켜져 있는지 확인하세요)"
                );
                return EXIT_NO_APP;
            }
        },
    };

    let token = match parsed.kv.get("token").cloned().or_else(|| read_token_at(&app_data)) {
        Some(t) => t,
        None => {
            eprintln!(
                "ctl: CLI 제어가 아직 승인되지 않았습니다 \
                 (앱 설정에서 'CLI 제어 승인'을 눌러 토큰을 발급하세요)"
            );
            return EXIT_NOT_APPROVED;
        }
    };

    let (path, body) = match build_request(&parsed) {
        Ok(x) => x,
        Err(e) => {
            eprintln!("ctl: {e}");
            return EXIT_USAGE;
        }
    };

    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(e) => {
            eprintln!("ctl: 런타임 생성 실패: {e}");
            return EXIT_CONNECT;
        }
    };

    match runtime.block_on(send(port, &token, path, &body)) {
        Err(e) => {
            eprintln!("ctl: {e}");
            EXIT_CONNECT
        }
        Ok((401, _)) => {
            eprintln!("ctl: 인증 실패 — 토큰이 유효하지 않습니다(앱에서 재승인이 필요할 수 있습니다)");
            EXIT_UNAUTHORIZED
        }
        Ok((_, value)) => {
            if value.get("ok").and_then(Value::as_bool) == Some(true) {
                let data = &value["data"];
                print_success(&parsed, data);
                // `ask`는 답을 받아 오는 명령이다 — 시간 안에 못 받으면 성공이 아니다.
                if parsed.sub() == "talk"
                    && parsed.positionals.get(1).map(String::as_str) == Some("ask")
                    && data.get("reply").is_none_or(Value::is_null)
                {
                    return EXIT_NO_REPLY;
                }
                EXIT_OK
            } else {
                let msg = value
                    .get("error")
                    .and_then(Value::as_str)
                    .unwrap_or("알 수 없는 오류");
                eprintln!("ctl: {msg}");
                EXIT_CMD_ERROR
            }
        }
    }
}

async fn send(
    port: u16,
    token: &str,
    path: &str,
    body: &Value,
) -> Result<(u16, Value), String> {
    // 롱폴링(talk ask/inbox)은 서버가 최대 waitMs 동안 붙잡는다 — 그만큼 여유를 준다.
    let timeout = Duration::from_secs(10) + Duration::from_millis(request_wait_ms(body));
    let client = reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(timeout)
        .build()
        .map_err(|e| format!("HTTP 클라이언트 생성 실패: {e}"))?;
    let mut req = client
        .post(format!("http://127.0.0.1:{port}{path}"))
        .header(TOKEN_HEADER, token)
        .json(body);
    // 발신자 신원: 앱이 세션 셸에 심어 둔 값. 없으면 서버가 400으로 거절한다
    // (앱 밖 셸에서 남을 사칭할 수 없다 — docs/agent-talk-design.md §1).
    if let Ok(session) = std::env::var("AGENT_OFFICE_SESSION") {
        if !session.trim().is_empty() {
            req = req.header(super::protocol::SESSION_HEADER, session);
        }
    }
    let resp = req
        .send()
        .await
        .map_err(|e| format!("연결 실패: {e}"))?;
    let status = resp.status().as_u16();
    let value = resp.json::<Value>().await.unwrap_or(Value::Null);
    Ok((status, value))
}

/// 토큰 없이도 동작하는 진단 — app_data/포트/토큰 존재와 실제 ping 결과를 보고.
fn run_status(parsed: &Parsed, app_data: Option<PathBuf>) -> i32 {
    let Some(app_data) = app_data else {
        eprintln!("ctl: app_data 경로를 찾을 수 없습니다 — --app-data 로 지정하세요");
        return EXIT_NO_APP;
    };
    let port = parsed
        .kv
        .get("port")
        .and_then(|s| s.parse::<u16>().ok())
        .or_else(|| read_port(&app_data));
    let token = parsed
        .kv
        .get("token")
        .cloned()
        .or_else(|| read_token_at(&app_data));

    println!("app_data: {}", app_data.display());
    println!(
        "control-port: {}",
        port.map(|p| p.to_string()).unwrap_or_else(|| "없음 (앱 미실행 또는 CLI 제어 OFF)".into())
    );
    println!(
        "control-token: {}",
        if token.is_some() {
            "있음 (승인됨)"
        } else {
            "없음 (미승인 — 앱 설정에서 승인 필요)"
        }
    );

    match (port, token) {
        (Some(port), Some(token)) => {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("current-thread runtime");
            match runtime.block_on(send(port, &token, "/v1/ping", &json!({}))) {
                Ok((200, value)) if value.get("ok").and_then(Value::as_bool) == Some(true) => {
                    let d = &value["data"];
                    println!(
                        "연결: OK — agent-office v{} (agents={}, running={})",
                        d["appVersion"].as_str().unwrap_or("?"),
                        d["agentCount"],
                        d["runningCount"]
                    );
                    EXIT_OK
                }
                Ok((401, _)) => {
                    println!("연결: 인증 실패(토큰 무효) — 재승인이 필요합니다");
                    EXIT_UNAUTHORIZED
                }
                Ok((code, _)) => {
                    println!("연결: 예기치 않은 응답 코드 {code}");
                    EXIT_CONNECT
                }
                Err(e) => {
                    println!("연결: 실패 — {e}");
                    EXIT_CONNECT
                }
            }
        }
        (None, _) => EXIT_NO_APP,
        (_, None) => EXIT_NOT_APPROVED,
    }
}

fn print_success(parsed: &Parsed, data: &Value) {
    print!("{}", render_success(parsed, data));
}

/// 성공 응답의 stdout 표현을 그대로 만든다(개행 포함). attach는 셸이 eval할
/// 스크립트라 **한 글자도 덧붙이지 않고** 내보내야 하므로, 출력을 문자열로
/// 조립해 한 번에 쓴다(사람용 안내는 전부 stderr에 남는다).
fn render_success(parsed: &Parsed, data: &Value) -> String {
    use std::fmt::Write as _;

    if parsed.json {
        return format!(
            "{}\n",
            serde_json::to_string_pretty(data).unwrap_or_else(|_| data.to_string())
        );
    }
    let mut out = String::new();
    match parsed.sub() {
        "ping" => {
            let _ = writeln!(
                out,
                "connected: agent-office v{} (agents={}, running={})",
                data["appVersion"].as_str().unwrap_or("?"),
                data["agentCount"],
                data["runningCount"]
            );
        }
        "list" => match data.as_array() {
            Some(rows) if !rows.is_empty() => {
                for r in rows {
                    let _ = writeln!(
                        out,
                        "{:<16} {:<9} {}",
                        r["agentId"].as_str().unwrap_or("?"),
                        r["state"].as_str().unwrap_or("-"),
                        r["name"].as_str().unwrap_or("")
                    );
                }
            }
            _ => out.push_str("(프로필 없음)\n"),
        },
        "create" => {
            let _ = writeln!(
                out,
                "created: {} ({})",
                data["sessionId"].as_str().unwrap_or("?"),
                data["state"].as_str().unwrap_or("?")
            );
        }
        // eval 대상 — 스크립트 원문만 나간다.
        "attach" => out.push_str(data["script"].as_str().unwrap_or_default()),
        "notifications" => match data.as_array() {
            Some(rows) if !rows.is_empty() => {
                for r in rows {
                    let _ = writeln!(
                        out,
                        "- [{}] {}",
                        r["source"].as_str().unwrap_or("?"),
                        r["message"].as_str().unwrap_or("")
                    );
                }
            }
            _ => out.push_str("(알림 없음)\n"),
        },
        "talk" => render_talk(parsed, data, &mut out),
        "settings" => {
            let _ = writeln!(
                out,
                "{}",
                serde_json::to_string_pretty(data).unwrap_or_else(|_| data.to_string())
            );
        }
        _ => out.push_str("ok\n"),
    }
    out
}


/// `talk` 응답의 사람용 출력. 에이전트가 그대로 읽고 판단할 문장이라 사유를
/// 감추지 않는다(닿지 않는 동료는 이유까지 보여 준다).
fn render_talk(parsed: &Parsed, data: &Value, out: &mut String) {
    use std::fmt::Write as _;
    let msg_line = |m: &Value| {
        format!(
            "[{}] {}: {}",
            m["convId"].as_str().unwrap_or("?"),
            m["fromName"].as_str().unwrap_or("?"),
            m["text"].as_str().unwrap_or("")
        )
    };
    match parsed.positionals.get(1).map(String::as_str) {
        Some("roster") => match data.as_array() {
            Some(rows) if !rows.is_empty() => {
                for r in rows {
                    let mark = if r["reachable"].as_bool() == Some(true) {
                        if r["busy"].as_bool() == Some(true) {
                            "작업 중"
                        } else {
                            "대기"
                        }
                    } else {
                        r["reason"].as_str().unwrap_or("불가")
                    };
                    let _ = writeln!(
                        out,
                        "{:<16} {:<12} {:<10} {}",
                        r["agentId"].as_str().unwrap_or("?"),
                        r["name"].as_str().unwrap_or(""),
                        mark,
                        r["role"].as_str().unwrap_or("")
                    );
                }
            }
            _ => out.push_str("(동료 없음)\n"),
        },
        Some("inbox") => match data.as_array() {
            Some(rows) if !rows.is_empty() => {
                for m in rows {
                    let _ = writeln!(out, "{}", msg_line(m));
                }
            }
            _ => out.push_str("(새 메시지 없음)\n"),
        },
        Some("ask" | "send" | "reply") => {
            let conv = data["convId"].as_str().unwrap_or("?");
            match data.get("reply") {
                Some(reply) if !reply.is_null() => {
                    let _ = writeln!(out, "{}", msg_line(reply));
                }
                _ => {
                    let _ = writeln!(
                        out,
                        "보냈습니다(conv={conv}). 답은 나중에 `talk inbox`로 확인하세요."
                    );
                }
            }
        }
        _ => out.push_str("ok\n"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(s: &[&str]) -> Vec<String> {
        s.iter().map(|x| x.to_string()).collect()
    }

    #[test]
    fn parse_flags_positionals_and_values() {
        let p = parse(&args(&[
            "send", "builder", "npm test", "--enter", "--json", "--token", "abc",
        ]))
        .unwrap();
        assert_eq!(p.sub(), "send");
        assert_eq!(p.positionals, vec!["send", "builder", "npm test"]);
        assert!(p.enter);
        assert!(p.json);
        assert_eq!(p.kv.get("token").map(String::as_str), Some("abc"));
    }

    #[test]
    fn parse_supports_equals_and_double_dash_terminator() {
        let p = parse(&args(&["send", "a1", "--", "--enter"])).unwrap();
        // `--` 이후는 위치인자 → 텍스트가 "--enter"로 들어간다(개행 없음).
        assert_eq!(p.positionals, vec!["send", "a1", "--enter"]);
        assert!(!p.enter);

        let p = parse(&args(&["create", "a1", "--cwd=/tmp/x"])).unwrap();
        assert_eq!(p.kv.get("cwd").map(String::as_str), Some("/tmp/x"));
    }

    #[test]
    fn parse_missing_flag_value_errors() {
        assert!(parse(&args(&["create", "a1", "--cwd"])).is_err());
    }

    #[test]
    fn build_create_maps_kebab_flags_to_camel_fields() {
        let p = parse(&args(&[
            "create",
            "reviewer",
            "--cwd",
            "~/proj",
            "--startup-command",
            "source ./init.sh",
            "--cols",
            "100",
        ]))
        .unwrap();
        let (path, body) = build_request(&p).unwrap();
        assert_eq!(path, "/v1/create");
        assert_eq!(body["agentId"], "reviewer");
        assert_eq!(body["cwd"], "~/proj");
        assert_eq!(body["startupCommand"], "source ./init.sh");
        assert_eq!(body["cols"], 100);
    }

    #[test]
    fn build_send_appends_newline_only_with_enter() {
        let (_, without) = build_request(&parse(&args(&["send", "b", "ls"])).unwrap()).unwrap();
        assert_eq!(without["data"], "ls");
        let (_, with) =
            build_request(&parse(&args(&["send", "b", "ls", "--enter"])).unwrap()).unwrap();
        assert_eq!(with["data"], "ls\n");
    }

    #[test]
    fn build_settings_set_infers_types_and_requires_pairs() {
        let (path, body) = build_request(
            &parse(&args(&[
                "settings",
                "set",
                "typingSoundEnabled=false",
                "soundVolume=0.25",
                "attentionHoldMs=3000",
                "summaryProvider=codex",
            ]))
            .unwrap(),
        )
        .unwrap();
        assert_eq!(path, "/v1/settings/set");
        assert_eq!(body["typingSoundEnabled"], false);
        assert_eq!(body["soundVolume"], 0.25);
        assert_eq!(body["attentionHoldMs"], 3000);
        assert_eq!(body["summaryProvider"], "codex");

        assert!(build_request(&parse(&args(&["settings", "set", "novalue"])).unwrap()).is_err());
        assert!(build_request(&parse(&args(&["settings", "set"])).unwrap()).is_err());
    }

    #[test]
    fn build_clear_includes_ids_only_when_present() {
        let (_, all) = build_request(&parse(&args(&["clear", "a1"])).unwrap()).unwrap();
        assert!(all.get("ids").is_none());
        let (_, some) = build_request(&parse(&args(&["clear", "a1", "n1", "n2"])).unwrap()).unwrap();
        assert_eq!(some["ids"], json!(["n1", "n2"]));
    }

    #[test]
    fn build_rejects_missing_agent_and_unknown_command() {
        assert!(build_request(&parse(&args(&["send", "a1"])).unwrap()).is_err());
        assert!(build_request(&parse(&args(&["dispose"])).unwrap()).is_err());
        assert!(build_request(&parse(&args(&["bogus"])).unwrap()).is_err());
    }

    #[test]
    fn build_attach_collects_pid_and_cwd_automatically() {
        let (path, body) = build_request(&parse(&args(&["attach", "a1"])).unwrap()).unwrap();
        assert_eq!(path, "/v1/attach");
        assert_eq!(body["agentId"], "a1");
        assert_eq!(
            body["cwd"],
            json!(std::env::current_dir()
                .unwrap()
                .to_string_lossy()
                .into_owned())
        );
        #[cfg(unix)]
        assert_eq!(body["pid"], json!(std::os::unix::process::parent_id()));
        #[cfg(not(unix))]
        assert!(body.get("pid").is_none());

        // --cwd는 자동 수집을 덮어쓴다. agentId는 --agent 형태로도 받는다.
        let (_, explicit) =
            build_request(&parse(&args(&["attach", "--agent", "a2", "--cwd", "/tmp/x"])).unwrap())
                .unwrap();
        assert_eq!(explicit["agentId"], "a2");
        assert_eq!(explicit["cwd"], "/tmp/x");

        assert!(build_request(&parse(&args(&["attach"])).unwrap()).is_err());
    }

    #[test]
    fn build_attach_forwards_the_tmux_target_only_when_given() {
        let (_, plain) = build_request(&parse(&args(&["attach", "a1"])).unwrap()).unwrap();
        assert!(plain.get("tmux").is_none());

        let (path, body) =
            build_request(&parse(&args(&["attach", "a1", "--tmux", "work"])).unwrap()).unwrap();
        assert_eq!(path, "/v1/attach");
        assert_eq!(body["agentId"], "a1");
        assert_eq!(body["tmux"], "work");

        // `--tmux=이름` 형태도 같은 필드로 들어간다.
        let (_, eq) =
            build_request(&parse(&args(&["attach", "--agent", "a2", "--tmux=my sess"])).unwrap())
                .unwrap();
        assert_eq!(eq["tmux"], "my sess");
    }

    #[test]
    fn build_detach_requires_an_agent() {
        let (path, body) = build_request(&parse(&args(&["detach", "a1"])).unwrap()).unwrap();
        assert_eq!(path, "/v1/detach");
        assert_eq!(body, json!({ "agentId": "a1" }));
        assert!(build_request(&parse(&args(&["detach"])).unwrap()).is_err());
    }

    #[test]
    fn attach_success_output_is_the_raw_script_only() {
        let parsed = parse(&args(&["attach", "a1"])).unwrap();
        let data = json!({
            "sessionId": "sid-1",
            "mode": "new",
            "script": "export AGENT_OFFICE_SESSION='sid-1'\nclaude() {\n  command claude \"$@\"\n}\n",
        });
        // eval에 그대로 먹일 수 있어야 한다 — 접두/접미 한 글자도 붙지 않는다.
        assert_eq!(
            render_success(&parsed, &data),
            data["script"].as_str().unwrap()
        );

        // --json이면 사람/기계용 JSON(이 경우 eval 대상이 아니다).
        let json_parsed = parse(&args(&["attach", "a1", "--json"])).unwrap();
        assert!(render_success(&json_parsed, &data).contains("\"sessionId\""));
    }

    #[test]
    fn resolve_app_data_prefers_flag_then_env() {
        assert_eq!(
            resolve_app_data(Some("/flag"), Some("/env")),
            Some(PathBuf::from("/flag"))
        );
        assert_eq!(
            resolve_app_data(None, Some("/env")),
            Some(PathBuf::from("/env"))
        );
        assert_eq!(resolve_app_data(Some(""), Some("")), default_app_data());
    }

    #[test]
    fn infer_value_covers_bool_int_float_string() {
        assert_eq!(infer_value("true"), Value::Bool(true));
        assert_eq!(infer_value("false"), Value::Bool(false));
        assert_eq!(infer_value("42"), json!(42));
        assert_eq!(infer_value("0.5"), json!(0.5));
        assert_eq!(infer_value("codex"), Value::String("codex".into()));
    }
}
