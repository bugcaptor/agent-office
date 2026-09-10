// src-tauri/src/session/kilo_plugin.rs
//
// Kilo Code CLI(`kilo`/`kilocode`, OpenCode 포크 v7.6.0) 세션 상태 감지용
// 플러그인 배포. pi_extension.rs와 같은 결이다 — Kilo는 세션별 설정 파일이
// 필요 없고, 프로세스 env(AGENT_OFFICE_SESSION / AGENT_OFFICE_HOOK_URL)를
// 읽는 정적 플러그인 하나로 충분하다. 다만 주입 방식이 다르다: pi는 `-e
// <경로>`로 확장 파일을 직접 가리키지만, Kilo는 `KILO_CONFIG=<json 경로>`
// env로 설정 파일을 가리키고 그 JSON의 `plugin` 배열(`file://` URL)에 실린
// 플러그인을 로드한다. 두 파일(.ts 플러그인 본체 + .json 설정) 모두 정적
// (세션 무관)이라 pi처럼 부팅/세션 준비 시 blind overwrite한다.
//
// 스파이크 실측(2026-09-10, kilo 7.6.0):
//   - `KILO_CONFIG=<json>` env가 전역 `~/.config/kilo/kilo.jsonc`와 병합되어
//     플러그인이 추가 로드된다(`kilo debug config`로 확인). 전역 설정은
//     건드리지 않는다 — 이 env는 이 세션의 호출 한 번에만 앞자리 대입으로
//     붙는다(session/wrapper_script.rs `set_env_from_env`).
//   - 사용자가 이미 자기 `KILO_CONFIG`를 쓰고 있으면(`skip_if_env_set`) 관찰을
//     포기하고 원본 명령을 그대로 실행한다 — 덮어쓰면 사용자 설정이 깨진다.
//   - 이벤트 페이로드는 docs/kilo-support-design.md의 매핑 표를 따른다.
//     핵심만 다시 적으면: `chat.message`가 프롬프트/도구 하트비트 겸용(자식
//     세션이면 하트비트, 아니면 프롬프트), `tool.execute.before`가 도구
//     시작, `event`(session.created/session.idle/permission.*)가 자식 세션
//     회계·완료·권한 알림을 담당한다. **자식 세션의 idle이 부모 idle보다
//     먼저 온다** — activeChildren을 부모 idle보다 먼저 감소시켜야 한다.

use std::io;
use std::path::{Path, PathBuf};

