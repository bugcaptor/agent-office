// src-tauri/src/session/tmux_host.rs
//
// 프로필의 "tmux 호스팅"(kbm #2pc)이 새 세션마다 스스로 여는 tmux 세션 관리.
// `control/tmux.rs`의 `--tmux <target>` 수동 attach와는 목적이 다르다 — 여기는
// **우리가 tmux 세션 자체를 만들고 이름을 관리**한다(생성·고아 정리·종료).
// 사용자가 손으로 붙는 `ctl attach --tmux`는 손대지 않는다.
//
// 이 모듈은 순수 로직 + 러너 주입만 담는다(M2). `SessionManager`가 실제로
// 이 러너들을 호출해 새 세션을 여는 분기는 아직 없다(M3).
//
// 러너 주입은 `control/tmux.rs`의 `TmuxProbe` 관례를 그대로 복제한다 — 클로저를
// `Arc`로 감싸 프로덕션은 `system_runner()`, 테스트는 가짜를 넣는다. 그래야
// 이 모듈의 순수 함수·오케스트레이션 테스트가 실제 tmux 설치 여부와 무관하게
// 전 플랫폼에서 돈다. `#[cfg(unix)]`로 감싸지 않는 이유도 같다 — Windows 차단은
// M3의 매니저 분기가 `cfg!(windows)`로 한다.
use std::sync::Arc;

use crate::session::wrapper_script::sh_quote;

/// `tmux` 서브커맨드 한 번 실행 결과.
pub struct TmuxRun {
    /// 종료 코드가 0이었나.
    pub ok: bool,
    pub stdout: String,
    pub stderr: String,
}

/// tmux 서브프로세스 실행기. `Err` = tmux 실행 자체 실패(미설치·PATH 밖).
/// `Ok(TmuxRun { ok: false, .. })` = tmux는 돌았고 비영 종료(예: 세션 없음).
pub type TmuxRunner = Arc<dyn Fn(&[String]) -> Result<TmuxRun, String> + Send + Sync>;

/// 실제 `tmux <argv...>`를 돌리는 러너.
pub fn system_runner() -> TmuxRunner {
    Arc::new(|argv: &[String]| {
        let output = std::process::Command::new("tmux")
            .args(argv)
            .stdin(std::process::Stdio::null())
            .output()
            .map_err(|e| e.to_string())?;
        Ok(TmuxRun {
            ok: output.status.success(),
            stdout: String::from_utf8_lossy(&output.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&output.stderr).into_owned(),
        })
    })
}

/// 슬러그에 ASCII가 하나도 안 남을 때의 폴백(순한글 폴더명 등).
const SLUG_FALLBACK: &str = "work";

/// 작업 폴더 슬러그의 최대 길이. 캐릭터 이름 조각(12자)보다 넉넉하다 —
/// 이름에 남는 사람이 읽는 정보가 이제 이것뿐이라 잘리면 손해가 크다.
const DIR_SLUG_MAX: usize = 20;

/// tmux 세션 이름에 쓸 조각을 뽑는다. 소문자화 → `[a-z0-9]` 외 전부 `-` →
/// 연속 `-` 압축 → 앞뒤 `-` 제거 → 최대 `max_len`자. 비면 `"work"`.
pub fn slug(raw: &str, max_len: usize) -> String {
    let lower = raw.to_lowercase();
    let mut out = String::with_capacity(lower.len());
    let mut last_was_dash = false;
    for ch in lower.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch);
            last_was_dash = false;
        } else if !last_was_dash {
            out.push('-');
            last_was_dash = true;
        }
    }
    let trimmed = out.trim_matches('-');
    let truncated: String = trimmed.chars().take(max_len).collect();
    let truncated = truncated.trim_end_matches('-');
    if truncated.is_empty() {
        SLUG_FALLBACK.to_string()
    } else {
        truncated.to_string()
    }
}

/// 세션 소유자(agentId)를 적어 두는 tmux 사용자 옵션. gc가 "그 캐릭터 것만"
/// 죽이는 근거가 이름이 아니라 이 옵션이다 — 그래야 이름을 사람이 읽기 좋게
/// 자유롭게 지을 수 있다.
pub const AGENT_OPTION: &str = "@ao_agent";

