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
        "" => Err("명령이 필요합니다 (help 참고)".into()),
        other => Err(format!("알 수 없는 명령: {other} (help 참고)")),
    }
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
  settings set <key=value>...     설정을 변경한다(cliEnabled 제외)

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
                print_success(&parsed, &value["data"]);
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
    let client = reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(10))
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
    if parsed.json {
        println!(
            "{}",
            serde_json::to_string_pretty(data).unwrap_or_else(|_| data.to_string())
        );
        return;
    }
    match parsed.sub() {
        "ping" => println!(
            "connected: agent-office v{} (agents={}, running={})",
            data["appVersion"].as_str().unwrap_or("?"),
            data["agentCount"],
            data["runningCount"]
        ),
        "list" => match data.as_array() {
            Some(rows) if !rows.is_empty() => {
                for r in rows {
                    println!(
                        "{:<16} {:<9} {}",
                        r["agentId"].as_str().unwrap_or("?"),
                        r["state"].as_str().unwrap_or("-"),
                        r["name"].as_str().unwrap_or("")
                    );
                }
            }
            _ => println!("(프로필 없음)"),
        },
        "create" => println!(
            "created: {} ({})",
            data["sessionId"].as_str().unwrap_or("?"),
            data["state"].as_str().unwrap_or("?")
        ),
        "notifications" => match data.as_array() {
            Some(rows) if !rows.is_empty() => {
                for r in rows {
                    println!(
                        "- [{}] {}",
                        r["source"].as_str().unwrap_or("?"),
                        r["message"].as_str().unwrap_or("")
                    );
                }
            }
            _ => println!("(알림 없음)"),
        },
        "settings" => println!(
            "{}",
            serde_json::to_string_pretty(data).unwrap_or_else(|_| data.to_string())
        ),
        _ => println!("ok"),
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
                "soundEnabled=false",
                "soundVolume=0.25",
                "attentionHoldMs=3000",
                "summaryProvider=codex",
            ]))
            .unwrap(),
        )
        .unwrap();
        assert_eq!(path, "/v1/settings/set");
        assert_eq!(body["soundEnabled"], false);
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
