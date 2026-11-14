// src-tauri/src/control/client.rs
//
// `agent-office ctl …` 서브커맨드(이슈 #55). 같은
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

// 종료 코드 계약.
const EXIT_OK: i32 = 0;
const EXIT_CMD_ERROR: i32 = 1; // 서버가 ok:false로 거절
const EXIT_CONNECT: i32 = 2; // 연결 실패(서버 없음/네트워크)
const EXIT_NO_APP: i32 = 3; // 포트 파일 없음(앱 미실행 또는 CLI 제어 OFF)
const EXIT_NOT_APPROVED: i32 = 4; // 토큰 없음(미승인)
const EXIT_UNAUTHORIZED: i32 = 5; // 401(토큰 무효/취소됨)
const EXIT_USAGE: i32 = 64; // 잘못된 사용법
const EXIT_ATTENTION: i32 = 10; // 기다린 끝에 사람·외부 에이전트의 조치가 필요(멈춤·승인 대기)
const EXIT_TIMEOUT: i32 = 11; // 기다리는 시간이 다 됐다(아직 진행 중)
/// `autorun status --wait`에 `--timeout`이 없을 때 기다리는 시간(초).
const DEFAULT_WAIT_SECS: u64 = 600;

/// 파싱된 CLI 호출. `positionals[0]`이 서브커맨드다.
#[derive(Debug, Default, PartialEq)]
struct Parsed {
    json: bool,
    enter: bool,
    wait: bool,
    no_usage_guard: bool,
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
            "--wait" => p.wait = true,
            "--no-usage-guard" => p.no_usage_guard = true,
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
pub(crate) fn default_app_data() -> Option<PathBuf> {
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
        "state" => Ok(("/v1/state", json!({ "agentId": agent(1, "state")? }))),
        "ask" => {
            let agent = agent(1, "ask")?;
            let prompt = pos.get(2).ok_or("ask: 보낼 프롬프트가 필요합니다")?;
            let mut o = Map::new();
            o.insert("agentId".into(), Value::String(agent));
            o.insert("prompt".into(), Value::String(prompt.clone()));
            o.insert("timeoutSecs".into(), json!(timeout_secs(p, 600)?));
            if let Some(cli) = p.kv.get("cli") {
                o.insert("cli".into(), Value::String(cli.clone()));
            }
            Ok(("/v1/ask", Value::Object(o)))
        }
        "autorun" => {
            let op = pos.get(1).map(String::as_str).ok_or("autorun: status|start|resume|pause|end|escalate 하위 명령이 필요합니다")?;
            let root = pos.get(2).cloned().ok_or_else(|| format!("autorun {op}: 루트 태스크 id가 필요합니다"))?;
            let path = match op {
                "status" => {
                    let mut o = Map::new();
                    o.insert("rootId".into(), Value::String(root));
                    if let Some(since) = p.kv.get("since") {
                        o.insert("since".into(), Value::String(since.clone()));
                    }
                    if p.wait {
                        o.insert("waitSecs".into(), json!(timeout_secs(p, DEFAULT_WAIT_SECS)?.min(super::autorun::MAX_WAIT_SECS)));
                    }
                    return Ok(("/v1/autorun/status", Value::Object(o)));
                }
                "start" => "/v1/autorun/start",
                "resume" => "/v1/autorun/resume",
                "pause" => "/v1/autorun/pause",
                "end" => "/v1/autorun/end",
                "escalate" => {
                    let message = pos.get(3).ok_or("autorun escalate: 사람에게 전할 메시지가 필요합니다")?;
                    return Ok(("/v1/autorun/escalate", json!({ "rootId": root, "message": message })));
                }
                other => return Err(format!("autorun: 알 수 없는 하위 명령: {other}")),
            };
            let mut body = json!({ "rootId": root });
            if op == "start" && p.no_usage_guard {
                body["usageGuard"] = json!(false);
            }
            Ok((path, body))
        }
        "" => Err("명령이 필요합니다 (help 참고)".into()),
        other => Err(format!("알 수 없는 명령: {other} (help 참고)")),
    }
}

fn timeout_secs(p: &Parsed, default: u64) -> Result<u64, String> {
    match p.kv.get("timeout") {
        None => Ok(default),
        Some(v) => v.parse::<u64>().ok().filter(|n| *n > 0).ok_or_else(|| format!("--timeout은 1 이상의 초여야 합니다: {v}")),
    }
}

/// 성공 응답의 종료 코드. 기다리는 명령만 결과에 따라 나뉜다(셸이 JSON 없이 분기하게).
fn success_exit(parsed: &Parsed, data: &Value) -> i32 {
    let waited = match parsed.sub() {
        "ask" => true,
        "autorun" => parsed.wait && parsed.positionals.get(1).map(String::as_str) == Some("status"),
        _ => false,
    };
    if !waited {
        return EXIT_OK;
    }
    if data.get("timedOut").and_then(Value::as_bool) == Some(true) {
        return EXIT_TIMEOUT;
    }
    match data.get("state").and_then(Value::as_str) {
        Some("finished" | "idle") => EXIT_OK,
        _ => EXIT_ATTENTION,
    }
}