/// 작업 폴더(cwd)의 마지막 조각에서 뽑은 슬러그. `/a/b/agent-office/` →
/// `agent-office`. 경로가 통째로 비면 폴백.
pub fn dir_slug(cwd: &str) -> String {
    let trimmed = cwd.trim_end_matches('/');
    let base = trimmed.rsplit('/').next().unwrap_or(trimmed);
    slug(base, DIR_SLUG_MAX)
}

/// 실제 tmux 세션 이름. `ao-<폴더 슬러그>-<HHMM>`이 기본형이고, 그 이름이
/// 이미 있으면 초까지, 그래도 겹치면 sid 조각까지 붙여 피한다. `taken`은
/// gc 직전에 받아 둔 `list-sessions` 결과(우리 것이 아닌 세션도 포함)다.
pub fn session_name(cwd: &str, hms: (u32, u32, u32), sid: &str, taken: &[&str]) -> String {
    let dir = dir_slug(cwd);
    let (h, m, sec) = hms;
    let sid_part: String = sid.chars().take(4).collect();
    let candidates = [
        format!("ao-{dir}-{h:02}{m:02}"),
        format!("ao-{dir}-{h:02}{m:02}{sec:02}"),
        format!("ao-{dir}-{h:02}{m:02}{sec:02}-{sid_part}"),
    ];
    for candidate in &candidates {
        if !taken.contains(&candidate.as_str()) {
            return candidate.clone();
        }
    }
    // 셋 다 겹치는 건 같은 초에 같은 sid 앞 4자가 또 나온 경우뿐이다.
    format!("ao-{dir}-{h:02}{m:02}{sec:02}-{sid}")
}

/// `"tmux 3.5a"`, `"tmux 3.1c"`, `"tmux next-3.4"` 같은 `tmux -V` 출력에서
/// (major, minor)를 뽑는다. 못 알아보면 `None`.
pub fn parse_version(raw: &str) -> Option<(u32, u32)> {
    let s = raw.trim();
    let s = s.strip_prefix("tmux ").unwrap_or(s);
    // "next-3.4" 처럼 숫자 앞에 알파벳/대시 접두가 붙는 빌드가 있다 — 첫
    // 숫자부터 본다.
    let start = s.find(|c: char| c.is_ascii_digit())?;
    let rest = &s[start..];
    let mut parts = rest.splitn(2, '.');
    let major_str = parts.next()?;
    let minor_str = parts.next()?;
    let major: u32 = major_str.parse().ok()?;
    // 마이너 뒤에 "a"/"c" 같은 패치 문자나 개행이 붙을 수 있다 — 선행 숫자만.
    let minor_digits: String = minor_str.chars().take_while(|c| c.is_ascii_digit()).collect();
    if minor_digits.is_empty() {
        return None;
    }
    let minor: u32 = minor_digits.parse().ok()?;
    Some((major, minor))
}

/// `new-session -e`(pane 환경변수 직접 지정) 지원 여부. tmux 3.2부터.
pub fn supports_env_flag(version: (u32, u32)) -> bool {
    version >= (3, 2)
}

/// `list-sessions` 한 줄.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionRow {
    pub name: String,
    pub attached: bool,
    /// `AGENT_OPTION` 값. 우리가 만들지 않은 세션이면 빈 문자열.
    pub owner: String,
}

/// gc·이름 중복 판정에 쓰는 `list-sessions` argv.
///
/// 구분자가 공백이 아니라 탭인 이유: 세션 이름에는 공백이 들어갈 수 있고
/// (밖에서 사람이 만든 세션), 이름을 맨 뒤로 보내야 앞 두 칸만 잘라내고
/// 나머지를 통째로 이름으로 받을 수 있다.
pub fn list_sessions_args() -> Vec<String> {
    vec![
        "list-sessions".to_string(),
        "-F".to_string(),
        format!("#{{session_attached}}\t#{{{AGENT_OPTION}}}\t#{{session_name}}"),
    ]
}

