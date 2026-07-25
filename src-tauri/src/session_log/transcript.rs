// src-tauri/src/session_log/transcript.rs
//
// 원시 PTY 바이트 스트림 -> "읽을 수 있는 전사(transcript)" 필터.
// docs/session-log-design.md §3 이 정본.
//
// 왜 단순 strip-ANSI가 아닌가: Claude Code(Ink)·Codex 같은 TUI는 커서를 위로
// 올리고 지운 뒤 같은 자리를 다시 그린다. 이스케이프만 제거하면 스피너 프레임
// 하나하나가 별도 줄로 남아 로그가 수십 배로 부풀고 읽을 수 없게 된다.
//
// 그래서 아주 작은 터미널 모델을 둔다. `pending`은 "아직 덮어쓰일 수 있는"
// 라이브 영역이고, 거기서 밀려난 줄만 확정(commit)해 호출자에게 돌려준다.
// 되감기(CSI n A)는 화면 높이를 넘을 수 없으므로 화면 두 배보다 오래된 줄은
// 원리상 다시 쓰일 수 없다 -- 그것이 확정 기준의 근거다.
//
// 이 모듈은 파일을 모른다(순수 변환). 접기·시각 마커·파일 쓰기는 store.rs.

use std::collections::VecDeque;

/// 대체 화면(alternate screen) 진입/이탈에 남기는 마커. 그 안의 내용은
/// 기록하지 않지만 "여기서 전체 화면 앱을 썼다"는 사실은 남긴다.
pub const ALT_ENTER_MARKER: &str = "[전체 화면 앱 시작]";
pub const ALT_EXIT_MARKER: &str = "[전체 화면 앱 종료]";

/// 라이브 영역 하한 -- 화면이 아주 작게 보고돼도 이만큼은 되감기 여유를 둔다.
const MIN_LIVE_ROWS: usize = 80;
/// 커서 이동 파라미터 상한. 악의적/깨진 시퀀스가 거대한 할당을 유발하지 못하게.
const MAX_MOVE: usize = 4096;
/// 한 줄 길이 상한(문자). 넘으면 뒤를 버린다 -- `\r`만 반복하는 진행 표시줄이
/// 무한히 길어지는 것을 막는다.
const MAX_LINE_CHARS: usize = 8192;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    Ground,
    /// ESC 를 받은 직후.
    Esc,
    /// CSI(ESC [) 파라미터/중간 바이트 수집 중.
    Csi,
    /// OSC/DCS/APC/PM 등 문자열 시퀀스 -- ST(ESC \\) 또는 BEL 까지 버린다.
    Str,
    /// 문자열 시퀀스 안에서 ESC 를 봤다(다음이 `\\`면 종료).
    StrEsc,
}

pub struct TranscriptFilter {
    /// 아직 덮어쓰일 수 있는 줄들. 앞에서 밀려나면 확정된다.
    pending: VecDeque<Vec<char>>,
    row: usize,
    col: usize,
    /// 보고된 화면 높이. CUP(절대 좌표) 환산과 유휴 확정 여유에 쓴다.
    rows: usize,
    live_rows: usize,
    /// 대체 화면 안에서는 아무것도 기록하지 않는다.
    alt_screen: bool,
    saved_cursor: Option<(usize, usize)>,

    state: State,
    /// CSI 파라미터/중간 바이트 버퍼.
    seq: Vec<u8>,
    /// 미완성 UTF-8 시퀀스 캐리.
    utf8: Vec<u8>,
}

impl TranscriptFilter {
    pub fn new(rows: u16) -> Self {
        let rows = (rows as usize).max(1);
        Self {
            pending: VecDeque::new(),
            row: 0,
            col: 0,
            rows,
            live_rows: (rows * 2).max(MIN_LIVE_ROWS),
            alt_screen: false,
            saved_cursor: None,
            state: State::Ground,
            seq: Vec::new(),
            utf8: Vec::new(),
        }
    }