/// 요청 하나가 서버에서 머무를 수 있는 시간 + 여유.
fn request_timeout(path: &str, body: &Value) -> Duration {
    let held = match path {
        "/v1/ask" => body["timeoutSecs"].as_u64().unwrap_or(600),
        "/v1/autorun/status" => body["waitSecs"].as_u64().unwrap_or(0),
        _ => 0,
    };
    Duration::from_secs(10 + held)
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
  send <agentId> <text> [--enter] 세션 stdin에 text를 주입(--enter=개행 추가)
  dispose <agentId>               세션을 종료한다
  notifications <agentId>         대기 중 알림을 나열한다
  clear <agentId> [id...]         알림을 지운다(id 없으면 전체)
  settings get                    현재 앱 설정을 출력한다
  settings set <key=value>...     설정을 변경한다(cliEnabled·cliAutoRunEnabled 제외)
  state <agentId>                 턴 상태: working | needs-human | idle | ended | none

플랜 위임(앱 설정 'CLI 자동 진행 허용' 필요, status·escalate는 제외):
  autorun start <rootId> [--no-usage-guard]
                                  플랜 루트의 자동 진행을 시작한다(GUI ▶와 같다)
  autorun status <rootId> [--wait] [--timeout S] [--since REV]
                                  상태·단계별 보고·멈춘 이유(attention.actions)를 준다.
                                  --wait: 끝나거나 조치가 필요할 때까지(기본 600초),
                                  --since: 그 rev에서 무엇이든 바뀔 때까지 기다린다
  autorun resume|pause|end <rootId>
                                  재개(같은 단계에서 2회까지) / 멈춤 / 진행 기록 지우기
  autorun escalate <rootId> <메시지>
                                  멈추고 사람에게 넘긴다(OS 알림)
  ask <agentId> <프롬프트> [--timeout S] [--cli claude|codex]
                                  실행 중인 세션에 입력하고 턴이 끝나면 마지막 응답을 준다

종료 코드(기다리는 명령): 0 끝남, 10 조치 필요(멈춤·승인 대기), 11 시간 초과.

전역 옵션:
  --json                          응답 data를 JSON으로 출력(기계 파싱용)
  --app-data <경로>               app_data 위치 지정(자동발견 대체)
  --port <포트> / --token <토큰>  포트/토큰 직접 지정
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

    // `status --wait`는 서버가 한 번에 50초까지만 붙잡으므로 남은 시간만큼 다시 묻는다.
    let deadline = std::time::Instant::now() + Duration::from_secs(timeout_secs(&parsed, DEFAULT_WAIT_SECS).unwrap_or(DEFAULT_WAIT_SECS));
    let mut body = body;
    let result = loop {
        let result = runtime.block_on(send(port, &token, path, &body));
        let again = matches!(&result, Ok((200, v)) if path == "/v1/autorun/status"
            && v["data"]["timedOut"].as_bool() == Some(true));
        let left = deadline.saturating_duration_since(std::time::Instant::now()).as_secs();
        if !again || left == 0 {
            break result;
        }
        body["waitSecs"] = json!(left.min(super::autorun::MAX_WAIT_SECS));
    };
    match result {
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
                success_exit(&parsed, data)
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
    let timeout = request_timeout(path, body);
    let client = reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(timeout)
        .build()
        .map_err(|e| format!("HTTP 클라이언트 생성 실패: {e}"))?;
    let resp = client
        .post(format!("http://127.0.0.1:{port}{path}"))
        .header(TOKEN_HEADER, token)
        .json(body)
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

/// 성공 응답의 stdout 표현을 그대로 만든다(개행 포함). 출력을 문자열로
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
        "settings" => {
            let _ = writeln!(
                out,
                "{}",
                serde_json::to_string_pretty(data).unwrap_or_else(|_| data.to_string())
            );
        }
        "state" => {
            let _ = writeln!(
                out,
                "{} {}{}",
                data["agentId"].as_str().unwrap_or("?"),
                data["state"].as_str().unwrap_or("?"),
                data["message"].as_str().map(|m| format!(" — {m}")).unwrap_or_default()
            );
        }
        "ask" => match data["reply"].as_str() {
            Some(reply) => {
                let _ = writeln!(out, "{reply}");
            }
            None => {
                let _ = writeln!(
                    out,
                    "({}{})",
                    data["state"].as_str().unwrap_or("?"),
                    data["message"].as_str().map(|m| format!(": {m}")).unwrap_or_default()
                );
            }
        },
        "autorun" => render_autorun(&mut out, data),
        _ => out.push_str("ok\n"),
    }
    out
}

