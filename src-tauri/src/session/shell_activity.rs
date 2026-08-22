// src-tauri/src/session/shell_activity.rs
//
// 셸 탭 "작업 중" 감지 (kbm #2f9).
//
// 에이전트 CLI는 훅(observer)으로 턴 시작/종료를 알려 주지만, 사용자가 캐릭터
// 탭에서 그냥 셸을 띄워 놓고 `npm test`·`cargo build` 같은 긴 명령을 돌릴 때는
// 아무 신호도 없어 캐릭터가 계속 유휴로 보였다. 이 모듈은 PTY 마스터만 보고
// 그 공백을 메운다 — 출력/프롬프트 문자열 휴리스틱을 쓰지 않는다.
//
// 판정 = `tcgetpgrp(master) != 셸 pgid` **그리고** 터미널이 정규 모드(ICANON).
//   - 앞 조건: 셸이 프롬프트에서 대기 중이 아니라 자식 명령을 기다리는 중.
//   - 뒤 조건: 그 자식이 배치성 명령이지 대화형 TUI가 아님. zsh/bash의 라인
//     에디터도, vim·less·claude 같은 TUI도 raw 모드를 쓰므로 이 비트 하나로
//     "claude를 띄워 둔 세션이 영구 작업중으로 잡히는" 오탐이 사라진다
//     (에이전트 CLI의 진짜 작업중 표시는 기존 훅 경로가 담당한다).
//
// 방출은 전부 **기존 파이프라인 재사용**이다:
//   시작 → `ObserverEvent::Prompt`(= ActivityKind::Prompt, 턴 open)
//   미니미 → `ObserverEvent::SubCount`(= 서브에이전트 미니미와 완전히 같은 경로)
//   종료 → `ActivityKind::Idle`(턴 정산 전용 신호 — 알림은 내지 않는다)

use std::sync::Arc;
use std::time::Duration;

use crate::notification::hub::NotificationHub;
use crate::observer::ObserverEvent;
use crate::session::pty_factory::{ForegroundSample, PtyControl};
use crate::types::{now_ms, ActivityKind};

/// 포그라운드 프로세스 그룹을 들여다보는 주기.
pub const POLL_INTERVAL_MS: u64 = 500;
/// 이 시간 이상 연속으로 명령이 돌고 있어야 "작업 중"으로 보고한다. `ls`·`cd`
/// 같은 순식간 명령에 캐릭터가 깜빡이지 않게 하는 유일한 장치다.
pub const BUSY_ONSET_MS: u64 = 1_200;
/// 한 번 작업중으로 보고했으면 최소 이만큼은 유지한다(명령이 그 전에 끝나도).
/// 연속 명령 사이에 표시가 껐다 켜지는 것을 막는다.
pub const BUSY_MIN_VISIBLE_MS: u64 = 3_000;
/// 작업중인 동안 미니미 수(= 포그라운드 프로세스 그룹의 프로세스 수)를 다시 세는 주기.
pub const JOB_POLL_INTERVAL_MS: u64 = 2_000;
/// 라벨에 실을 명령 문자열 최대 길이(chars).
pub const MAX_COMMAND_CHARS: usize = 80;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ShellActivityConfig {
    pub onset_ms: u64,
    pub min_visible_ms: u64,
}

impl Default for ShellActivityConfig {
    fn default() -> Self {
        Self {
            onset_ms: BUSY_ONSET_MS,
            min_visible_ms: BUSY_MIN_VISIBLE_MS,
        }
    }
}

/// `observe()` 한 틱의 결정. `Start`는 "이 포그라운드 그룹이 새 작업으로
/// 확정됐다", `Stop`은 "작업중 표시를 거둔다".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShellAction {
    None,
    Start,
    Stop,
}