/// Kilo 프로세스 내에서 로드되는 플러그인 소스. Kilo 패키지에서 타입을
/// import하지 않는다(버전 드리프트 격리) — ctx/event/input/output은 구조적
/// 타이핑(`any`)으로만 쓴다. env(HOOK_URL/SESSION)가 없으면 빈 훅 객체를
/// 돌려주므로 agent-office 밖에서 사용자가 이 플러그인을 로드해도 무해하다.
const KILO_PLUGIN_TS: &str = r#"// agent-office-kilo.ts — agent-office가 생성. 편집 금지(부팅 시 덮어씀).
// Kilo Code CLI 세션 이벤트를 agent-office 로컬 훅 서버로 POST해 작업상태를 알린다.
export const AgentOffice = async (ctx: any) => {
  const session = process.env.AGENT_OFFICE_SESSION;
  let url = process.env.AGENT_OFFICE_HOOK_URL;
  if (!url || !session) return {}; // agent-office 밖: 완전 no-op

  const g = globalThis as any;
  if (g.__AGENT_OFFICE_KILO_HOOKED__) return {}; // 전역 설정과 중복 로드 방어
  g.__AGENT_OFFICE_KILO_HOOKED__ = true;

  // 스테일 포트 재시도(pi_extension.rs와 같은 계약). 옵저버 서버는 매 실행마다
  // 포트 0으로 바인딩하므로 앱을 껐다 켜면 포트가 바뀌는데, 입양된 세션의 env는
  // 스폰 시점 포트를 그대로 들고 있다. 실패하면 재시도만 포기(관찰은 부가
  // 기능이라 Kilo 동작에 영향 없음).
  const fs = (process as any).getBuiltinModule?.("node:fs");
  const appData = process.env.AGENT_OFFICE_APP_DATA;
  const portFileUrl = (): string | undefined => {
    if (!fs || !appData) return undefined;
    try {
      const text = String(fs.readFileSync(`${appData}/observer-port`, "utf8")).trim();
      const port = Number.parseInt(text, 10);
      if (!Number.isInteger(port) || port <= 0 || port > 65535) return undefined;
      const parsed = new URL(url as string);
      // 루프백 평문만 — forwarder의 parse_local_hook_url과 같은 제약.
      if (parsed.protocol !== "http:" || parsed.hostname !== "127.0.0.1") return undefined;
      parsed.port = String(port);
      return parsed.toString();
    } catch {
      return undefined;
    }
  };

  const send = (target: string, source: string, body: unknown) =>
    fetch(`${target}?session=${session}&source=${source}&agent=kilo`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify(body ?? {}),
      signal: AbortSignal.timeout(2000),
    });

  // POST 직렬화 큐: prompt→tool 역전으로 백엔드 at 타임스탬프가 뒤집히는 것 방지.
  let chain: Promise<unknown> = Promise.resolve();
  const post = (source: string, body: unknown) => {
    chain = chain.then(async () => {
      try {
        await send(url as string, source, body);
        return;
      } catch { /* 연결 실패 — 아래에서 최신 포트로 1회 재시도 */ }
      const retry = portFileUrl();
      if (!retry || retry === url) return; // 앱이 내려가 있어도 kilo는 무사
      try {
        await send(retry, source, body);
        url = retry; // 이후 POST는 곧장 새 포트로
      } catch { /* 그래도 안 되면 조용히 포기 */ }
    });
  };

  // 자식 세션(task 도구가 만든 서브에이전트) 회계. 부모보다 자식 idle이 먼저
  // 오므로, activeChildren을 자식 idle에서 먼저 줄이고 그 다음 부모 idle에서
  // runOpen을 정산한다.
  const childSessions = new Set<string>();
  let activeChildren = 0;
  let runOpen = false; // 사용자 요청 1건이 진행 중인가(루트 세션 기준)
  // 리뷰 지적: "자식이 아니면 루트"라는 소거법 판정은 오류·중단 턴에서
  // session.idle이 두 번 오는 실측(SessionProcessor.halt + 러너 onIdle이
  // 각각 set(idle))과 만나면 두 번째 idle도 "자식이 아니니 루트"로 오판할
  // 여지가 있다. 루트 세션 ID를 chat.message에서 명시적으로 기록해 두고
  // session.idle에서 그 ID와 정확히 비교한다 — 중복 idle은 runOpen 가드가
  // 흡수한다(두 번째 idle 시점엔 이미 runOpen=false).
  let rootSessionID: string | undefined;
  let lastError = false; // 이번 루트 턴에서 session.error를 봤는가

  return {
    event: async ({ event }: any) => {
      try {
        const type = event?.type;
        const props = event?.properties ?? {};
        if (type === "session.created") {
          const parentID = props?.info?.parentID;
          const sid = props?.sessionID;
          if (parentID && sid) {
            childSessions.add(sid);
            activeChildren += 1;
            post("sub-start", {});
          }
        } else if (type === "session.idle") {
          const sid = props?.sessionID;
          if (sid && childSessions.has(sid)) {
            childSessions.delete(sid);
            activeChildren = Math.max(0, activeChildren - 1);
            post("sub-stop", {});
          } else if (sid && sid === rootSessionID && runOpen) {
            runOpen = false;
            const message = lastError ? "Kilo stopped with an error" : "Kilo finished a task";
            post("stop", { message, running: activeChildren });
          }
          // 그 외(같은 루트 세션의 두 번째 idle, 또는 무관한 세션의 idle)는
          // 무시한다 — runOpen이 이미 꺼져 있어 중복 stop이 나가지 않는다.
        } else if (type === "session.error") {
          const sid = props?.sessionID;
          if (sid && sid === rootSessionID) {
            lastError = true;
          }
        } else if (type === "permission.asked") {
          const permission = props?.permission ?? "";
          const pattern = Array.isArray(props?.patterns) ? props.patterns[0] : undefined;
          const message = pattern
            ? `Kilo needs permission: ${permission} ${pattern}`
            : `Kilo needs permission: ${permission}`;
          post("hook", { message });
        } else if (type === "permission.replied") {
          post("tool", {});
        }
      } catch { /* 핸들러 예외가 Kilo 턴을 깨지 않도록 삼킨다 */ }
    },

    "chat.message": async (input: any, output: any) => {
      try {
        const sid = input?.sessionID;
        const isChildTurn = (sid && childSessions.has(sid)) || Boolean(input?.agent);
        if (isChildTurn) {
          // 자식(서브에이전트) 프롬프트는 완료 판정을 건드리지 않는 하트비트로만 쓴다.
          post("tool", {});
          return;
        }
        rootSessionID = sid;
        // 새 루트 프롬프트 시작: 이전 요청의 자식 세션 회계를 리셋한다.
        // kilo의 task 도구는 부모가 자식을 await하므로, 새 요청이 들어온
        // 시점에 이전 자식이 남아 있을 이유가 없다 — 남아 있다면 이전 턴의
        // sub-stop을 놓친 회계 버그이므로, 여기서 강제로 0에서 다시 센다.
        childSessions.clear();
        activeChildren = 0;
        lastError = false;
        const part = Array.isArray(output?.parts)
          ? output.parts.find((p: any) => p?.type === "text")
          : undefined;
        runOpen = true;
        post("prompt", { prompt: part?.text ?? "", cwd: ctx?.directory ?? process.cwd() });
      } catch { /* 관찰 실패는 삼킨다 */ }
    },

    "tool.execute.before": async (input: any, output: any) => {
      try {
        post("tool", { tool_name: input?.tool ?? "", tool_input: output?.args ?? {} });
      } catch { /* 관찰 실패는 삼킨다 */ }
    },
  };
};