    /// 터미널 크기 변경 반영. 라이브 영역은 **줄이지 않는다** -- 줄이면 아직
    /// 되감기 대상일 수 있는 줄이 확정돼 버린다.
    pub fn set_rows(&mut self, rows: u16) {
        let rows = (rows as usize).max(1);
        self.rows = rows;
        self.live_rows = self.live_rows.max((rows * 2).max(MIN_LIVE_ROWS));
    }

    /// 바이트 덩어리를 먹이고, 이번에 확정된 줄들을 돌려준다.
    pub fn feed(&mut self, bytes: &[u8]) -> Vec<String> {
        let mut out = Vec::new();
        for &b in bytes {
            self.step(b, &mut out);
        }
        self.drain_overflow(&mut out);
        out
    }

    /// 유휴 확정: 커서 위로 화면 한 장 분량만 남기고 나머지를 확정한다.
    /// 출력이 멎은 세션의 마지막 대화가 파일에 닿게 하는 경로다.
    pub fn flush_idle(&mut self) -> Vec<String> {
        let keep = self.rows.max(1);
        let mut out = Vec::new();
        while self.pending.len() > keep {
            self.pop_front_into(&mut out);
        }
        out
    }

    /// 전량 확정(세션 종료·앱 종료·화면 전체 지우기).
    pub fn flush_all(&mut self) -> Vec<String> {
        let mut out = Vec::new();
        // 커서가 마지막 줄 위에 앉아 있으면 빈 줄이 하나 매달려 있다. 확정
        // 경계에서 그런 꼬리 공백 줄은 잡음이라 버린다.
        while self
            .pending
            .back()
            .is_some_and(|line| line.iter().all(|c| *c == ' '))
        {
            self.pending.pop_back();
        }
        while !self.pending.is_empty() {
            self.pop_front_into(&mut out);
        }
        self.row = 0;
        self.col = 0;
        out
    }

    // ---- 내부 ----

    fn step(&mut self, b: u8, out: &mut Vec<String>) {
        match self.state {
            State::Ground => self.ground(b, out),
            State::Esc => {
                self.seq.clear();
                match b {
                    b'[' => self.state = State::Csi,
                    // OSC/DCS/SOS/PM/APC -- 전부 문자열 시퀀스로 취급해 버린다.
                    b']' | b'P' | b'X' | b'^' | b'_' => self.state = State::Str,
                    // ESC 7/8: 커서 저장/복원.
                    b'7' => {
                        self.saved_cursor = Some((self.row, self.col));
                        self.state = State::Ground;
                    }
                    b'8' => {
                        self.restore_cursor();
                        self.state = State::Ground;
                    }
                    // ESC M: 역방향 개행(한 줄 위로).
                    b'M' => {
                        self.row = self.row.saturating_sub(1);
                        self.state = State::Ground;
                    }
                    // 그 외 2바이트 ESC 시퀀스(문자셋 지정 등)는 버린다.
                    _ => self.state = State::Ground,
                }
            }
            State::Csi => {
                // 파라미터(0x30-0x3F)와 중간 바이트(0x20-0x2F)를 모으고,
                // 최종 바이트(0x40-0x7E)에서 실행한다.
                if (0x20..=0x3f).contains(&b) {
                    if self.seq.len() < 64 {
                        self.seq.push(b);
                    }
                } else if (0x40..=0x7e).contains(&b) {
                    let seq = std::mem::take(&mut self.seq);
                    self.csi(&seq, b, out);
                    self.state = State::Ground;
                } else {
                    // 시퀀스 중간의 제어문자는 그대로 처리하고 CSI는 계속.
                    if b == 0x1b {
                        self.state = State::Esc;
                    }
                }
            }
            State::Str => match b {
                0x07 => self.state = State::Ground, // BEL 종료
                0x1b => self.state = State::StrEsc,
                _ => {}
            },
            State::StrEsc => {
                // ESC \\ 가 ST. 그 외면 새 ESC 시퀀스의 시작으로 본다.
                self.state = if b == b'\\' { State::Ground } else { State::Esc };
            }
        }
    }