/// 순수 상태 머신 — 시계도 PTY도 모른다(호출자가 샘플과 `now_ms`를 준다).
#[derive(Debug)]
pub struct ShellActivityTracker {
    cfg: ShellActivityConfig,
    /// 셸 자신의 pgid. spawn 시점에 알면 주입하고, 모르면 첫 샘플에서 래치한다
    /// (세션이 막 뜬 시점의 포그라운드는 언제나 셸 자신이다).
    shell_pgid: Option<i32>,
    /// 아직 onset 디바운스를 채우지 못한 후보 그룹.
    candidate: Option<i32>,
    candidate_since: u64,
    /// 작업중으로 보고한 그룹과 그 시각(최소 표시시간 기준).
    reported: Option<i32>,
    reported_at: u64,
}

impl ShellActivityTracker {
    pub fn new(shell_pgid: Option<i32>, cfg: ShellActivityConfig) -> Self {
        Self {
            cfg,
            shell_pgid,
            candidate: None,
            candidate_since: 0,
            reported: None,
            reported_at: 0,
        }
    }

    pub fn is_busy(&self) -> bool {
        self.reported.is_some()
    }

    /// 현재 작업중으로 보고된 포그라운드 그룹(미니미 카운트/명령 조회의 키).
    pub fn foreground_pgid(&self) -> Option<i32> {
        self.reported
    }

    pub fn observe(&mut self, sample: ForegroundSample, now: u64) -> ShellAction {
        // 셸 pgid 래치: 세션 시작 직후 첫 포그라운드는 셸 자신이다.
        if self.shell_pgid.is_none() {
            self.shell_pgid = sample.fg_pgid;
        }

        let running = match sample.fg_pgid {
            // 포그라운드가 셸 자신이면 프롬프트 대기 중. raw 모드면 대화형
            // 프로그램(TUI/에이전트 CLI)이 화면을 쥐고 있는 것이라 셸 작업이 아니다.
            Some(pgid) if Some(pgid) != self.shell_pgid && sample.canonical => Some(pgid),
            _ => None,
        };

        let Some(pgid) = running else {
            self.candidate = None;
            if self.reported.is_some() && now.saturating_sub(self.reported_at) >= self.cfg.min_visible_ms
            {
                self.reported = None;
                return ShellAction::Stop;
            }
            return ShellAction::None;
        };

        if self.reported == Some(pgid) {
            // 같은 명령이 계속 도는 중 — 보고할 것 없음.
            self.candidate = None;
            return ShellAction::None;
        }

        // 새 후보(첫 명령이거나, 앞 명령에 이어 다른 명령이 시작됐거나).
        if self.candidate != Some(pgid) {
            self.candidate = Some(pgid);
            self.candidate_since = now;
        }
        if now.saturating_sub(self.candidate_since) < self.cfg.onset_ms {
            return ShellAction::None;
        }
        self.candidate = None;
        self.reported = Some(pgid);
        self.reported_at = now;
        ShellAction::Start
    }
}

// ── 명령 이름 / 동시 작업 수 조회 ───────────────────────────────────────
//
// 둘 다 포그라운드 그룹이 확정된 뒤에만, 그것도 드물게 부르는 보조 조회다
// (시작 1회 + 작업중 2초마다 1회). 실패하면 조용히 폴백한다 — 표시 기능이
// 셸 세션 동작을 막아선 안 된다.

/// `ps -o command= -p <pid>` 출력 → 라벨용 명령 문자열. 그룹 리더가 이미
/// 죽었으면 빈 출력이라 None.
pub fn parse_ps_command(stdout: &str) -> Option<String> {
    let line = stdout.lines().find(|l| !l.trim().is_empty())?.trim();
    if line.is_empty() {
        return None;
    }
    Some(line.chars().take(MAX_COMMAND_CHARS).collect())
}

/// `pgrep -g <pgid>` 출력 → 그 그룹에 속한 프로세스 수. 파이프라인 폭이나
/// 병렬 빌드 자식 수가 그대로 미니미 수가 된다. 숫자가 아닌 줄은 무시한다.
pub fn count_pgrep_pids(stdout: &str) -> u32 {
    stdout
        .lines()
        .filter(|l| {
            let t = l.trim();
            !t.is_empty() && t.chars().all(|c| c.is_ascii_digit())
        })
        .count() as u32
}