export default AgentOffice;
"#;

const PLUGIN_FILENAME: &str = "agent-office-kilo.ts";
const CONFIG_FILENAME: &str = "agent-office-kilo.json";

/// 플러그인 파일을 담는 디렉터리. pi 확장(`<app_data>/observer/pi`)과 같은
/// 결로 `<app_data>/observer/kilo`를 쓴다.
pub fn plugin_dir(app_data: &Path) -> PathBuf {
    app_data.join("observer").join("kilo")
}

/// `base`(없으면 생성)에 플러그인 본체(.ts)와 그것을 가리키는 설정
/// JSON(`{"plugin": ["file:///…"]}`)을 함께 쓴다. 내용이 정적이라 blind
/// overwrite한다(pi_extension::write_extension과 같은 패턴). 반환값은
/// **설정 JSON 경로**다 — env `KILO_CONFIG`에 실리는 값은 이쪽이다(플러그인
/// 파일 경로 자체가 아니다).
///
/// file URL은 `url`/`reqwest::Url::from_file_path`로 만든다 — macOS app_data
/// 경로에 공백이 섞일 수 있는데, 문자열로 직접 `file://`를 이어 붙이면 그
/// 공백이 %20으로 인코딩되지 않아 Kilo의 URL 파서가 깨진다.
pub fn write_plugin(base: &Path) -> io::Result<PathBuf> {
    std::fs::create_dir_all(base)?;
    let plugin_path = base.join(PLUGIN_FILENAME);
    write_atomic(&plugin_path, KILO_PLUGIN_TS.as_bytes())?;

    let plugin_url = reqwest::Url::from_file_path(&plugin_path).map_err(|_| {
        io::Error::other(format!(
            "kilo plugin path is not a valid absolute file path: {}",
            plugin_path.display(),
        ))
    })?;
    let config = serde_json::json!({ "plugin": [plugin_url.as_str()] });
    let config_path = base.join(CONFIG_FILENAME);
    write_atomic(&config_path, &serde_json::to_vec_pretty(&config)?)?;
    Ok(config_path)
}

