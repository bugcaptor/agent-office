// src-tauri/src/shell_assist/recorder.rs
//
// 셸 캐릭터 세션 하나의 명령 블록 기록기(순수 — 시계는 호출자가 준다).
//
// PTY 바이트를 받아 표식(`marks.rs`)으로 명령 하나하나를 가르고, 그 사이 출력을
// 세션 로그와 같은 전사 필터(`TranscriptFilter`)로 읽을 수 있는 줄로 바꿔 둔다.
// 프롬프트에 있는 동안의 출력(프롬프트 그림, 사람이 치는 글자 메아리)은 기록하지 않는다.

use std::collections::VecDeque;

use serde::Serialize;

use super::marks::{Mark, MarkScanner, Piece};
use crate::session_log::transcript::{TranscriptFilter, ALT_ENTER_MARKER, ALT_EXIT_MARKER};

/// 세션당 보관할 블록 수.
pub const MAX_BLOCKS: usize = 40;
/// 블록당 보관할 출력 줄 수(넘으면 앞을 버리고 버린 수를 센다).
pub const MAX_LINES: usize = 600;
/// 웹이 보낸 명령을 셸이 실행 표식으로 알려 오기까지 기다리는 시간.
const PENDING_WEB_MS: u64 = 15_000;
/// 전사 필터가 가정할 화면 높이. 되감기 여유일 뿐이라 실제 크기와 달라도 된다.
const FILTER_ROWS: u16 = 50;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Phase {
    /// 아직 표식을 하나도 못 봤다(셸이 rc를 읽는 중이거나 지원하지 않는 셸).
    Unknown,
    Prompt,
    Running,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Origin {
    /// 웹 상태판에서 실행했다.
    Web,
    /// 앱 터미널에서 사람이 직접 쳤다.
    Terminal,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase", tag = "state")]
pub enum Explanation {
    Pending,
    Done { text: String, source: String },
    Failed { error: String },
    /// 쓸 수 있는 LLM이 없다.
    Off,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Block {
    pub id: u64,
    pub command: String,
    pub cwd: Option<String>,
    pub started_at: u64,
    pub ended_at: Option<u64>,
    /// 종료 코드. 끝 표식 없이 다음 명령·프롬프트가 오면 None.
    pub exit: Option<i32>,
    pub lines: VecDeque<String>,
    /// MAX_LINES를 넘어 버린 앞쪽 줄 수.
    pub dropped: usize,
    /// 전체 화면 앱(vim·top·less…)을 썼다.
    pub fullscreen: bool,
    pub origin: Origin,
    pub explanation: Option<Explanation>,
}

impl Block {
    pub fn finished(&self) -> bool {
        self.ended_at.is_some()
    }

    /// 설명 요청에 실을 출력: 앞 `head`줄과 뒤 `tail`줄.
    pub fn output_excerpt(&self, head: usize, tail: usize) -> String {
        let n = self.lines.len();
        if self.dropped == 0 && n <= head + tail {
            return self.lines.iter().cloned().collect::<Vec<_>>().join("\n");
        }
        // 앞줄을 이미 버렸으면 머리는 남아 있지 않다 — 꼬리만 싣는다.
        let head = if self.dropped == 0 { head } else { 0 };
        let tail = tail.min(n - head);
        let omitted = self.dropped + n - head - tail;
        let mut out: Vec<String> = self.lines.iter().take(head).cloned().collect();
        out.push(format!("…({omitted} lines omitted)…"));
        out.extend(self.lines.iter().skip(n - tail).cloned());
        out.join("\n")
    }
}

/// 기록기 이벤트. 호출자가 설명 요청 등 부수 효과를 건다.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Event {
    Finished(u64),
}

pub struct Recorder {
    pub session_id: String,
    /// 셸 종류("zsh"·"bash"·"pwsh"…). 세션을 이 앱이 띄웠을 때만 안다.
    pub shell: Option<String>,
    pub cwd: Option<String>,
    pub phase: Phase,
    pub blocks: VecDeque<Block>,
    next_id: u64,
    scanner: MarkScanner,
    filter: Option<TranscriptFilter>,
    pending_web: Option<(String, u64)>,
    /// 바뀔 때마다 오른다. 웹이 같은 내용을 다시 받지 않게 한다.
    pub rev: u64,
}

impl Recorder {
    pub fn new(session_id: &str, shell: Option<String>) -> Self {
        Self {
            session_id: session_id.to_string(),
            shell,
            cwd: None,
            phase: Phase::Unknown,
            blocks: VecDeque::new(),
            next_id: 1,
            scanner: MarkScanner::default(),
            filter: None,
            pending_web: None,
            rev: 1,
        }
    }

    /// 웹에서 실행할 명령을 적어 둔다. 셸이 명령 줄을 알려 주지 않아도 블록에 이 글을 쓴다.
    pub fn note_web_command(&mut self, command: &str, now: u64) {
        self.pending_web = Some((command.to_string(), now));
    }

    pub fn current(&self) -> Option<&Block> {
        self.blocks.back().filter(|b| !b.finished())
    }

    pub fn block_mut(&mut self, id: u64) -> Option<&mut Block> {
        self.blocks.iter_mut().find(|b| b.id == id)
    }

    /// 진행 중 블록의 아직 확정되지 않은 꼬리 줄.
    pub fn live_tail(&self) -> Vec<String> {
        match (&self.filter, self.current()) {
            (Some(f), Some(_)) if !f.in_alt_screen() => f.peek_pending(),
            _ => Vec::new(),
        }
    }

    pub fn feed(&mut self, bytes: &[u8], now: u64) -> Vec<Event> {
        let mut events = Vec::new();
        for piece in self.scanner.feed(bytes) {
            match piece {
                Piece::Data(data) => self.data(&data),
                Piece::Mark(Mark::Start { command }) => {
                    if let Some(e) = self.finish(None, now) {
                        events.push(e);
                    }
                    self.start(command, now);
                }
                Piece::Mark(Mark::Done { exit }) => {
                    if let Some(e) = self.finish(Some(exit), now) {
                        events.push(e);
                    }
                }
                Piece::Mark(Mark::Prompt { cwd }) => {
                    if let Some(e) = self.finish(None, now) {
                        events.push(e);
                    }
                    if !cwd.is_empty() {
                        self.cwd = Some(cwd);
                    }
                    if self.phase != Phase::Prompt {
                        self.phase = Phase::Prompt;
                    }
                    self.rev += 1;
                }
            }
        }
        events
    }

    fn data(&mut self, bytes: &[u8]) {
        let Some(filter) = self.filter.as_mut() else { return };
        let lines = filter.feed(bytes);
        let Some(block) = self.blocks.back_mut().filter(|b| !b.finished()) else { return };
        let had_output = !bytes.is_empty();
        push_lines(block, lines);
        if filter.in_alt_screen() {
            block.fullscreen = true;
        }
        if had_output {
            self.rev += 1;
        }
    }

    fn start(&mut self, command: String, now: u64) {
        let pending = self
            .pending_web
            .take()
            .filter(|(_, at)| now.saturating_sub(*at) <= PENDING_WEB_MS);
        let (command, origin) = match pending {
            Some((web, _)) if command.is_empty() || command == web.trim() => (web, Origin::Web),
            _ => (command, Origin::Terminal),
        };
        let id = self.next_id;
        self.next_id += 1;
        self.blocks.push_back(Block {
            id,
            command,
            cwd: self.cwd.clone(),
            started_at: now,
            ended_at: None,
            exit: None,
            lines: VecDeque::new(),
            dropped: 0,
            fullscreen: false,
            origin,
            explanation: None,
        });
        while self.blocks.len() > MAX_BLOCKS {
            self.blocks.pop_front();
        }
        self.filter = Some(TranscriptFilter::new(FILTER_ROWS));
        self.phase = Phase::Running;
        self.rev += 1;
    }

    /// 진행 중 블록을 닫는다. `exit`가 None이면 끝 표식 없이 닫힌 것(코드 모름).
    fn finish(&mut self, exit: Option<Option<i32>>, now: u64) -> Option<Event> {
        let mut filter = self.filter.take()?;
        let block = self.blocks.back_mut().filter(|b| !b.finished())?;
        let rest = filter.flush_all();
        push_lines(block, rest);
        while block.lines.back().is_some_and(|l| l.trim().is_empty()) {
            block.lines.pop_back();
        }
        block.exit = exit.flatten();
        block.ended_at = Some(now);
        self.phase = Phase::Prompt;
        self.rev += 1;
        Some(Event::Finished(block.id))
    }
}

fn push_lines(block: &mut Block, lines: Vec<String>) {
    for line in lines {
        if line == ALT_ENTER_MARKER || line == ALT_EXIT_MARKER {
            block.fullscreen = true;
            continue;
        }
        // 명령 줄 메아리 뒤의 첫 빈 줄들은 잡음이다.
        if block.lines.is_empty() && block.dropped == 0 && line.trim().is_empty() {
            continue;
        }
        block.lines.push_back(line);
        if block.lines.len() > MAX_LINES {
            block.lines.pop_front();
            block.dropped += 1;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(b: &Block) -> Vec<&str> {
        b.lines.iter().map(String::as_str).collect()
    }

    #[test]
    fn records_one_block_per_command_with_exit_code_and_cwd() {
        let mut r = Recorder::new("s1", Some("zsh".into()));
        assert_eq!(r.phase, Phase::Unknown);
        r.feed(b"\x1b]6973;P;/home/u\x07% ", 1);
        assert_eq!(r.phase, Phase::Prompt);
        assert_eq!(r.cwd.as_deref(), Some("/home/u"));
        // 프롬프트에서 친 글자 메아리는 기록하지 않는다.
        r.feed(b"ls\r\n", 2);
        r.feed(b"\x1b]6973;C;ls\x07a.txt\r\nb.txt\r\n", 3);
        assert_eq!(r.phase, Phase::Running);
        assert_eq!(r.current().unwrap().command, "ls");
        let ev = r.feed(b"\x1b]6973;D;0\x07\x1b]6973;P;/home/u\x07% ", 5);
        assert_eq!(ev, vec![Event::Finished(1)]);
        let b = r.blocks.back().unwrap();
        assert_eq!(lines(b), vec!["a.txt", "b.txt"]);
        assert_eq!(b.exit, Some(0));
        assert_eq!(b.cwd.as_deref(), Some("/home/u"));
        assert_eq!(b.ended_at, Some(5));
        assert_eq!(b.origin, Origin::Terminal);
        assert_eq!(r.phase, Phase::Prompt);
    }

    #[test]
    fn web_command_fills_missing_command_text_and_marks_origin() {
        let mut r = Recorder::new("s1", None);
        r.feed(b"\x1b]6973;P;/x\x07$ ", 1);
        r.note_web_command("git status", 2);
        r.feed(b"\x1b]6973;C\x07clean\r\n\x1b]6973;D;1\x07", 3);
        let b = r.blocks.back().unwrap();
        assert_eq!(b.command, "git status");
        assert_eq!(b.origin, Origin::Web);
        assert_eq!(b.exit, Some(1));
    }

    #[test]
    fn stale_web_command_is_not_attributed() {
        let mut r = Recorder::new("s1", None);
        r.note_web_command("git status", 0);
        r.feed(b"\x1b]6973;C;make\x07", PENDING_WEB_MS + 1);
        let b = r.current().unwrap();
        assert_eq!(b.command, "make");
        assert_eq!(b.origin, Origin::Terminal);
    }

    #[test]
    fn prompt_without_done_closes_block_with_unknown_exit() {
        let mut r = Recorder::new("s1", None);
        r.feed(b"\x1b]6973;C;sleep 1\x07zz\r\n", 1);
        let ev = r.feed(b"\x1b]6973;P;/y\x07", 2);
        assert_eq!(ev, vec![Event::Finished(1)]);
        assert_eq!(r.blocks.back().unwrap().exit, None);
        assert_eq!(r.cwd.as_deref(), Some("/y"));
    }

    #[test]
    fn progress_bar_redraws_collapse_and_live_tail_shows_uncommitted() {
        let mut r = Recorder::new("s1", None);
        r.feed(b"\x1b]6973;C;dl\x07 10%\r 50%\r 99%", 1);
        assert_eq!(r.live_tail(), vec![" 99%"]);
        r.feed(b"\r100%\r\ndone\r\n\x1b]6973;D;0\x07", 2);
        assert_eq!(lines(r.blocks.back().unwrap()), vec!["100%", "done"]);
        assert!(r.live_tail().is_empty());
    }

    #[test]
    fn fullscreen_app_is_flagged_not_transcribed() {
        let mut r = Recorder::new("s1", None);
        r.feed(b"\x1b]6973;C;top\x07\x1b[?1049hTOP SCREEN\x1b[?1049l\x1b]6973;D;0\x07", 1);
        let b = r.blocks.back().unwrap();
        assert!(b.fullscreen);
        assert!(lines(b).is_empty(), "{:?}", lines(b));
    }

    #[test]
    fn caps_lines_and_blocks() {
        let mut r = Recorder::new("s1", None);
        let mut out = b"\x1b]6973;C;yes\x07".to_vec();
        for i in 0..(MAX_LINES + 5) {
            out.extend(format!("line {i}\r\n").as_bytes());
        }
        out.extend(b"\x1b]6973;D;0\x07");
        r.feed(&out, 1);
        let b = r.blocks.back().unwrap();
        assert_eq!(b.lines.len(), MAX_LINES);
        assert_eq!(b.dropped, 5);
        assert_eq!(b.lines.back().unwrap(), &format!("line {}", MAX_LINES + 4));
        for i in 0..MAX_BLOCKS + 3 {
            r.feed(format!("\x1b]6973;C;c{i}\x07\x1b]6973;D;0\x07").as_bytes(), 2);
        }
        assert_eq!(r.blocks.len(), MAX_BLOCKS);
        assert_eq!(r.blocks.back().unwrap().command, format!("c{}", MAX_BLOCKS + 2));
    }

    #[test]
    fn excerpt_keeps_head_and_tail() {
        let mut r = Recorder::new("s1", None);
        let mut out = b"\x1b]6973;C;x\x07".to_vec();
        for i in 0..10 {
            out.extend(format!("l{i}\r\n").as_bytes());
        }
        out.extend(b"\x1b]6973;D;0\x07");
        r.feed(&out, 1);
        let b = r.blocks.back().unwrap();
        assert_eq!(b.output_excerpt(2, 2), "l0\nl1\n…(6 lines omitted)…\nl8\nl9");
        assert_eq!(b.output_excerpt(5, 5).lines().count(), 10);
    }
}
