// src-tauri/src/session/agy_hook.rs
//
// Antigravity CLI(agy) 훅 셸 스크립트 정적 배포.
// docs/antigravity-support-design.md §3.1 그대로 구현하되, 스파이크 실측으로
// 확정된 사실 셋을 반영한다:
//
//   1. `invocationNum`은 0부터 센다(턴 시작 = PreInvocation && invocationNum
//      == 0). 이 필터링은 셸에서 JSON을 파싱하지 않고 **서버**
//      (`observer::event::agy_invocation_num` / `ObserverRuntime::
//      ingest_agy_source`)가 한다 — 이 스크립트는 이벤트 이름 → source
//      매핑만 한다.
//   2. 훅 실패/부재에도 agy는 정상 종료한다(S2 실측) — 그래도 매 호출 첫
//      줄에 무해 출력(`{}`)을 먼저 찍는다. Stop의 `{"decision":""}`도
//      `{}`와 동등하게 취급되므로 세 이벤트 모두 같은 출력으로 통일한다.
//   3. cwd는 쿼리 문자열이 아니라 **헤더**(`X-Agent-Office-Cwd`)로 얹는다.
//      POSIX sh에는 표준 percent-encoding 도구가 없어 쿼리 문자열에 실으면
//      공백·비ASCII 경로에서 깨진다 — curl -H는 인코딩이 필요 없다.
//      서버 쪽은 `observer::server::handle_hook`이 이 헤더를 읽는다.
//   4. 스테일 포트 재시도(리뷰 지적, forwarder.rs/pi_extension.rs와 같은
//      계약): 옵저버 서버는 매 실행 포트 0으로 바인딩하므로 앱을 껐다 켜면
//      포트가 바뀌는데, 입양된 세션의 env(`AGENT_OFFICE_HOOK_URL`)는 스폰
//      시점 포트를 그대로 들고 있다. curl이 "연결 실패"(exit 7)로 끝나면
//      `AGENT_OFFICE_APP_DATA/observer-port`를 읽어(정수 검증 후) 그 포트로
//      1회만 재시도한다. `AGENT_OFFICE_APP_DATA`는 session/manager.rs가
//      세션 env에 항상 심어 두므로(관찰 여부와 무관, §핵심 5) 래퍼가 따로
//      넘길 필요는 없다 — hook.sh는 agy의 자식으로서 이 env를 그대로
//      물려받는다.
//
// pi 확장(session/pi_extension.rs)과 같은 배포 방식: 세션마다 쓰지 않고
// app_data 안정 경로에 정적 파일 하나를 blind overwrite한다(내용이
// 고정이라 안전). agy는 세션별로 훅 파일을 넘길 방법이 없어 이 스크립트의
// **절대 경로**를 전역 `~/.gemini/config/hooks.json`에 박아 둔다
// (agy_hooks_file.rs).

use std::io;
use std::path::{Path, PathBuf};

/// hooks.json에 박히는 훅 명령이 부르는 POSIX sh 스크립트.
/// `AGENT_OFFICE_AGY_EVENT`(hooks.json 명령이 앞에 붙이는 env)로 이벤트를
/// 구분하고, `AGENT_OFFICE_HOOK_URL`/`AGENT_OFFICE_SESSION`이 없으면(=
/// agent-office 밖에서 agy를 실행 중) 즉시 종료한다.
const AGY_HOOK_SH: &str = r#"#!/bin/sh
# agent-office-agy-hook.sh — agent-office가 생성. 편집 금지(부팅 시 덮어씀).
# Antigravity CLI(agy) 훅 이벤트를 agent-office 로컬 훅 서버로 전달한다.
#
# 실측(docs/antigravity-support-design.md §1.4): PreToolUse의 decision은
# 실행을 게이트하고 Stop의 decision=="continue"는 루프를 멈추지 못하게
# 만든다 — 그래서 관찰 대상 세 이벤트(PreInvocation/PostToolUse/Stop) 모두
# 무해한 `{}`를 stdin을 읽기도 전에 먼저 찍는다.
printf '{}'

# agent-office 밖(env 없음)에서는 stdin을 마저 읽지 않고 바로 종료 — 완전 no-op.
if [ -z "$AGENT_OFFICE_HOOK_URL" ] || [ -z "$AGENT_OFFICE_SESSION" ]; then
  exit 0
fi