/// 같은 디렉터리에 임시 파일로 쓴 뒤 원자적으로 rename한다(`observer/claude.rs`의
/// 훅 설정 temp+rename과 같은 취지). 두 파일 다 세션과 무관하게 부팅·세션
/// 준비마다 blind overwrite되므로, write 도중 프로세스가 죽거나 다른
/// 세션 준비와 겹치면 절반만 쓰인 파일이 그대로 로드될 위험이 있다 — Kilo가
/// 그 시점에 플러그인을 읽으면 파싱 오류로 죽는다. rename은 같은 파일시스템
/// 안에서 원자적이라 이 창을 없앤다. 실패하면 임시 파일을 지운다(잔여물 방지).
///
/// 임시 파일 이름에 uuid를 섞는다 — 고정 이름(`*.tmp`)을 쓰면, app_data가
/// 없는 구성(테스트 등)에서 여러 세션 준비가 동시에 같은 OS temp 경로로
/// 떨어질 때 한쪽이 rename한 직후 다른 쪽이 이미 사라진 같은 임시 파일을
/// rename하려다 ENOENT로 실패하는 레이스가 실제로 발생했다(병렬 테스트에서
/// 재현).
fn write_atomic(path: &Path, contents: &[u8]) -> io::Result<()> {
    let file_name = path.file_name().and_then(|n| n.to_str()).ok_or_else(|| {
        io::Error::other(format!("path has no valid file name: {}", path.display()))
    })?;
    let tmp = path.with_file_name(format!("{file_name}.tmp-{}", uuid::Uuid::new_v4()));
    std::fs::write(&tmp, contents)
        .and_then(|()| std::fs::rename(&tmp, path))
        .map_err(|error| {
            let _ = std::fs::remove_file(&tmp);
            error
        })
}