    fn ground(&mut self, b: u8, out: &mut Vec<String>) {
        // UTF-8 연속 바이트 조립이 진행 중이면 그것부터.
        if !self.utf8.is_empty() {
            if (0x80..0xc0).contains(&b) {
                self.utf8.push(b);
                self.try_decode();
                return;
            }
            // 깨진 시퀀스 -- 버리고 이 바이트를 새로 해석한다.
            self.utf8.clear();
        }

        match b {
            0x1b => self.state = State::Esc,
            b'\n' => {
                self.row += 1;
                self.ensure_row();
                self.drain_overflow(out);
            }
            b'\r' => self.col = 0,
            0x08 => self.col = self.col.saturating_sub(1),
            b'\t' => self.col = (self.col / 8 + 1) * 8,
            0x07 => {} // BEL -- 알림은 다른 경로가 본다
            0x0b | 0x0c => {
                // VT/FF: 한 줄 아래로.
                self.row += 1;
                self.ensure_row();
                self.drain_overflow(out);
            }
            0x00..=0x1f | 0x7f => {} // 그 밖의 제어문자는 버린다
            0x20..=0x7e => self.put(b as char),
            _ => {
                // UTF-8 선두 바이트.
                self.utf8.push(b);
                self.try_decode();
            }
        }
    }

    fn try_decode(&mut self) {
        // 완성되면 문자로, 4바이트를 넘겨도 못 읽으면 버린다.
        match std::str::from_utf8(&self.utf8) {
            Ok(s) => {
                let chars: Vec<char> = s.chars().collect();
                self.utf8.clear();
                for ch in chars {
                    self.put(ch);
                }
            }
            Err(_) if self.utf8.len() >= 4 => self.utf8.clear(),
            Err(_) => {} // 아직 미완성 -- 다음 바이트를 기다린다
        }
    }

    fn csi(&mut self, seq: &[u8], final_byte: u8, out: &mut Vec<String>) {
        // private 마커(`?`, `>`, `<`, `=`)가 붙은 시퀀스는 모드 설정이다.
        let private = seq.first().is_some_and(|c| !c.is_ascii_digit() && *c != b';');
        let digits = if private { &seq[1..] } else { seq };
        let params: Vec<usize> = std::str::from_utf8(digits)
            .unwrap_or("")
            .split(';')
            .map(|p| p.trim().parse::<usize>().unwrap_or(0))
            .collect();
        let p0 = params.first().copied().unwrap_or(0);
        let n = p0.max(1).min(MAX_MOVE);

        if private {
            // 대체 화면 전환만 본다(1049/1047/47). 나머지 모드는 무시.
            let alt = matches!(p0, 47 | 1047 | 1049);
            if alt && final_byte == b'h' && !self.alt_screen {
                out.extend(self.flush_all());
                out.push(ALT_ENTER_MARKER.to_string());
                self.alt_screen = true;
            } else if alt && final_byte == b'l' && self.alt_screen {
                self.alt_screen = false;
                self.pending.clear();
                self.row = 0;
                self.col = 0;
                out.push(ALT_EXIT_MARKER.to_string());
            }
            return;
        }

        if self.alt_screen {
            return; // 대체 화면 안의 그리기는 전부 무시
        }

        match final_byte {
            b'A' => self.row = self.row.saturating_sub(n), // CUU
            b'B' | b'e' => {
                // CUD
                self.row = self.row.saturating_add(n).min(self.row + MAX_MOVE);
                self.ensure_row();
                self.drain_overflow(out);
            }
            b'C' | b'a' => self.col = (self.col + n).min(MAX_LINE_CHARS), // CUF
            b'D' => self.col = self.col.saturating_sub(n),                // CUB
            b'E' => {
                // CNL: n줄 아래 + 행 처음
                self.row += n;
                self.col = 0;
                self.ensure_row();
                self.drain_overflow(out);
            }
            b'F' => {
                // CPL: n줄 위 + 행 처음
                self.row = self.row.saturating_sub(n);
                self.col = 0;
            }
            b'G' | b'`' => self.col = n - 1, // CHA
            b'd' => self.set_abs_row(n),     // VPA
            b'H' | b'f' => {
                // CUP: 화면 절대 좌표 -> 라이브 영역 하단 기준으로 환산.
                let c = params.get(1).copied().unwrap_or(0).max(1);
                self.set_abs_row(n);
                self.col = c - 1;
            }
            b'J' => self.erase_display(p0, out),
            b'K' => self.erase_line(p0),
            b'L' => {
                // IL: 커서 위치에 빈 줄 n개 삽입
                self.ensure_row();
                for _ in 0..n.min(self.live_rows) {
                    self.pending.insert(self.row, Vec::new());
                }
                self.drain_overflow(out);
            }
            b'M' => {
                // DL: 커서 줄부터 n줄 삭제
                for _ in 0..n {
                    if self.row < self.pending.len() {
                        self.pending.remove(self.row);
                    }
                }
            }
            b'P' => {
                // DCH: 커서 위치 문자 n개 삭제
                if let Some(line) = self.pending.get_mut(self.row) {
                    for _ in 0..n {
                        if self.col < line.len() {
                            line.remove(self.col);
                        }
                    }
                }
            }
            b'X' => {
                // ECH: 커서 위치부터 n개 공백으로
                let col = self.col;
                if let Some(line) = self.pending.get_mut(self.row) {
                    for i in col..(col + n).min(MAX_LINE_CHARS) {
                        if i < line.len() {
                            line[i] = ' ';
                        }
                    }
                }
            }
            b'@' => {
                // ICH: 커서 위치에 공백 n개 삽입
                let col = self.col;
                if let Some(line) = self.pending.get_mut(self.row) {
                    while line.len() < col {
                        line.push(' ');
                    }
                    for _ in 0..n.min(MAX_LINE_CHARS) {
                        line.insert(col, ' ');
                    }
                }
            }
            b's' => self.saved_cursor = Some((self.row, self.col)),
            b'u' => self.restore_cursor(),
            // SGR(m)·스크롤 영역(r)·장치 질의 등은 전사에 영향이 없다.
            _ => {}
        }
    }