/// `list_sessions_args` 출력을 행 목록으로 파싱한다. attached는 `"0"`이
/// 아니면 true.
pub fn parse_sessions(stdout: &str) -> Vec<SessionRow> {
    stdout
        .lines()
        .filter_map(|line| {
            let line = line.trim_end_matches('\r');
            if line.is_empty() {
                return None;
            }
            let mut parts = line.splitn(3, '\t');
            let attached = parts.next()?;
            let owner = parts.next()?;
            let name = parts.next()?;
            if name.is_empty() {
                return None;
            }
            Some(SessionRow {
                name: name.to_string(),
                attached: attached != "0",
                owner: owner.to_string(),
            })
        })
        .collect()
}

/// `list`에서 이 캐릭터가 만들었고 붙어 있지 않은 세션 이름만 골라낸다(고아).
pub fn orphans<'a>(list: &'a [SessionRow], agent_id: &str) -> Vec<&'a str> {
    list.iter()
        .filter(|row| !row.attached && !row.owner.is_empty() && row.owner == agent_id)
        .map(|row| row.name.as_str())
        .collect()
}

/// pane에 심을 환경변수. plan env + extra_env에서 `TERM`만 뺀다 — TERM은
/// 앱 PTY(=tmux 클라이언트)용 값이라 pane에 그대로 박으면 잘못된 terminfo로
/// 뜬다(pane의 TERM은 tmux가 직접 `tmux-256color`로 정한다).
pub fn pane_env(env: &[(String, String)]) -> Vec<(String, String)> {
    env.iter()
        .filter(|(k, _)| k != "TERM")
        .cloned()
        .collect()
}

/// `new-session -d -s <name> -c <cwd> -e K=V...` argv.
pub fn new_session_args(name: &str, cwd: &str, env: &[(String, String)]) -> Vec<String> {
    let mut argv = vec![
        "new-session".to_string(),
        "-d".to_string(),
        "-s".to_string(),
        name.to_string(),
        "-c".to_string(),
        cwd.to_string(),
    ];
    for (k, v) in env {
        argv.push("-e".to_string());
        argv.push(format!("{k}={v}"));
    }
    argv
}

/// `send-keys -t '=<name>' <command> Enter` argv. 명령은 단일 argv(쉘 파싱을
/// 거치지 않는다), `Enter`는 별도 argv로 tmux가 키 이름으로 해석한다.
pub fn send_keys_args(name: &str, command: &str) -> Vec<String> {
    vec![
        "send-keys".to_string(),
        "-t".to_string(),
        format!("={name}"),
        command.to_string(),
        "Enter".to_string(),
    ]
}

/// `set-option -t '=<name>' @ao_agent <agentId>` argv. 세션 소유자 표시.
pub fn set_owner_args(name: &str, agent_id: &str) -> Vec<String> {
    vec![
        "set-option".to_string(),
        "-t".to_string(),
        format!("={name}"),
        AGENT_OPTION.to_string(),
        agent_id.to_string(),
    ]
}

/// `kill-session -t '=<name>'` argv.
pub fn kill_session_args(name: &str) -> Vec<String> {
    vec!["kill-session".to_string(), "-t".to_string(), format!("={name}")]
}

/// 우리가 만든 이름에 정확히 붙는 attach 명령. `-t`는 `=` 정확일치를 써서
/// `ao-agent-office-2134`가 `ao-agent-office-213456`을 잡는 접두 매칭 사고를
/// 막는다(중복 회피가 초를 덧붙이므로 실제로 생길 수 있는 짝이다).
/// `ctl attach --tmux`(사용자가 손으로 치는 이름)는 퍼지 매칭이 편의이므로
/// `control/tmux.rs::attach_command`를 그대로 쓴다 — 이건 별개.
pub fn attach_command_exact(name: &str) -> String {
    format!("exec tmux attach-session -t {}", sh_quote(&format!("={name}")))
}