/// 플러그인·설정 파일을 안정 경로에 써 넣고 설정 JSON 경로(env `KILO_CONFIG`에
/// 실리는 값, `set_env_from_env`가 읽어 갈 `AGENT_OFFICE_KILO_CONFIG`)를
/// 돌려준다. 세션마다 불러도 안전하다 — 내용이 정적이라 매번 같은 파일을
/// 덮어쓴다.
///
/// pi 확장(이슈 #40)과 같은 이유로 **app_data**가 정본 위치다. `app_data`가
/// None이면(테스트·app_data 없는 구성) OS temp로 떨어진다 — 관찰은 되지만
/// 청소에 취약한 예전 동작 그대로다.
pub fn ensure_plugin(app_data: Option<&Path>) -> io::Result<PathBuf> {
    let base = match app_data {
        Some(dir) => plugin_dir(dir),
        None => std::env::temp_dir().join("agent-office").join("kilo"),
    };
    write_plugin(&base)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch_dir() -> PathBuf {
        std::env::temp_dir().join(format!("agent-office-kilo-plugin-test-{}", uuid::Uuid::new_v4()))
    }

    #[test]
    fn write_plugin_creates_both_files_and_returns_the_config_path() {
        let base = scratch_dir();
        let config_path = write_plugin(&base).expect("write_plugin succeeds");

        assert_eq!(config_path, base.join(CONFIG_FILENAME));
        assert!(config_path.is_file(), "config file must exist");
        let plugin_path = base.join(PLUGIN_FILENAME);
        assert!(plugin_path.is_file(), "plugin file must exist");

        let ts_contents = std::fs::read_to_string(&plugin_path).unwrap();
        assert_eq!(ts_contents, KILO_PLUGIN_TS, "plugin file must contain the embedded source verbatim");

        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn write_plugin_config_points_at_the_plugin_file_via_file_url() {
        let base = scratch_dir();
        let config_path = write_plugin(&base).expect("write_plugin succeeds");
        let config: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&config_path).unwrap()).unwrap();
        let plugins = config.get("plugin").and_then(|v| v.as_array()).expect("plugin array");
        assert_eq!(plugins.len(), 1);
        let url = plugins[0].as_str().unwrap();
        assert!(url.starts_with("file://"), "{url}");
        assert!(url.ends_with(PLUGIN_FILENAME), "{url}");

        let _ = std::fs::remove_dir_all(&base);
    }

    /// macOS app_data 경로의 공백이 file URL에서 %20으로 인코딩되는지 확인한다
    /// (문자열로 직접 `file://`를 이어 붙이면 이게 깨져 Kilo의 URL 파서가 실패한다).
    #[test]
    fn write_plugin_percent_encodes_spaces_in_the_file_url() {
        let base = std::env::temp_dir().join(format!(
            "agent office kilo plugin test {}",
            uuid::Uuid::new_v4(),
        ));
        let config_path = write_plugin(&base).expect("write_plugin succeeds even with spaces");
        let config: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&config_path).unwrap()).unwrap();
        let url = config["plugin"][0].as_str().unwrap();
        assert!(url.contains("%20"), "{url}");
        assert!(!url.contains(' '), "raw space must not survive encoding: {url}");

        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn ensure_plugin_writes_under_app_data_when_given_one() {
        let app_data = scratch_dir();
        let config_path = ensure_plugin(Some(&app_data)).expect("ensure_plugin succeeds");

        assert_eq!(
            config_path,
            app_data.join("observer").join("kilo").join(CONFIG_FILENAME),
        );
        assert!(config_path.is_file());
        assert_eq!(plugin_dir(&app_data), config_path.parent().unwrap());

        let _ = std::fs::remove_dir_all(&app_data);
    }

    #[test]
    fn ensure_plugin_falls_back_to_os_temp_without_app_data() {
        let config_path = ensure_plugin(None).expect("temp fallback succeeds");

        assert_eq!(
            config_path,
            std::env::temp_dir()
                .join("agent-office")
                .join("kilo")
                .join(CONFIG_FILENAME),
        );
        assert!(config_path.is_file());
    }

    #[test]
    fn write_plugin_is_idempotent_and_overwrites_cleanly() {
        let base = scratch_dir();
        write_plugin(&base).unwrap();
        let config_path = write_plugin(&base).expect("2nd write must not error");
        assert!(config_path.is_file());
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn plugin_source_guards_env_and_double_load_and_tags_agent() {
        assert!(KILO_PLUGIN_TS.contains("AGENT_OFFICE_HOOK_URL"));
        assert!(KILO_PLUGIN_TS.contains("AGENT_OFFICE_SESSION"));
        assert!(KILO_PLUGIN_TS.contains("__AGENT_OFFICE_KILO_HOOKED__"));
        assert!(KILO_PLUGIN_TS.contains("&agent=kilo"));
        assert!(KILO_PLUGIN_TS.contains("export const AgentOffice"));
        assert!(KILO_PLUGIN_TS.contains("export default AgentOffice"));
    }

    #[test]
    fn plugin_source_subscribes_to_the_confirmed_events() {
        for needle in [
            "\"chat.message\"",
            "\"tool.execute.before\"",
            "event: async",
            "session.created",
            "session.idle",
            "session.error",
            "permission.asked",
            "permission.replied",
        ] {
            assert!(KILO_PLUGIN_TS.contains(needle), "plugin must reference `{needle}`");
        }
    }

    /// 리뷰 지적 회귀 방지: 루트 세션 판정은 "자식이 아니면 루트"라는
    /// 소거법이 아니라 `chat.message` 루트 분기에서 기록한 `rootSessionID`와
    /// `session.idle`의 `sessionID`를 명시적으로 비교해야 한다. 오류·중단
    /// 턴에서 session.idle이 두 번 오는 실측(SessionProcessor.halt + 러너
    /// onIdle) 때문에 소거법으로는 중복 idle을 다른 세션 이벤트로 오판할
    /// 여지가 있었다.
    #[test]
    fn plugin_source_matches_session_idle_against_an_explicit_root_session_id() {
        assert!(KILO_PLUGIN_TS.contains("let rootSessionID"));
        assert!(KILO_PLUGIN_TS.contains("rootSessionID = sid"));
        assert!(
            KILO_PLUGIN_TS.contains("sid === rootSessionID && runOpen"),
            "session.idle must compare against the recorded root session id, not fall through by elimination",
        );
    }

    /// 리뷰 지적: kilo의 task 도구는 부모가 자식을 await하므로, 새 루트
    /// 프롬프트가 시작되는 시점에 이전 요청의 자식 세션 회계가 남아 있을
    /// 이유가 없다. `chat.message` 루트 분기에서 리셋해야 한다.
    #[test]
    fn plugin_source_resets_child_accounting_and_error_flag_on_a_new_root_prompt() {
        assert!(KILO_PLUGIN_TS.contains("childSessions.clear()"));
        assert!(KILO_PLUGIN_TS.contains("activeChildren = 0"));
        assert!(KILO_PLUGIN_TS.contains("lastError = false"));
    }

    /// 리뷰 지적: 오류·중단으로 끝난 턴은 "Kilo finished a task"가 아니라
    /// "Kilo stopped with an error"로 알려야 한다. `session.error`를 보고
    /// `lastError`를 세우고, stop 메시지가 그걸 반영해야 한다.
    #[test]
    fn plugin_source_reports_a_distinct_stop_message_when_the_root_turn_errored() {
        assert!(KILO_PLUGIN_TS.contains("lastError = true"));
        assert!(KILO_PLUGIN_TS.contains("Kilo stopped with an error"));
        assert!(
            KILO_PLUGIN_TS.contains(
                "const message = lastError ? \"Kilo stopped with an error\" : \"Kilo finished a task\";"
            ),
            "stop message must branch on lastError",
        );
    }

    /// 완료 판정 회귀 방지: 자식 세션의 idle이 부모보다 먼저 온다는 실측을
    /// 반영해, 자식 idle에서 activeChildren을 먼저 줄이고(0 미만 금지) sub-stop을
    /// 내며, 루트 idle에서만 runOpen을 정산해 stop을 낸다.
    #[test]
    fn plugin_source_accounts_child_sessions_before_settling_the_root_turn() {
        assert!(KILO_PLUGIN_TS.contains("childSessions.add(sid)"));
        assert!(KILO_PLUGIN_TS.contains("Math.max(0, activeChildren - 1)"));
        assert!(KILO_PLUGIN_TS.contains("post(\"sub-start\""));
        assert!(KILO_PLUGIN_TS.contains("post(\"sub-stop\""));
        assert!(
            KILO_PLUGIN_TS.contains("post(\"stop\", { message, running: activeChildren });"),
            "stop must report the still-running child count",
        );
    }

    /// 스테일 포트 재시도(pi_extension.rs와 같은 계약).
    #[test]
    fn plugin_source_retries_on_the_observer_port_file() {
        assert!(KILO_PLUGIN_TS.contains("AGENT_OFFICE_APP_DATA"));
        assert!(KILO_PLUGIN_TS.contains("observer-port"));
        assert!(KILO_PLUGIN_TS.contains("url = retry"));
        assert!(KILO_PLUGIN_TS.contains("127.0.0.1"));
    }
}