    /// 화면 절대 행(1-based)을 라이브 영역 인덱스로 환산한다. 화면 하단이
    /// `pending`의 끝이라고 보고 거기서 거슬러 올라간다.
    fn set_abs_row(&mut self, row_1based: usize) {
        let base = self.pending.len().saturating_sub(self.rows);
        self.row = base + (row_1based - 1);
        self.ensure_row();
    }

    fn restore_cursor(&mut self) {
        if let Some((r, c)) = self.saved_cursor {
            self.row = r.min(self.pending.len().saturating_sub(1));
            self.col = c;
        }
    }

    fn erase_line(&mut self, mode: usize) {
        let col = self.col;
        let Some(line) = self.pending.get_mut(self.row) else {
            return;
        };
        match mode {
            // 커서부터 줄 끝까지
            0 => line.truncate(col),
            // 줄 처음부터 커서까지(공백으로)
            1 => {
                for i in 0..=col.min(line.len().saturating_sub(1)) {
                    line[i] = ' ';
                }
            }
            _ => line.clear(),
        }
    }

    fn erase_display(&mut self, mode: usize, out: &mut Vec<String>) {
        match mode {
            // 커서 아래 전부 -- Ink 재그리기의 핵심 경로다.
            0 => {
                self.erase_line(0);
                while self.pending.len() > self.row + 1 {
                    self.pending.pop_back();
                }
            }
            // 커서 위 전부(공백화). 확정된 내용은 이미 파일에 있으니 여기만 비운다.
            1 => {
                for r in 0..self.row.min(self.pending.len()) {
                    self.pending[r].clear();
                }
                self.erase_line(1);
            }
            // 화면 전체 지우기: 지금까지를 확정하고 라이브 영역을 비운다.
            // (화면에서 사라졌다고 로그에서까지 사라질 이유는 없다.)
            _ => {
                out.extend(self.flush_all());
            }
        }
    }

