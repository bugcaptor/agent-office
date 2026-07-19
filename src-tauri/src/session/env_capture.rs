// src-tauri/src/session/env_capture.rs
//
// 봇 모드(#58) 시작 시 로그인 셸 환경을 1회 캡처해 프로세스 env에 병합한다.
//
// 왜 필요한가: GUI(Finder/launchd)로 띄운 번들 앱은 로그인 셸 프로파일
// (`.zshrc`/`.zprofile`)을 거치지 않아, 사용자가 `export`한 `GITEA_TOKEN`·
// `GITEA_BASE_URL`이 프로세스에 **비어 있고** `PATH`도 최소값
// (`/usr/bin:/bin:…`)이다. 이 때문에 봇의 app-side REST 폴링이 토큰을 못 얻어
// 실패했다(이슈 #58). 에이전트 PTY는 `-l -i` 로그인 셸(shells.rs:406)이라
// 프로파일을 소싱해 이 문제를 겪지 않으므로, 여기서 **같은 로그인 셸로 지정
// 키만** 캡처해 프로세스 env에 심으면 폴링(app)과 쓰기(agent PTY 상속) 양쪽이
// 동일한 자격/로케일을 본다.
//
// 안전장치: 지정 키만 캡처(민감 env 최소화), 타임아웃+실패 시 no-op 폴백(봇
// 시작을 블로킹하지 않음), 캡처값은 로그에 싣지 않는다.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};

/// 캡처 대상 키(지정 키만). 토큰/베이스 URL과 PATH·로케일에 한정한다.
const CAPTURE_KEYS: &[&str] = &[
    "GITEA_TOKEN",
    "GITEA_BASE_URL",
    "PATH",
    "LANG",
    "LC_ALL",
    "LC_CTYPE",
    "LC_MESSAGES",
];

/// 레코드 구분자(0x1E). rc 파일이 stdout에 찍는 잡음과 키=값 페어를 안전하게
/// 가른다(값에 이 바이트가 들어갈 일은 없다).
const RS: char = '\u{1e}';
/// 페어 앞에 찍는 시작 마커. 대화형 rc가 stdout에 프롬프트/배너를 남겨도
/// 이 마커 이후만 파싱해 오염을 막는다.
const START_MARKER: &str = "AO_ENV_CAPTURE_v1";

/// 로그인 셸 캡처 타임아웃. 봇 시작을 오래 막지 않도록 짧게 잡되, 무거운 rc를
/// 감안해 여유를 둔다. 초과하면 캡처를 포기(no-op)한다.
#[cfg(unix)]
const CAPTURE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(8);

/// 캡처가 성공적으로 반영됐는지. 성공 시에만 세워, 실패(타임아웃 등)했을 땐
/// 다음 봇 시작에서 재시도할 수 있게 둔다.
static CAPTURED: AtomicBool = AtomicBool::new(false);

/// 로그인 셸 env를 1회 캡처해 프로세스 env에 병합한다(멱등). 성공한 뒤로는
/// no-op. 실패하면 아무것도 바꾸지 않고 다음 호출에 재시도한다.
pub fn ensure_captured() {
    if CAPTURED.load(Ordering::Relaxed) {
        return;
    }
    if let Some(captured) = capture_login_shell_env() {
        apply_merge(&captured);
        CAPTURED.store(true, Ordering::Relaxed);
    }
}

/// 로그인 셸에서 지정 키를 읽어오는 스크립트를 만든다. 시작 마커를 먼저 찍고,
/// 각 키를 `NAME=VALUE`<RS> 형태로 나열한다. 키 이름을 셸 간접 확장 없이
/// 그대로 박아 zsh/bash 모두에서 동작하게 한다.
fn build_capture_script(keys: &[&str]) -> String {
    // printf의 8진 이스케이프로 RS(036) 삽입.
    let mut s = format!("printf '%s\\036' '{START_MARKER}';");
    for k in keys {
        // `printf 'NAME=%s\036' "$NAME"` — 값이 비어도 빈 문자열로 출력된다.
        s.push_str(&format!(" printf '{k}=%s\\036' \"${{{k}}}\";"));
    }
    s
}