/// `probe_version`·`ensure_hosted`·`kill`이 반환하는 실패 사유.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HostError {
    /// 이 플랫폼(Windows)에서는 tmux 호스팅을 지원하지 않는다.
    Unsupported,
    /// tmux 실행 자체가 실패했다(미설치·PATH 밖). 문자열은 원인.
    Missing(String),
    /// tmux는 있는데 `-e` 미지원 등 버전이 너무 낮다.
    TooOld((u32, u32)),
    /// tmux는 돌았는데 명령이 비영 종료했다.
    Failed(String),
}

/// `tmux -V`로 버전을 확인한다.
pub fn probe_version(runner: &TmuxRunner) -> Result<(u32, u32), HostError> {
    let run = runner(&["-V".to_string()]).map_err(HostError::Missing)?;
    if !run.ok {
        return Err(HostError::Failed(run.stderr));
    }
    parse_version(&run.stdout).ok_or_else(|| HostError::Failed(format!("알 수 없는 tmux -V 출력: {}", run.stdout)))
}

/// 새 호스팅 세션을 만들 때 필요한 값.
pub struct HostSpec<'a> {
    pub agent_id: &'a str,
    pub sid: &'a str,
    pub cwd: &'a str,
    /// 세션 시작 시각(로컬) `(시, 분, 초)`. 이름에 들어간다. 호출자가 넘기는
    /// 이유는 테스트 때문이다 — 여기서 시계를 직접 읽으면 이름을 못 박는다.
    pub hms: (u32, u32, u32),
    pub env: &'a [(String, String)],
    /// pane에 주입할 시작 명령(있으면 send-keys). None/공백이면 미주입.
    pub startup_command: Option<&'a str>,
}

/// gc(고아 정리) → new-session → (있으면) send-keys. 성공하면 만든 세션
/// 이름을 반환한다.
///
/// gc의 `list-sessions`는 tmux 서버가 안 떠 있으면 비영 종료 + stderr에
/// `no server running`을 낸다 — 이건 에러가 아니라 "고아 없음"으로 본다.
pub fn ensure_hosted(
    runner: &TmuxRunner,
    version: (u32, u32),
    spec: HostSpec,
) -> Result<String, HostError> {
    if !supports_env_flag(version) {
        return Err(HostError::TooOld(version));
    }
    let list_run = runner(&list_sessions_args()).map_err(HostError::Missing)?;
    let sessions = if list_run.ok {
        parse_sessions(&list_run.stdout)
    } else if list_run.stderr.contains("no server running") {
        Vec::new()
    } else {
        return Err(HostError::Failed(list_run.stderr));
    };
    for orphan in orphans(&sessions, spec.agent_id) {
        kill(runner, orphan);
    }

    let taken: Vec<&str> = sessions.iter().map(|row| row.name.as_str()).collect();
    let name = session_name(spec.cwd, spec.hms, spec.sid, &taken);
    let env = pane_env(spec.env);
    let new_run = runner(&new_session_args(&name, spec.cwd, &env)).map_err(HostError::Missing)?;
    if !new_run.ok {
        return Err(HostError::Failed(new_run.stderr));
    }

    // 소유자 표시는 best-effort다. 실패해도 세션 자체는 멀쩡하니 죽이지
    // 않는다 — 대신 이 세션은 나중에 gc가 못 알아보고 남을 수 있다.
    match runner(&set_owner_args(&name, spec.agent_id)) {
        Ok(run) if run.ok => {}
        Ok(run) => eprintln!("tmux set-option {AGENT_OPTION} '{name}' 실패: {}", run.stderr),
        Err(e) => eprintln!("tmux set-option {AGENT_OPTION} '{name}' 실행 실패: {e}"),
    }

    if let Some(command) = spec.startup_command {
        if !command.trim().is_empty() {
            let send_run = runner(&send_keys_args(&name, command)).map_err(HostError::Missing)?;
            if !send_run.ok {
                return Err(HostError::Failed(send_run.stderr));
            }
        }
    }

    Ok(name)
}