    fn put(&mut self, ch: char) {
        if self.alt_screen {
            return;
        }
        self.ensure_row();
        let col = self.col;
        let line = &mut self.pending[self.row];
        if col >= MAX_LINE_CHARS {
            return;
        }
        while line.len() < col {
            line.push(' ');
        }
        if col < line.len() {
            line[col] = ch;
        } else {
            line.push(ch);
        }
        self.col = col + 1;
    }

    fn ensure_row(&mut self) {
        // 커서가 라이브 영역 밖으로 나가면 그만큼 줄을 만든다. row가 터무니없이
        // 크면(깨진 CUD) 상한으로 클램프한다.
        if self.row > self.pending.len() + MAX_MOVE {
            self.row = self.pending.len();
        }
        while self.pending.len() <= self.row {
            self.pending.push_back(Vec::new());
        }
    }

    /// 라이브 영역 상한을 넘은 만큼 앞에서 확정한다.
    fn drain_overflow(&mut self, out: &mut Vec<String>) {
        while self.pending.len() > self.live_rows {
            self.pop_front_into(out);
        }
    }

    fn pop_front_into(&mut self, out: &mut Vec<String>) {
        let Some(line) = self.pending.pop_front() else {
            return;
        };
        self.row = self.row.saturating_sub(1);
        let text: String = line.into_iter().collect();
        out.push(text.trim_end().to_string());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 편의: 먹인 뒤 전량 확정까지 합쳐 한 문자열로.
    fn transcribe(rows: u16, chunks: &[&str]) -> String {
        let mut f = TranscriptFilter::new(rows);
        let mut lines = Vec::new();
        for c in chunks {
            lines.extend(f.feed(c.as_bytes()));
        }
        lines.extend(f.flush_all());
        lines.join("\n")
    }

    #[test]
    fn plain_lines_pass_through() {
        assert_eq!(transcribe(24, &["hello\r\nworld\r\n"]), "hello\nworld");
    }

    #[test]
    fn sgr_colors_are_dropped() {
        assert_eq!(transcribe(24, &["\x1b[31mred\x1b[0m text\r\n"]), "red text");
    }

    #[test]
    fn carriage_return_overwrites_in_place() {
        // 진행 표시줄: 같은 줄을 \r로 덮어쓴다 -> 마지막 상태만 남아야 한다.
        assert_eq!(transcribe(24, &["10%\r50%\r100%\r\n"]), "100%");
    }

    #[test]
    fn erase_line_truncates_at_cursor() {
        assert_eq!(transcribe(24, &["abcdef\r\x1b[Kxy\r\n"]), "xy");
    }

    #[test]
    fn ink_style_redraw_keeps_only_final_frame() {
        // Ink 관례: 커서를 프레임 높이만큼 올리고 아래를 지운 뒤 다시 그린다.
        let out = transcribe(
            24,
            &[
                "✻ Thinking…\r\nline2\r\n",
                "\x1b[2A\x1b[0J", // 2줄 위로 + 아래 전부 지우기
                "✻ Done\r\nline2b\r\n",
            ],
        );
        assert_eq!(out, "✻ Done\nline2b");
    }

    #[test]
    fn cursor_up_then_overwrite_replaces_that_line() {
        let out = transcribe(24, &["one\r\ntwo\r\n", "\x1b[2A\rONE\r\n"]);
        assert_eq!(out, "ONE\ntwo");
    }

    #[test]
    fn alt_screen_content_is_replaced_by_markers() {
        let out = transcribe(
            24,
            &[
                "before\r\n",
                "\x1b[?1049h",
                "VIM SCREEN GARBAGE\x1b[5;5Hmore",
                "\x1b[?1049l",
                "after\r\n",
            ],
        );
        assert_eq!(
            out,
            format!("before\n{ALT_ENTER_MARKER}\n{ALT_EXIT_MARKER}\nafter")
        );
    }

    #[test]
    fn osc_title_sequence_is_dropped() {
        assert_eq!(transcribe(24, &["\x1b]0;my title\x07ok\r\n"]), "ok");
        assert_eq!(transcribe(24, &["\x1b]0;t\x1b\\ok\r\n"]), "ok");
    }

    #[test]
    fn utf8_split_across_chunks_is_reassembled() {
        let bytes = "한글".as_bytes();
        let mut f = TranscriptFilter::new(24);
        let mut lines = Vec::new();
        lines.extend(f.feed(&bytes[..3]));
        lines.extend(f.feed(&bytes[3..]));
        lines.extend(f.feed(b"\r\n"));
        lines.extend(f.flush_all());
        assert_eq!(lines.join("\n"), "한글");
    }

    #[test]
    fn overflow_commits_oldest_lines_only() {
        let mut f = TranscriptFilter::new(1); // live_rows = MIN_LIVE_ROWS(80)
        let mut committed = Vec::new();
        for i in 0..100 {
            committed.extend(f.feed(format!("line{i}\r\n").as_bytes()));
        }
        // 100줄을 썼으면 80줄 라이브 영역을 넘긴 만큼은 이미 확정돼 있어야 한다.
        assert!(!committed.is_empty(), "확정된 줄이 없다");
        assert_eq!(committed[0], "line0");
        // 확정 + 잔여를 합치면 전부 순서대로 나온다.
        committed.extend(f.flush_all());
        let joined = committed.join("\n");
        assert!(joined.starts_with("line0\nline1"), "{joined}");
        assert!(joined.contains("line99"), "{joined}");
    }

    #[test]
    fn flush_idle_keeps_one_screen() {
        let mut f = TranscriptFilter::new(10);
        f.feed(b"a\r\nb\r\nc\r\nd\r\n");
        let out = f.flush_idle();
        // 10줄(화면 한 장) 이하이므로 아무것도 확정하지 않는다.
        assert!(out.is_empty(), "{out:?}");

        // 화면 2줄이면 끝에서 2줄만 남기고 확정한다(되감기는 화면 높이를
        // 넘을 수 없으므로 그 위는 다시 쓰일 수 없다). 커서가 앉은 빈 줄이
        // 마지막 한 줄을 차지한다.
        let mut f = TranscriptFilter::new(2);
        f.feed(b"a\r\nb\r\nc\r\nd\r\n");
        let out = f.flush_idle();
        assert_eq!(out, vec!["a", "b", "c"]);
    }

    #[test]
    fn trailing_whitespace_is_trimmed() {
        assert_eq!(transcribe(24, &["text     \r\n"]), "text");
    }

    #[test]
    fn absolute_cursor_position_lands_in_live_region() {
        // 화면 3줄. CUP(1;1)은 라이브 영역의 화면 시작 줄을 가리켜야 한다.
        let out = transcribe(3, &["a\r\nb\r\nc\r\n", "\x1b[1;1HX\r\n"]);
        assert_eq!(out, "a\nX\nc");
    }

    #[test]
    fn erase_display_all_commits_instead_of_losing() {
        // clear: 화면에서는 사라지지만 로그에는 남아야 한다.
        let out = transcribe(24, &["important\r\n", "\x1b[2J\x1b[H", "next\r\n"]);
        assert_eq!(out, "important\nnext");
    }

    #[test]
    fn broken_escape_does_not_panic_or_leak() {
        let out = transcribe(24, &["\x1b[999999999;9999999Hx\r\n", "ok\r\n"]);
        assert!(out.contains("ok"), "{out}");
    }

    #[test]
    fn insert_and_delete_line_shift_pending() {
        // DL: 커서 줄 삭제
        let out = transcribe(24, &["a\r\nb\r\nc\r\n", "\x1b[3A\x1b[M"]);
        assert_eq!(out, "b\nc");
    }

    #[test]
    fn backspace_and_tab_move_the_cursor() {
        assert_eq!(transcribe(24, &["abc\x08X\r\n"]), "abX");
        assert_eq!(transcribe(24, &["a\tb\r\n"]), "a       b");
    }
}