/// 캡처 스크립트 stdout에서 키=값을 파싱한다. 시작 마커 이후만 보고, RS로 나눈
/// 각 세그먼트에서 첫 `=`를 기준으로 지정 키만 취한다(그 외 잡음 무시).
fn parse_captured(stdout: &[u8], keys: &[&str]) -> HashMap<String, String> {
    let text = String::from_utf8_lossy(stdout);
    let mut out = HashMap::new();
    let after = match text.find(START_MARKER) {
        Some(idx) => &text[idx + START_MARKER.len()..],
        None => return out, // 마커 없음 → 셸이 스크립트를 못 돌린 것으로 보고 포기
    };
    for seg in after.split(RS) {
        let Some((k, v)) = seg.split_once('=') else {
            continue;
        };
        if keys.contains(&k) {
            out.insert(k.to_string(), v.to_string());
        }
    }
    out
}

/// 캡처값을 현재 프로세스 env에 병합한다(덮어쓰기 아님):
/// - PATH: 현재 항목을 앞에 두고 캡처 항목 중 없는 것만 뒤에 덧붙인다(union).
///   번들의 최소 PATH를 Homebrew 등 경로로 보강하면서 기존 우선순위를 지킨다.
/// - 그 외 키: **현재 프로세스 값이 비어 있을 때만** 설정한다. dev 실행(터미널
///   기동)이나 `launchctl setenv`로 이미 있는 값을 존중한다.
/// 빈 캡처값은 무시한다(진짜 미설정을 덮지 않는다).
fn apply_merge(captured: &HashMap<String, String>) {
    for (k, v) in captured {
        if v.is_empty() {
            continue;
        }
        if k == "PATH" {
            let current = std::env::var("PATH").unwrap_or_default();
            let merged = merge_path(&current, v);
            if merged != current {
                std::env::set_var("PATH", merged);
            }
            continue;
        }
        let cur = std::env::var(k).unwrap_or_default();
        if cur.trim().is_empty() {
            std::env::set_var(k, v);
        }
    }
}

/// 현재 PATH에 캡처 PATH를 union한다: 현재 항목 순서를 유지하고, 현재에 없는
/// 캡처 항목만 뒤에 덧붙인다(중복 제거).
fn merge_path(current: &str, captured: &str) -> String {
    let mut seen: Vec<&str> = Vec::new();
    for p in current.split(':').filter(|p| !p.is_empty()) {
        if !seen.contains(&p) {
            seen.push(p);
        }
    }
    for p in captured.split(':').filter(|p| !p.is_empty()) {
        if !seen.contains(&p) {
            seen.push(p);
        }
    }
    seen.join(":")
}

/// `$SHELL -l -i -c <script>`를 타임아웃과 함께 실행해 stdout을 파싱한다.
/// 실패·타임아웃·마커 부재 시 None(캡처 포기). unix 전용.
#[cfg(unix)]
fn capture_login_shell_env() -> Option<HashMap<String, String>> {
    use std::io::Read;
    use std::process::{Command, Stdio};
    use std::sync::mpsc;
    use std::thread;
    use std::time::Instant;

    let shell = std::env::var("SHELL").unwrap_or_else(|_| {
        if cfg!(target_os = "macos") {
            "/bin/zsh".into()
        } else {
            "/bin/bash".into()
        }
    });
    let script = build_capture_script(CAPTURE_KEYS);

    // shells.rs의 로그인+인터랙티브 셸 패턴(-l -i)을 그대로 쓴다. 인터랙티브
    // 프롬프트/배너는 stderr로 가고(무시), 우리 데이터는 stdout으로만 나온다.
    let mut child = Command::new(&shell)
        .args(["-l", "-i", "-c", &script])
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;

    // 파이프 교착 방지를 위해 별도 스레드로 끝까지 읽는다(gitea.rs::run 패턴).
    let stdout = child.stdout.take();
    let (otx, orx) = mpsc::channel();
    if let Some(mut s) = stdout {
        thread::spawn(move || {
            let mut buf = Vec::new();
            let _ = s.read_to_end(&mut buf);
            let _ = otx.send(buf);
        });
    } else {
        let _ = otx.send(Vec::new());
    }

    let deadline = Instant::now() + CAPTURE_TIMEOUT;
    let ok = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status.success(),
            Ok(None) => {
                if Instant::now() >= deadline {
                    let _ = child.kill();
                    let _ = child.wait();
                    break false;
                }
                thread::sleep(std::time::Duration::from_millis(20));
            }
            Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                break false;
            }
        }
    };
    if !ok {
        return None;
    }
    let stdout = orx.recv().unwrap_or_default();
    let parsed = parse_captured(&stdout, CAPTURE_KEYS);
    if parsed.is_empty() {
        None
    } else {
        Some(parsed)
    }
}