fn render_autorun(out: &mut String, data: &Value) {
    use std::fmt::Write as _;
    if data.is_null() {
        out.push_str("ok\n");
        return;
    }
    let _ = writeln!(
        out,
        "{} [{}] {}",
        data["rootId"].as_str().unwrap_or("?"),
        data["state"].as_str().unwrap_or("?"),
        data["title"].as_str().unwrap_or("")
    );
    if let Some(a) = data.get("attention") {
        let actions: Vec<&str> = a["actions"].as_array().map(|v| v.iter().filter_map(Value::as_str).collect()).unwrap_or_default();
        let _ = writeln!(
            out,
            "  attention: {}{} → {}",
            a["reason"].as_str().unwrap_or("?"),
            a["detail"].as_str().map(|d| format!(" ({d})")).unwrap_or_default(),
            actions.join(", ")
        );
    }
    if let Some(steps) = data["steps"].as_array() {
        let current = data["current"]["taskId"].as_str();
        for s in steps {
            let id = s["id"].as_str().unwrap_or("?");
            let _ = writeln!(
                out,
                "  {} {:<11} {} {}",
                if Some(id) == current { "▶" } else { "-" },
                s["status"].as_str().unwrap_or("?"),
                id,
                s["title"].as_str().unwrap_or("")
            );
        }
    }
    if let Some(rev) = data["rev"].as_str() {
        let _ = writeln!(out, "  rev: {rev}");
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
                "keepAwakeEnabled=true",
                "soundVolume=0.25",
                "attentionHoldMs=3000",
                "summaryProvider=codex",
            ]))
            .unwrap(),
        )
        .unwrap();
        assert_eq!(path, "/v1/settings/set");
        assert_eq!(body["typingSoundEnabled"], false);
        assert_eq!(body["keepAwakeEnabled"], true);
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
    fn build_autorun_and_ask_requests() {
        let (path, body) = build_request(&parse(&args(&["autorun", "status", "t-1", "--wait", "--timeout", "120", "--since", "abc"])).unwrap()).unwrap();
        assert_eq!(path, "/v1/autorun/status");
        assert_eq!(body, json!({ "rootId": "t-1", "since": "abc", "waitSecs": 50 }));
        let (_, plain) = build_request(&parse(&args(&["autorun", "status", "t-1"])).unwrap()).unwrap();
        assert!(plain.get("waitSecs").is_none());

        let (path, body) = build_request(&parse(&args(&["autorun", "start", "t-1", "--no-usage-guard"])).unwrap()).unwrap();
        assert_eq!((path, body), ("/v1/autorun/start", json!({ "rootId": "t-1", "usageGuard": false })));
        let (path, body) = build_request(&parse(&args(&["autorun", "escalate", "t-1", "DB 마이그레이션 확인 필요"])).unwrap()).unwrap();
        assert_eq!(path, "/v1/autorun/escalate");
        assert_eq!(body["message"], "DB 마이그레이션 확인 필요");
        assert!(build_request(&parse(&args(&["autorun", "escalate", "t-1"])).unwrap()).is_err());
        assert!(build_request(&parse(&args(&["autorun", "start"])).unwrap()).is_err());
        assert!(build_request(&parse(&args(&["autorun", "launch", "t-1"])).unwrap()).is_err());

        let (path, body) = build_request(&parse(&args(&["ask", "a1", "테스트 돌려 줘", "--timeout", "30", "--cli", "codex"])).unwrap()).unwrap();
        assert_eq!(path, "/v1/ask");
        assert_eq!(body, json!({ "agentId": "a1", "prompt": "테스트 돌려 줘", "timeoutSecs": 30, "cli": "codex" }));
        assert!(build_request(&parse(&args(&["ask", "a1", "x", "--timeout", "0"])).unwrap()).is_err());
        assert_eq!(request_timeout("/v1/ask", &body), Duration::from_secs(40));
    }

    #[test]
    fn waiting_commands_map_outcome_to_exit_code() {
        let wait = parse(&args(&["autorun", "status", "t-1", "--wait"])).unwrap();
        assert_eq!(success_exit(&wait, &json!({ "state": "finished", "timedOut": false })), EXIT_OK);
        assert_eq!(success_exit(&wait, &json!({ "state": "paused", "timedOut": false })), EXIT_ATTENTION);
        assert_eq!(success_exit(&wait, &json!({ "state": "needs-human", "timedOut": false })), EXIT_ATTENTION);
        assert_eq!(success_exit(&wait, &json!({ "state": "running", "timedOut": true })), EXIT_TIMEOUT);
        let peek = parse(&args(&["autorun", "status", "t-1"])).unwrap();
        assert_eq!(success_exit(&peek, &json!({ "state": "paused" })), EXIT_OK);
        let ask = parse(&args(&["ask", "a1", "hi"])).unwrap();
        assert_eq!(success_exit(&ask, &json!({ "state": "idle", "reply": "done" })), EXIT_OK);
        assert_eq!(success_exit(&ask, &json!({ "state": "needs-human" })), EXIT_ATTENTION);
        assert_eq!(success_exit(&ask, &json!({ "state": "working", "timedOut": true })), EXIT_TIMEOUT);
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
}