/// 세션 종료. best-effort — 실패해도 패닉하지 않고 stderr에만 남긴다(앱
/// 종료 경로에서 이 하나 실패로 나머지 정리가 막히면 안 된다).
pub fn kill(runner: &TmuxRunner, name: &str) {
    match runner(&kill_session_args(name)) {
        Ok(run) if run.ok => {}
        Ok(run) => eprintln!("tmux kill-session '{name}' 실패: {}", run.stderr),
        Err(e) => eprintln!("tmux kill-session '{name}' 실행 실패: {e}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    #[test]
    fn slug_normalizes_and_falls_back() {
        assert_eq!(slug("김철수", 12), "work");
        assert_eq!(slug("Nova Kim", 12), "nova-kim");
        assert_eq!(slug("a.b:c", 12), "a-b-c");
        assert_eq!(slug("abcdefghijklmnop", 12), "abcdefghijkl"); // 절단
        assert_eq!(slug("", 12), "work");
        assert_eq!(slug("---", 12), "work");
    }

    #[test]
    fn dir_slug_takes_last_path_segment() {
        assert_eq!(dir_slug("/Users/me/dev/agent-office"), "agent-office");
        assert_eq!(dir_slug("/Users/me/dev/agent-office/"), "agent-office");
        assert_eq!(dir_slug("/Users/me/dev/ex-07-raylib_space"), "ex-07-raylib-space");
        assert_eq!(dir_slug("/Users/me/dev/작업"), "work");
        assert_eq!(dir_slug("/"), "work");
        // 20자 절단.
        assert_eq!(dir_slug("/x/abcdefghijklmnopqrstuvwxyz"), "abcdefghijklmnopqrst");
    }

    #[test]
    fn session_name_uses_folder_and_start_time() {
        assert_eq!(
            session_name("/Users/me/dev/agent-office", (9, 5, 3), "abcdef", &[]),
            "ao-agent-office-0905"
        );
        assert_eq!(
            session_name("/Users/me/dev/agent-office", (21, 34, 56), "abcdef", &[]),
            "ao-agent-office-2134"
        );
    }

    #[test]
    fn session_name_escalates_on_collision() {
        let cwd = "/Users/me/dev/agent-office";
        let taken1 = vec!["ao-agent-office-2134"];
        assert_eq!(
            session_name(cwd, (21, 34, 56), "abcdef", &taken1),
            "ao-agent-office-213456"
        );
        let taken2 = vec!["ao-agent-office-2134", "ao-agent-office-213456"];
        assert_eq!(
            session_name(cwd, (21, 34, 56), "abcdef", &taken2),
            "ao-agent-office-213456-abcd"
        );
    }

    #[test]
    fn parse_version_handles_known_formats() {
        assert_eq!(parse_version("tmux 3.2a\n"), Some((3, 2)));
        assert_eq!(parse_version("tmux 3.1c"), Some((3, 1)));
        assert_eq!(parse_version("tmux next-3.4"), Some((3, 4)));
        assert_eq!(parse_version("garbage"), None);
        assert_eq!(parse_version(""), None);
    }

    #[test]
    fn supports_env_flag_thresholds_at_3_2() {
        assert!(!supports_env_flag((3, 1)));
        assert!(supports_env_flag((3, 2)));
        assert!(!supports_env_flag((2, 9)));
        assert!(supports_env_flag((4, 0)));
    }

    #[test]
    fn parse_sessions_and_orphans_filter_correctly() {
        let stdout = "0\tab12cdef\tao-agent-office-2134\n\
                      1\tab12cdef\tao-agent-office-2140\n\
                      0\tef34ghij\tao-agent-office-2150\n\
                      0\t\tmy manual session\n";
        let sessions = parse_sessions(stdout);
        assert_eq!(sessions.len(), 4);
        assert_eq!(sessions[0].name, "ao-agent-office-2134");
        assert!(!sessions[0].attached);
        assert_eq!(sessions[0].owner, "ab12cdef");
        assert!(sessions[1].attached);
        // 이름에 공백이 있어도 통째로 이름이다(구분자가 탭이라).
        assert_eq!(sessions[3].name, "my manual session");
        assert_eq!(sessions[3].owner, "");

        let orphaned = orphans(&sessions, "ab12cdef");
        // 붙은 것 제외, 다른 캐릭터 소유 제외, 소유자 없는 사용자 세션 제외.
        assert_eq!(orphaned, vec!["ao-agent-office-2134"]);
    }

    #[test]
    fn pane_env_drops_only_term() {
        let env = vec![
            ("TERM".to_string(), "xterm-256color".to_string()),
            ("ZDOTDIR".to_string(), "/tmp/shim".to_string()),
            ("AGENT_OFFICE_HOOK_URL".to_string(), "http://x".to_string()),
        ];
        let out = pane_env(&env);
        assert!(!out.iter().any(|(k, _)| k == "TERM"));
        assert!(out.iter().any(|(k, _)| k == "ZDOTDIR"));
        assert!(out.iter().any(|(k, _)| k == "AGENT_OFFICE_HOOK_URL"));
    }

    /// 가짜 러너: `list-sessions`에 고정 응답을 주고, 그 이후 호출된 argv를
    /// 전부 기록한다.
    fn fake_runner(list_response: TmuxRun) -> (TmuxRunner, Arc<Mutex<Vec<Vec<String>>>>) {
        let calls: Arc<Mutex<Vec<Vec<String>>>> = Arc::new(Mutex::new(Vec::new()));
        let calls_for_closure = calls.clone();
        let list_stdout = list_response.stdout.clone();
        let list_stderr = list_response.stderr.clone();
        let list_ok = list_response.ok;
        let runner: TmuxRunner = Arc::new(move |argv: &[String]| {
            calls_for_closure.lock().unwrap().push(argv.to_vec());
            if argv.first().map(String::as_str) == Some("list-sessions") {
                return Ok(TmuxRun {
                    ok: list_ok,
                    stdout: list_stdout.clone(),
                    stderr: list_stderr.clone(),
                });
            }
            Ok(TmuxRun {
                ok: true,
                stdout: String::new(),
                stderr: String::new(),
            })
        });
        (runner, calls)
    }

    #[test]
    fn ensure_hosted_kills_orphans_and_builds_new_session_args() {
        let list_stdout = "0\tab12cdef\tao-work-0901\n\
                           0\tab12cdef\tao-work-0902\n\
                           1\tab12cdef\tao-work-0903\n\
                           0\tef34ghij\tao-work-0904\n";
        let (runner, calls) = fake_runner(TmuxRun {
            ok: true,
            stdout: list_stdout.to_string(),
            stderr: String::new(),
        });
        let env = vec![("ZDOTDIR".to_string(), "/tmp/shim".to_string())];
        let spec = HostSpec {
            agent_id: "ab12cdef",
            sid: "000099xyz",
            cwd: "/Users/me/dev/agent-office",
            hms: (21, 34, 56),
            env: &env,
            startup_command: Some("claude --resume abc"),
        };
        let name = ensure_hosted(&runner, (3, 4), spec).unwrap();
        assert_eq!(name, "ao-agent-office-2134");

        let calls = calls.lock().unwrap();
        // list-sessions, kill x2(내 고아만), new-session, set-option, send-keys.
        assert_eq!(calls[0][0], "list-sessions");
        let kills: Vec<&Vec<String>> = calls.iter().filter(|c| c[0] == "kill-session").collect();
        assert_eq!(kills.len(), 2);
        assert_eq!(kills[0], &vec!["kill-session".to_string(), "-t".to_string(), "=ao-work-0901".to_string()]);
        assert_eq!(kills[1], &vec!["kill-session".to_string(), "-t".to_string(), "=ao-work-0902".to_string()]);

        let new_session = calls.iter().find(|c| c[0] == "new-session").unwrap();
        assert_eq!(new_session[1], "-d");
        assert_eq!(new_session[2], "-s");
        assert_eq!(new_session[3], "ao-agent-office-2134");
        assert_eq!(new_session[4], "-c");
        assert_eq!(new_session[5], "/Users/me/dev/agent-office");
        assert_eq!(new_session[6], "-e");
        assert_eq!(new_session[7], "ZDOTDIR=/tmp/shim");

        let set_option = calls.iter().find(|c| c[0] == "set-option").unwrap();
        assert_eq!(
            set_option,
            &vec![
                "set-option".to_string(),
                "-t".to_string(),
                "=ao-agent-office-2134".to_string(),
                "@ao_agent".to_string(),
                "ab12cdef".to_string(),
            ]
        );

        let send_keys = calls.iter().find(|c| c[0] == "send-keys").unwrap();
        assert_eq!(
            send_keys,
            &vec![
                "send-keys".to_string(),
                "-t".to_string(),
                "=ao-agent-office-2134".to_string(),
                "claude --resume abc".to_string(),
                "Enter".to_string(),
            ]
        );
    }

    #[test]
    fn ensure_hosted_avoids_names_already_in_use() {
        let list_stdout = "1\tef34ghij\tao-agent-office-2134\n";
        let (runner, calls) = fake_runner(TmuxRun {
            ok: true,
            stdout: list_stdout.to_string(),
            stderr: String::new(),
        });
        let spec = HostSpec {
            agent_id: "ab12cdef",
            sid: "000099xyz",
            cwd: "/Users/me/dev/agent-office",
            hms: (21, 34, 56),
            env: &[],
            startup_command: None,
        };
        let name = ensure_hosted(&runner, (3, 4), spec).unwrap();
        assert_eq!(name, "ao-agent-office-213456");
        let calls = calls.lock().unwrap();
        // 남의 세션은 붙어 있든 아니든 죽이지 않는다.
        assert!(!calls.iter().any(|c| c[0] == "kill-session"));
    }

    #[test]
    fn ensure_hosted_skips_send_keys_without_startup_command() {
        let (runner, calls) = fake_runner(TmuxRun {
            ok: true,
            stdout: String::new(),
            stderr: String::new(),
        });
        let spec = HostSpec {
            agent_id: "ab12cdef",
            sid: "000099xyz",
            cwd: "/Users/me/dev/agent-office",
            hms: (21, 34, 56),
            env: &[],
            startup_command: None,
        };
        ensure_hosted(&runner, (3, 4), spec).unwrap();
        let calls = calls.lock().unwrap();
        assert!(!calls.iter().any(|c| c[0] == "send-keys"));
    }

    #[test]
    fn ensure_hosted_treats_no_server_running_as_empty_list() {
        let (runner, calls) = fake_runner(TmuxRun {
            ok: false,
            stdout: String::new(),
            stderr: "no server running".to_string(),
        });
        let spec = HostSpec {
            agent_id: "ab12cdef",
            sid: "000099xyz",
            cwd: "/Users/me/dev/agent-office",
            hms: (21, 34, 56),
            env: &[],
            startup_command: None,
        };
        let name = ensure_hosted(&runner, (3, 4), spec).unwrap();
        assert_eq!(name, "ao-agent-office-2134");
        let calls = calls.lock().unwrap();
        assert!(!calls.iter().any(|c| c[0] == "kill-session"));
        assert!(calls.iter().any(|c| c[0] == "new-session"));
    }

    #[test]
    fn probe_version_maps_runner_failure_to_missing() {
        let runner: TmuxRunner = Arc::new(|_argv: &[String]| Err("no such file or directory".to_string()));
        assert_eq!(
            probe_version(&runner),
            Err(HostError::Missing("no such file or directory".to_string()))
        );
    }

    #[test]
    fn ensure_hosted_rejects_versions_without_env_flag_support() {
        let (runner, _calls) = fake_runner(TmuxRun {
            ok: true,
            stdout: String::new(),
            stderr: String::new(),
        });
        let spec = HostSpec {
            agent_id: "ab12cdef",
            sid: "000099xyz",
            cwd: "/Users/me/dev/agent-office",
            hms: (21, 34, 56),
            env: &[],
            startup_command: None,
        };
        assert_eq!(ensure_hosted(&runner, (3, 1), spec), Err(HostError::TooOld((3, 1))));
    }

    #[test]
    fn attach_command_exact_uses_exact_match_and_sh_quote() {
        let cmd = attach_command_exact("a'b");
        // `=` 접두로 정확일치를 강제하고, sh_quote가 내부 작은따옴표를 데이터로 지킨다.
        assert_eq!(cmd, "exec tmux attach-session -t '=a'\"'\"'b'");
        assert!(cmd.starts_with("exec tmux attach-session -t '="));
    }
}