/// 비-unix(Windows)에서는 GUI 기동이 로그인 셸 소싱을 요구하지 않으므로 no-op.
#[cfg(not(unix))]
fn capture_login_shell_env() -> Option<HashMap<String, String>> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn script_has_marker_and_all_keys() {
        let s = build_capture_script(&["GITEA_TOKEN", "PATH"]);
        assert!(s.contains(START_MARKER));
        assert!(s.contains("GITEA_TOKEN=%s"));
        assert!(s.contains("\"${GITEA_TOKEN}\""));
        assert!(s.contains("PATH=%s"));
    }

    #[test]
    fn parse_skips_noise_before_marker() {
        // rc가 앞에 배너를 찍어도 마커 이후만 파싱한다.
        let raw = format!(
            "welcome banner\nlast login blah{marker}\u{1e}GITEA_TOKEN=abc\u{1e}PATH=/opt/homebrew/bin\u{1e}",
            marker = START_MARKER
        );
        let m = parse_captured(raw.as_bytes(), CAPTURE_KEYS);
        assert_eq!(m.get("GITEA_TOKEN"), Some(&"abc".to_string()));
        assert_eq!(m.get("PATH"), Some(&"/opt/homebrew/bin".to_string()));
    }

    #[test]
    fn parse_returns_empty_without_marker() {
        let m = parse_captured(b"GITEA_TOKEN=abc\x1e", CAPTURE_KEYS);
        assert!(m.is_empty());
    }

    #[test]
    fn parse_keeps_empty_values_and_ignores_unknown_keys() {
        let raw = format!("{START_MARKER}\u{1e}GITEA_TOKEN=\u{1e}FOO=bar\u{1e}");
        let m = parse_captured(raw.as_bytes(), CAPTURE_KEYS);
        assert_eq!(m.get("GITEA_TOKEN"), Some(&String::new())); // 빈 값도 파싱은 됨(병합에서 걸러짐)
        assert!(!m.contains_key("FOO")); // 지정 외 키는 무시
    }

    #[test]
    fn parse_handles_value_with_equals() {
        // 값 안에 '='가 있어도 첫 '='만 기준으로 자른다.
        let raw = format!("{START_MARKER}\u{1e}GITEA_BASE_URL=http://h/?a=b\u{1e}");
        let m = parse_captured(raw.as_bytes(), CAPTURE_KEYS);
        assert_eq!(m.get("GITEA_BASE_URL"), Some(&"http://h/?a=b".to_string()));
    }

    #[test]
    fn merge_path_appends_only_missing_preserving_order() {
        assert_eq!(
            merge_path("/usr/bin:/bin", "/opt/homebrew/bin:/usr/bin:/usr/local/bin"),
            "/usr/bin:/bin:/opt/homebrew/bin:/usr/local/bin"
        );
    }

    #[test]
    fn merge_path_dedups_within_inputs() {
        assert_eq!(merge_path("/a:/a:/b", "/b:/c:/c"), "/a:/b:/c");
    }

    #[test]
    fn merge_path_handles_empty_current() {
        assert_eq!(merge_path("", "/opt/homebrew/bin"), "/opt/homebrew/bin");
    }
}