fn run_capture(program: &str, args: &[&str]) -> Option<String> {
    let out = std::process::Command::new(program)
        .args(args)
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .output()
        .ok()?;
    Some(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// 포그라운드 그룹 리더의 명령줄(라벨 텍스트). 그룹 리더의 pid == pgid다.
fn foreground_command(pgid: i32) -> Option<String> {
    let pid = pgid.to_string();
    parse_ps_command(&run_capture("ps", &["-o", "command=", "-p", &pid])?)
}

/// 포그라운드 그룹의 프로세스 수. 조회 실패 시 최소 1(명령이 도는 건 확실하다).
fn foreground_job_count(pgid: i32) -> u32 {
    let pgid = pgid.to_string();
    let counted = run_capture("pgrep", &["-g", &pgid])
        .map(|out| count_pgrep_pids(&out))
        .unwrap_or(0);
    counted.max(1)
}

// ── 감시 스레드 ─────────────────────────────────────────────────────────

/// 세션 하나의 포그라운드를 감시하는 스레드를 띄운다. `alive`가 false를
/// 돌려주면(세션 종료·핸드오프) 스레드가 스스로 끝난다.
///
/// `control.foreground_sample()`이 None을 주는 컨트롤(Windows·Fake·브로커
/// 소유 세션)에서는 호출자가 아예 부르지 않는 것이 정상이지만, 방어적으로
/// 연속 미지원 샘플이 이어지면 스레드를 접는다.
pub fn spawn_watcher(
    session_id: String,
    control: Arc<dyn PtyControl>,
    hub: Arc<NotificationHub>,
    shell_pgid: Option<i32>,
    alive: Arc<dyn Fn() -> bool + Send + Sync>,
) {
    std::thread::spawn(move || watch_loop(session_id, control, hub, shell_pgid, alive));
}

fn watch_loop(
    session_id: String,
    control: Arc<dyn PtyControl>,
    hub: Arc<NotificationHub>,
    shell_pgid: Option<i32>,
    alive: Arc<dyn Fn() -> bool + Send + Sync>,
) {
    let mut tracker = ShellActivityTracker::new(shell_pgid, ShellActivityConfig::default());
    let mut unsupported = 0u32;
    let mut last_job_poll = 0u64;
    let mut last_count = 0u32;

    loop {
        std::thread::sleep(Duration::from_millis(POLL_INTERVAL_MS));
        if !alive() {
            break;
        }
        let Some(sample) = control.foreground_sample() else {
            unsupported += 1;
            if unsupported >= 4 {
                break;
            }
            continue;
        };
        unsupported = 0;
        let now = now_ms();
        match tracker.observe(sample, now) {
            ShellAction::Start => {
                let pgid = tracker.foreground_pgid();
                let text = pgid.and_then(foreground_command);
                hub.ingest_observer(&session_id, ObserverEvent::Prompt { text, cwd: None });
                last_count = pgid.map(foreground_job_count).unwrap_or(1);
                last_job_poll = now;
                hub.ingest_observer(
                    &session_id,
                    ObserverEvent::SubCount {
                        running: last_count,
                    },
                );
            }
            ShellAction::Stop => {
                last_count = 0;
                hub.ingest_observer(&session_id, ObserverEvent::SubCount { running: 0 });
                hub.ingest_activity(&session_id, ActivityKind::Idle);
            }
            ShellAction::None => {
                if tracker.is_busy() && now.saturating_sub(last_job_poll) >= JOB_POLL_INTERVAL_MS {
                    last_job_poll = now;
                    let n = tracker.foreground_pgid().map(foreground_job_count).unwrap_or(1);
                    if n != last_count {
                        last_count = n;
                        hub.ingest_observer(
                            &session_id,
                            ObserverEvent::SubCount { running: n },
                        );
                    }
                }
            }
        }
    }

    // 세션이 사라지며 스레드가 끝나는 길 — 미니미만 거둬 둔다. 열려 있던 턴은
    // 렌더러가 session-state(exited/disposed)에서 강제 정산한다.
    if tracker.is_busy() {
        hub.ingest_observer(&session_id, ObserverEvent::SubCount { running: 0 });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(fg: Option<i32>, canonical: bool) -> ForegroundSample {
        ForegroundSample {
            fg_pgid: fg,
            canonical,
        }
    }

    fn tracker() -> ShellActivityTracker {
        ShellActivityTracker::new(Some(100), ShellActivityConfig::default())
    }

    #[test]
    fn prompt_wait_is_not_busy() {
        let mut t = tracker();
        // 셸이 포그라운드(zle raw 모드) — 프롬프트 대기.
        assert_eq!(t.observe(sample(Some(100), false), 0), ShellAction::None);
        assert_eq!(t.observe(sample(Some(100), false), 10_000), ShellAction::None);
        assert!(!t.is_busy());
    }

    #[test]
    fn long_foreground_command_starts_after_onset_debounce() {
        let mut t = tracker();
        assert_eq!(t.observe(sample(Some(200), true), 0), ShellAction::None);
        assert_eq!(t.observe(sample(Some(200), true), 500), ShellAction::None);
        assert_eq!(t.observe(sample(Some(200), true), 1_000), ShellAction::None);
        assert_eq!(t.observe(sample(Some(200), true), 1_200), ShellAction::Start);
        assert!(t.is_busy());
        assert_eq!(t.foreground_pgid(), Some(200));
        // 같은 명령이 계속 돌아도 재보고는 없다.
        assert_eq!(t.observe(sample(Some(200), true), 5_000), ShellAction::None);
    }

    #[test]
    fn short_command_never_reports() {
        let mut t = tracker();
        assert_eq!(t.observe(sample(Some(200), true), 0), ShellAction::None);
        assert_eq!(t.observe(sample(Some(200), true), 500), ShellAction::None);
        // 1.2초 전에 끝났다 — 시작 자체를 안 했으므로 Stop도 없다.
        assert_eq!(t.observe(sample(Some(100), false), 900), ShellAction::None);
        assert!(!t.is_busy());
    }

    #[test]
    fn interactive_tui_in_raw_mode_is_never_busy() {
        let mut t = tracker();
        // claude/vim처럼 포그라운드가 셸이 아니지만 raw 모드인 경우.
        for at in [0u64, 1_000, 5_000, 60_000] {
            assert_eq!(t.observe(sample(Some(300), false), at), ShellAction::None);
        }
        assert!(!t.is_busy());
    }

    #[test]
    fn stop_waits_for_min_visible_time() {
        let mut t = tracker();
        t.observe(sample(Some(200), true), 0);
        assert_eq!(t.observe(sample(Some(200), true), 1_200), ShellAction::Start);
        // 명령이 1.3초에 끝났지만 최소 표시시간(3초)까지는 유지.
        assert_eq!(t.observe(sample(Some(100), false), 1_300), ShellAction::None);
        assert_eq!(t.observe(sample(Some(100), false), 4_000), ShellAction::None);
        assert_eq!(t.observe(sample(Some(100), false), 4_200), ShellAction::Stop);
        assert!(!t.is_busy());
        // 이미 거뒀으면 반복 Stop은 없다.
        assert_eq!(t.observe(sample(Some(100), false), 9_000), ShellAction::None);
    }

    #[test]
    fn consecutive_commands_restart_without_a_gap() {
        let mut t = tracker();
        t.observe(sample(Some(200), true), 0);
        assert_eq!(t.observe(sample(Some(200), true), 1_200), ShellAction::Start);
        // 다음 명령(다른 pgid)도 onset을 채워야 새 턴으로 확정된다.
        assert_eq!(t.observe(sample(Some(201), true), 2_000), ShellAction::None);
        assert_eq!(t.observe(sample(Some(201), true), 3_200), ShellAction::Start);
        assert_eq!(t.foreground_pgid(), Some(201));
    }

    #[test]
    fn shell_pgid_is_latched_from_the_first_sample_when_unknown() {
        let mut t = ShellActivityTracker::new(None, ShellActivityConfig::default());
        // 첫 샘플의 포그라운드(777)가 셸로 래치된다 -- 정규 모드여도 작업중이 아니다.
        assert_eq!(t.observe(sample(Some(777), true), 0), ShellAction::None);
        assert_eq!(t.observe(sample(Some(777), true), 10_000), ShellAction::None);
        assert!(!t.is_busy());
        // 그 뒤 다른 그룹이 포그라운드를 잡으면 그때부터 작업중이다.
        assert_eq!(t.observe(sample(Some(888), true), 10_000), ShellAction::None);
        assert_eq!(t.observe(sample(Some(888), true), 11_200), ShellAction::Start);
    }

    #[test]
    fn missing_foreground_pgid_is_not_busy() {
        let mut t = tracker();
        assert_eq!(t.observe(sample(None, true), 0), ShellAction::None);
        assert_eq!(t.observe(sample(None, true), 5_000), ShellAction::None);
        assert!(!t.is_busy());
    }

    #[test]
    fn parse_ps_command_takes_the_first_non_empty_line() {
        assert_eq!(parse_ps_command("npm test\n").as_deref(), Some("npm test"));
        assert_eq!(parse_ps_command("\n\n  cargo build \n").as_deref(), Some("cargo build"));
        assert_eq!(parse_ps_command(""), None);
        assert_eq!(parse_ps_command("   \n"), None);
    }

    #[test]
    fn parse_ps_command_truncates_long_command_lines() {
        let long = "x".repeat(MAX_COMMAND_CHARS + 50);
        assert_eq!(parse_ps_command(&long).unwrap().chars().count(), MAX_COMMAND_CHARS);
    }

    /// 진짜 로그인 셸을 PTY로 띄워 이 모듈의 유일한 가정 두 개를 확인한다:
    /// ① 프롬프트 대기 중에는 (포그라운드 == 셸) 이거나 raw 모드다.
    /// ② 배치 명령이 도는 동안에는 (포그라운드 != 셸) 이고 정규 모드다.
    /// 셸 기동 시간과 `$SHELL` 편차 때문에 기본 `cargo test`에서는 제외한다:
    ///   cargo test --manifest-path src-tauri/Cargo.toml -- --ignored real_shell
    #[cfg(unix)]
    #[test]
    #[ignore = "real PTY; run explicitly"]
    fn real_shell_foreground_probe_separates_prompt_from_running_command() {
        use crate::session::pty_factory::{PortablePtyFactory, PtyFactory, PtySpawnOptions};
        use std::io::{Read, Write};

        let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".to_string());
        let mut spawned = PortablePtyFactory
            .spawn(PtySpawnOptions {
                shell,
                args: vec!["-l".into(), "-i".into()],
                cols: 80,
                rows: 24,
                cwd: std::env::var("HOME").unwrap_or_else(|_| "/".to_string()),
                env: vec![("TERM".into(), "xterm-256color".into())],
                agent_id: "smoke".into(),
                session_id: "smoke".into(),
                cleanup_paths: vec![],
            })
            .expect("spawn real pty");
        let shell_pgid = spawned.handoff.as_ref().and_then(|h| h.pgid);
        assert!(shell_pgid.is_some(), "spawn 시점 셸 pgid를 알아야 한다");

        // 셸이 출력 버퍼에 막히지 않게 계속 빨아들인다.
        let mut reader = std::mem::replace(&mut spawned.reader, Box::new(std::io::empty()));
        std::thread::spawn(move || {
            let mut buf = [0u8; 4096];
            while matches!(reader.read(&mut buf), Ok(n) if n > 0) {}
        });
        std::thread::sleep(Duration::from_millis(1_500)); // 프롬프트가 뜰 때까지

        let mut tracker =
            ShellActivityTracker::new(shell_pgid, ShellActivityConfig::default());
        let at_prompt = spawned.control.foreground_sample().expect("probe supported");
        for at in [0u64, 600, 1_200, 5_000] {
            assert_eq!(
                tracker.observe(at_prompt, at),
                ShellAction::None,
                "프롬프트 대기 중에는 작업중이 아니어야 한다: {at_prompt:?}"
            );
        }

        spawned
            .writer
            .write_all(b"sleep 4\r")
            .and_then(|()| spawned.writer.flush())
            .expect("write command");
        std::thread::sleep(Duration::from_millis(1_200));

        let running = spawned.control.foreground_sample().expect("probe supported");
        assert_ne!(running.fg_pgid, shell_pgid, "포그라운드가 자식으로 넘어가야 한다");
        assert!(running.canonical, "배치 명령 중에는 정규 모드여야 한다: {running:?}");
        assert_eq!(tracker.observe(running, 10_000), ShellAction::None);
        assert_eq!(tracker.observe(running, 11_200), ShellAction::Start);
        let pgid = tracker.foreground_pgid().expect("reported pgid");
        assert_eq!(foreground_command(pgid).as_deref(), Some("sleep 4"));
        assert!(foreground_job_count(pgid) >= 1);

        let _ = spawned.control.kill();
    }

    /// 대화형 TUI(vim·less·claude) 억제의 실물 확인. 포그라운드 자식이 터미널을
    /// raw 모드로 바꾸면, 포그라운드가 셸이 아님에도 작업중으로 잡히지 않아야 한다.
    #[cfg(unix)]
    #[test]
    #[ignore = "real PTY; run explicitly"]
    fn real_shell_raw_mode_child_is_not_reported_as_busy() {
        use crate::session::pty_factory::{PortablePtyFactory, PtyFactory, PtySpawnOptions};
        use std::io::{Read, Write};

        let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".to_string());
        let mut spawned = PortablePtyFactory
            .spawn(PtySpawnOptions {
                shell,
                args: vec!["-l".into(), "-i".into()],
                cols: 80,
                rows: 24,
                cwd: std::env::var("HOME").unwrap_or_else(|_| "/".to_string()),
                env: vec![("TERM".into(), "xterm-256color".into())],
                agent_id: "smoke".into(),
                session_id: "smoke".into(),
                cleanup_paths: vec![],
            })
            .expect("spawn real pty");
        let shell_pgid = spawned.handoff.as_ref().and_then(|h| h.pgid);
        let mut reader = std::mem::replace(&mut spawned.reader, Box::new(std::io::empty()));
        std::thread::spawn(move || {
            let mut buf = [0u8; 4096];
            while matches!(reader.read(&mut buf), Ok(n) if n > 0) {}
        });
        std::thread::sleep(Duration::from_millis(1_500));

        // vim 같은 TUI를 흉내낸다 — 자식이 스스로 터미널을 raw로 바꾸고 오래 산다.
        spawned
            .writer
            .write_all(b"sh -c 'stty raw; sleep 4'\r")
            .and_then(|()| spawned.writer.flush())
            .expect("write command");
        std::thread::sleep(Duration::from_millis(1_500));

        let sample = spawned.control.foreground_sample().expect("probe supported");
        assert_ne!(sample.fg_pgid, shell_pgid, "포그라운드는 자식이어야 한다");
        assert!(!sample.canonical, "raw 모드여야 한다: {sample:?}");

        let mut tracker =
            ShellActivityTracker::new(shell_pgid, ShellActivityConfig::default());
        for at in [0u64, 1_200, 3_000, 30_000] {
            assert_eq!(tracker.observe(sample, at), ShellAction::None);
        }
        assert!(!tracker.is_busy());

        let _ = spawned.control.kill();
    }

    #[test]
    fn count_pgrep_pids_counts_only_numeric_lines() {
        assert_eq!(count_pgrep_pids("101\n102\n103\n"), 3);
        assert_eq!(count_pgrep_pids(""), 0);
        assert_eq!(count_pgrep_pids("\n\n"), 0);
        assert_eq!(count_pgrep_pids("101\npgrep: bad\n102\n"), 2);
    }
}