case "$AGENT_OFFICE_AGY_EVENT" in
  PreInvocation) source=prompt ;;
  PostToolUse) source=tool ;;
  Stop) source=stop ;;
  *) exit 0 ;;
esac

# stdin을 임시 파일에 담는다 -- 큰 body를 셸 변수 하나에 담아 curl 인자로
# 넘기면 ARG_MAX 근처에서 위험하니, 파일로 받아 @파일 스트리밍으로 넘긴다.
# 재시도 시 stdin을 다시 읽을 수 없으므로 파일로 남겨 둬야 한다.
tmp="$(mktemp 2>/dev/null)" || tmp="/tmp/agent-office-agy-hook-$$"
cat > "$tmp" 2>/dev/null

post_to() {
  if [ -n "$AGENT_OFFICE_AGY_CWD" ]; then
    curl -s -o /dev/null --connect-timeout 0.5 --max-time 1.5 \
      -X POST -H 'Content-Type: application/json' \
      -H "X-Agent-Office-Cwd: $AGENT_OFFICE_AGY_CWD" \
      --data-binary @"$tmp" "$1" >/dev/null 2>&1
  else
    curl -s -o /dev/null --connect-timeout 0.5 --max-time 1.5 \
      -X POST -H 'Content-Type: application/json' \
      --data-binary @"$tmp" "$1" >/dev/null 2>&1
  fi
}

url="${AGENT_OFFICE_HOOK_URL}?session=${AGENT_OFFICE_SESSION}&agent=agy&source=${source}"
post_to "$url"
rc=$?

# 스테일 포트 재시도(위 모듈 코멘트 4). curl 종료 코드 7은 "연결 실패"만
# 가리킨다 -- 4xx/5xx 같은 실제 응답은 재시도 대상이 아니다(forwarder.rs와
# 같은 구분).
if [ "$rc" -eq 7 ] && [ -n "$AGENT_OFFICE_APP_DATA" ] && [ -f "$AGENT_OFFICE_APP_DATA/observer-port" ]; then
  port="$(cat "$AGENT_OFFICE_APP_DATA/observer-port" 2>/dev/null)"
  case "$port" in
    ''|*[!0-9]*) : ;; # 빈 값/정수가 아니면 재시도하지 않는다
    *) post_to "http://127.0.0.1:${port}/hook?session=${AGENT_OFFICE_SESSION}&agent=agy&source=${source}" ;;
  esac
fi

rm -f "$tmp"
exit 0
"#;

const HOOK_FILENAME: &str = "hook.sh";

/// 훅 스크립트가 담기는 디렉터리. pi 확장과 같은 결로
/// `<app_data>/observer/agy`를 쓴다.
pub fn hook_dir(app_data: &Path) -> PathBuf {
    app_data.join("observer").join("agy")
}

/// `base`(없으면 생성)에 훅 스크립트를 blind overwrite한다 — 내용이 정적이라
/// 매번 다시 써도 안전하다(pi_extension::write_extension과 같은 패턴).
/// 반환값은 스크립트 **파일** 경로(hooks.json 명령에 박히는 절대 경로).
pub fn write_hook_script(base: &Path) -> io::Result<PathBuf> {
    std::fs::create_dir_all(base)?;
    let p = base.join(HOOK_FILENAME);
    std::fs::write(&p, AGY_HOOK_SH)?;
    Ok(p)
}

