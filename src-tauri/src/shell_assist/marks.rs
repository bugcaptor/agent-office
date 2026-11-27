// src-tauri/src/shell_assist/marks.rs
//
// 셸 캐릭터의 명령 경계 표식. 셸 rc에 넣은 훅(`scripts.rs`)이 프롬프트·명령 시작·
// 명령 끝마다 비공개 OSC `ESC ] 6973 ; <종류> ; <값> BEL`을 찍는다. xterm은 모르는
// OSC를 그냥 버리므로 화면에는 아무것도 보이지 않는다.
//
//   P;<cwd>   프롬프트를 그리기 직전(지금 작업 폴더)
//   C;<명령>  명령을 실행하기 직전(명령 줄, 비어 있을 수 있음)
//   D;<코드>  명령이 끝난 뒤(종료 코드)
//
// 이 모듈은 바이트 흐름에서 표식만 떼어 내고 나머지는 그대로 돌려준다(순수).
// 청크 경계에 걸친 표식도 이어서 읽는다.

const PREFIX: &[u8] = b"\x1b]6973;";
/// 표식 본문 상한. 넘으면 표식이 아닌 것으로 보고 버린다.
const MAX_PAYLOAD: usize = 8 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Mark {
    Prompt { cwd: String },
    Start { command: String },
    Done { exit: Option<i32> },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Piece {
    Data(Vec<u8>),
    Mark(Mark),
}

#[derive(Debug, Default)]
enum State {
    #[default]
    Ground,
    /// PREFIX 앞부분 n바이트가 맞았다.
    Prefix(usize),
    Payload,
    /// 본문 안에서 ESC를 봤다(다음이 `\`면 ST).
    PayloadEsc,
}

#[derive(Debug, Default)]
pub struct MarkScanner {
    state: State,
    payload: Vec<u8>,
}

impl MarkScanner {
    pub fn feed(&mut self, bytes: &[u8]) -> Vec<Piece> {
        let mut out = Vec::new();
        let mut data = Vec::with_capacity(bytes.len());
        for &b in bytes {
            self.step(b, &mut data, &mut out);
        }
        if !data.is_empty() {
            out.push(Piece::Data(data));
        }
        out
    }

    fn step(&mut self, b: u8, data: &mut Vec<u8>, out: &mut Vec<Piece>) {
        match self.state {
            State::Ground => {
                if b == PREFIX[0] {
                    self.state = State::Prefix(1);
                } else {
                    data.push(b);
                }
            }
            State::Prefix(n) => {
                if b == PREFIX[n] {
                    if n + 1 == PREFIX.len() {
                        self.state = State::Payload;
                        self.payload.clear();
                    } else {
                        self.state = State::Prefix(n + 1);
                    }
                } else {
                    // 표식이 아니었다. 먹어 둔 앞부분을 그대로 돌려주고 이 바이트를 다시 본다.
                    data.extend_from_slice(&PREFIX[..n]);
                    self.state = State::Ground;
                    self.step(b, data, out);
                }
            }
            State::Payload => match b {
                0x07 => self.finish(data, out),
                0x1b => self.state = State::PayloadEsc,
                _ => {
                    self.payload.push(b);
                    if self.payload.len() > MAX_PAYLOAD {
                        self.payload.clear();
                        self.state = State::Ground;
                    }
                }
            },
            State::PayloadEsc => {
                self.finish(data, out);
                if b != b'\\' {
                    self.step(b, data, out);
                }
            }
        }
    }

    fn finish(&mut self, data: &mut Vec<u8>, out: &mut Vec<Piece>) {
        self.state = State::Ground;
        let payload = std::mem::take(&mut self.payload);
        let Some(mark) = parse(&payload) else { return };
        if !data.is_empty() {
            out.push(Piece::Data(std::mem::take(data)));
        }
        out.push(Piece::Mark(mark));
    }
}

fn parse(payload: &[u8]) -> Option<Mark> {
    let text = String::from_utf8_lossy(payload);
    let (kind, value) = match text.split_once(';') {
        Some((k, v)) => (k, v),
        None => (text.as_ref(), ""),
    };
    match kind {
        "P" => Some(Mark::Prompt { cwd: value.to_string() }),
        "C" => Some(Mark::Start { command: value.trim().to_string() }),
        "D" => Some(Mark::Done { exit: value.trim().parse().ok() }),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn all(scanner: &mut MarkScanner, chunks: &[&[u8]]) -> Vec<Piece> {
        let mut out: Vec<Piece> = Vec::new();
        for c in chunks {
            for p in scanner.feed(c) {
                match (out.last_mut(), p) {
                    (Some(Piece::Data(prev)), Piece::Data(next)) => prev.extend(next),
                    (_, p) => out.push(p),
                }
            }
        }
        out
    }

    #[test]
    fn splits_marks_from_output_and_keeps_other_escapes() {
        let mut s = MarkScanner::default();
        let got = all(&mut s, &[b"\x1b]6973;C;ls -la\x07hi \x1b[31mred\x1b[0m\n\x1b]6973;D;2\x07\x1b]0;title\x07\x1b]6973;P;/tmp/a b\x1b\\$ "]);
        assert_eq!(
            got,
            vec![
                Piece::Mark(Mark::Start { command: "ls -la".into() }),
                Piece::Data(b"hi \x1b[31mred\x1b[0m\n".to_vec()),
                Piece::Mark(Mark::Done { exit: Some(2) }),
                Piece::Data(b"\x1b]0;title\x07".to_vec()),
                Piece::Mark(Mark::Prompt { cwd: "/tmp/a b".into() }),
                Piece::Data(b"$ ".to_vec()),
            ]
        );
    }

    #[test]
    fn mark_split_across_chunks_byte_by_byte() {
        let raw: &[u8] = b"out\x1b]6973;D;0\x07\x1b]6973;P;/x\x07";
        let chunks: Vec<&[u8]> = raw.chunks(1).collect();
        let mut s = MarkScanner::default();
        let got = all(&mut s, &chunks);
        assert_eq!(
            got,
            vec![
                Piece::Data(b"out".to_vec()),
                Piece::Mark(Mark::Done { exit: Some(0) }),
                Piece::Mark(Mark::Prompt { cwd: "/x".into() }),
            ]
        );
    }

    #[test]
    fn near_miss_prefix_is_passed_through() {
        let mut s = MarkScanner::default();
        let got = all(&mut s, &[b"\x1b]697", b"0;x\x07\x1b\x1b[0m"]);
        assert_eq!(got, vec![Piece::Data(b"\x1b]6970;x\x07\x1b\x1b[0m".to_vec())]);
    }

    #[test]
    fn unknown_kind_and_bad_exit_code() {
        let mut s = MarkScanner::default();
        let got = all(&mut s, &[b"\x1b]6973;Z;1\x07\x1b]6973;D;\x07\x1b]6973;C\x07"]);
        assert_eq!(
            got,
            vec![
                Piece::Mark(Mark::Done { exit: None }),
                Piece::Mark(Mark::Start { command: String::new() }),
            ]
        );
    }

    #[test]
    fn oversized_payload_is_dropped() {
        let mut s = MarkScanner::default();
        let mut raw = b"\x1b]6973;C;".to_vec();
        raw.extend(std::iter::repeat_n(b'a', MAX_PAYLOAD + 10));
        raw.extend_from_slice(b"\x07after");
        let got = all(&mut s, &[&raw]);
        // 본문이 상한을 넘은 순간 표식을 버리고 이후 바이트는 평범한 출력으로 본다.
        let Piece::Data(d) = &got[0] else { panic!("{got:?}") };
        assert!(d.ends_with(b"\x07after"));
        assert!(!got.iter().any(|p| matches!(p, Piece::Mark(_))));
    }
}