/// 앱 부팅 시 호출하는 상위 엔트리. `app_data`가 없으면(테스트 등) OS temp로
/// 떨어진다 — pi_extension::ensure_extension과 같은 폴백.
pub fn ensure_hook_script(app_data: Option<&Path>) -> io::Result<PathBuf> {
    let base = match app_data {
        Some(dir) => hook_dir(dir),
        None => std::env::temp_dir().join("agent-office").join("agy"),
    };
    write_hook_script(&base)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch_dir() -> PathBuf {
        std::env::temp_dir().join(format!("agent-office-agy-hook-test-{}", uuid::Uuid::new_v4()))
    }

    #[test]
    fn write_hook_script_creates_the_file_and_returns_its_path() {
        let base = scratch_dir();
        let p = write_hook_script(&base).expect("write_hook_script succeeds");

        assert_eq!(p, base.join("hook.sh"));
        assert!(p.is_file());
        let contents = std::fs::read_to_string(&p).unwrap();
        assert_eq!(contents, AGY_HOOK_SH);

        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn ensure_hook_script_writes_under_app_data_when_given_one() {
        let app_data = scratch_dir();
        let p = ensure_hook_script(Some(&app_data)).expect("ensure_hook_script succeeds");

        assert_eq!(p, app_data.join("observer").join("agy").join(HOOK_FILENAME));
        assert!(p.is_file());
        assert_eq!(hook_dir(&app_data), p.parent().unwrap());

        let _ = std::fs::remove_dir_all(&app_data);
    }

    #[test]
    fn ensure_hook_script_falls_back_to_os_temp_without_app_data() {
        let p = ensure_hook_script(None).expect("temp fallback succeeds");
        assert_eq!(
            p,
            std::env::temp_dir()
                .join("agent-office")
                .join("agy")
                .join(HOOK_FILENAME),
        );
        assert!(p.is_file());
    }

    #[test]
    fn write_hook_script_is_idempotent_and_overwrites_cleanly() {
        let base = scratch_dir();
        write_hook_script(&base).unwrap();
        let p = write_hook_script(&base).expect("2nd write must not error");
        assert!(p.is_file());
        let _ = std::fs::remove_dir_all(&base);
    }

    /// S2 실측 회귀 방지: env가 없으면 stdin을 읽지도 않고 즉시 종료해야
    /// 한다 — agy가 stdin을 닫지 않고 기다리는 훅에 물려 있어도 무해해야
    /// 하므로, env 체크가 stdin을 임시 파일에 담는 `cat > "$tmp"`보다
    /// 앞에 와야 한다.
    #[test]
    fn hook_script_checks_env_before_reading_stdin() {
        let env_check = AGY_HOOK_SH.find("AGENT_OFFICE_HOOK_URL").unwrap();
        let stdin_read = AGY_HOOK_SH.find(r#"cat > "$tmp""#).unwrap();
        assert!(env_check < stdin_read, "env guard must precede stdin read");
    }

    /// 리뷰 지적: `--data-binary "$body"`(셸 변수)는 ARG_MAX 근처 큰 전사에서
    /// 위험하다 -- stdin을 임시 파일에 담아 `--data-binary @파일`로 스트리밍
    /// 해야 한다.
    #[test]
    fn hook_script_streams_stdin_via_a_temp_file_instead_of_a_shell_variable() {
        assert!(AGY_HOOK_SH.contains("mktemp"));
        assert!(AGY_HOOK_SH.contains(r#"cat > "$tmp""#));
        assert!(AGY_HOOK_SH.contains(r#"--data-binary @"$tmp""#));
        assert!(!AGY_HOOK_SH.contains("--data-binary \"$body\""));
        assert!(AGY_HOOK_SH.contains(r#"rm -f "$tmp""#));
    }

    /// 리뷰 지적(forwarder.rs/pi_extension.rs와 같은 계약): 스테일 포트
    /// 재시도가 curl 종료 코드 7(연결 실패)에서만 걸리고, 포트 값이 정수인지
    /// 검증한 뒤에만 재시도해야 한다.
    #[test]
    fn hook_script_retries_on_connect_failure_with_an_integer_validated_port() {
        assert!(AGY_HOOK_SH.contains("AGENT_OFFICE_APP_DATA"));
        assert!(AGY_HOOK_SH.contains("observer-port"));
        assert!(AGY_HOOK_SH.contains(r#"[ "$rc" -eq 7 ]"#));
        assert!(AGY_HOOK_SH.contains("*[!0-9]*"));
        // 재시도 URL은 새 포트로 http://127.0.0.1:<port>/hook을 다시 만든다.
        assert!(AGY_HOOK_SH.contains("http://127.0.0.1:${port}/hook"));
    }

    /// 1.4 실측 회귀 방지: 세 이벤트 모두 무해한 출력을 먼저 찍는다.
    #[test]
    fn hook_script_prints_harmless_output_first() {
        let printf_pos = AGY_HOOK_SH.find("printf '{}'").expect("prints {}");
        let env_check = AGY_HOOK_SH.find("AGENT_OFFICE_HOOK_URL").unwrap();
        assert!(printf_pos < env_check, "harmless output must print before any guard/exit");
    }

    #[test]
    fn hook_script_maps_the_three_observed_events() {
        for (event, source) in [
            ("PreInvocation", "source=prompt"),
            ("PostToolUse", "source=tool"),
            ("Stop", "source=stop"),
        ] {
            assert!(AGY_HOOK_SH.contains(event), "must branch on {event}");
            assert!(AGY_HOOK_SH.contains(source), "must map {event} to {source}");
        }
    }

    #[test]
    fn hook_script_tags_the_agy_agent_and_carries_cwd_as_a_header() {
        assert!(AGY_HOOK_SH.contains("agent=agy"));
        // 쿼리 문자열이 아니라 헤더로 cwd를 얹는다(위 모듈 코멘트 근거).
        assert!(AGY_HOOK_SH.contains("X-Agent-Office-Cwd"));
        assert!(!AGY_HOOK_SH.contains("&cwd="));
    }

    #[test]
    fn hook_script_posts_with_a_short_timeout_and_always_exits_zero() {
        assert!(AGY_HOOK_SH.contains("--connect-timeout 0.5"));
        assert!(AGY_HOOK_SH.contains("--max-time 1.5"));
        assert!(AGY_HOOK_SH.trim_end().ends_with("exit 0"));
    }

    /// 리뷰 지적 회귀 방지(실제 실행): 스테일 포트(연결 실패, curl exit 7)를
    /// 흉내 낸 가짜 `curl`을 PATH에 놓고 실제 `sh`로 hook.sh를 돌려, 새 포트로
    /// 정확히 1회 재시도하는지 끝까지 검증한다. Windows는 v1 제외(§5)라 unix
    /// 전용.
    #[cfg(unix)]
    #[test]
    fn real_sh_retries_exactly_once_against_the_observer_port_file_on_connect_failure() {
        use std::os::unix::fs::PermissionsExt;

        if !std::path::Path::new("/bin/sh").exists() {
            eprintln!("skipping: /bin/sh not present on this host");
            return;
        }

        let scratch = scratch_dir();
        let bin_dir = scratch.join("bin");
        let app_data = scratch.join("app_data");
        std::fs::create_dir_all(&bin_dir).unwrap();
        std::fs::create_dir_all(&app_data).unwrap();

        // 스폰 시점 포트(9)는 죽어 있고, 현재 포트(23456)만 기록돼 있다 -- §핵심 5와
        // 같은 시나리오.
        std::fs::write(app_data.join("observer-port"), "23456").unwrap();

        let log = scratch.join("curl-calls.log");
        // 가짜 curl: 마지막 인자(URL)를 로그에 남기고, 포트 9면 연결 실패(exit 7),
        // 포트 23456이면 성공(exit 0)으로 흉내 낸다.
        let fake_curl = bin_dir.join("curl");
        std::fs::write(
            &fake_curl,
            r#"#!/bin/sh
for last; do :; done
echo "$last" >> "$AGENT_OFFICE_TEST_LOG"
case "$last" in
  *:9/*) exit 7 ;;
  *) exit 0 ;;
esac
"#,
        )
        .unwrap();
        std::fs::set_permissions(&fake_curl, std::fs::Permissions::from_mode(0o755)).unwrap();

        let hook_path = write_hook_script(&scratch).unwrap();
        let output = std::process::Command::new("/bin/sh")
            .arg(&hook_path)
            .env_clear()
            .env("PATH", format!("{}:/bin:/usr/bin", bin_dir.display()))
            .env("AGENT_OFFICE_HOOK_URL", "http://127.0.0.1:9/hook")
            .env("AGENT_OFFICE_SESSION", "s1")
            .env("AGENT_OFFICE_AGY_EVENT", "Stop")
            .env("AGENT_OFFICE_APP_DATA", &app_data)
            .env("AGENT_OFFICE_TEST_LOG", &log)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .spawn()
            .and_then(|mut child| {
                use std::io::Write as _;
                child
                    .stdin
                    .take()
                    .unwrap()
                    .write_all(br#"{"terminationReason":"model_stop"}"#)?;
                child.wait_with_output()
            })
            .expect("spawn /bin/sh hook.sh");

        assert!(output.status.success(), "hook.sh must always exit 0");
        assert_eq!(
            String::from_utf8_lossy(&output.stdout),
            "{}",
            "must print the harmless output first",
        );

        let calls = std::fs::read_to_string(&log).unwrap_or_default();
        let calls: Vec<&str> = calls.lines().collect();
        assert_eq!(calls.len(), 2, "must call curl exactly twice: {calls:?}");
        assert!(calls[0].contains(":9/hook"), "first attempt hits the stale port: {calls:?}");
        assert!(
            calls[1].contains(":23456/hook"),
            "retry hits the port from observer-port: {calls:?}",
        );

        let _ = std::fs::remove_dir_all(&scratch);
    }
}
