// src-tauri/src/session/output_batcher.rs
//
// Pure PTY output batching logic. The 16ms deadline and the 64KB size cap
// are enforced by the output pump task (in `SessionManager`), which polls
// `pending_bytes()` and calls `flush()` once either threshold is hit; this
// struct only owns push/flush so it stays deterministic and testable
// without a timer.

use crate::types::OutputChunk;

pub const MAX_BYTES: usize = 65_536; // 64KB
pub const WINDOW_MS: u64 = 16; // ~60fps

/// Batch emission sink (test injection point). Production wraps a Tauri
/// `Channel`; tests use a `Vec`-recording fake.
pub trait FlushSink: Send + Sync {
    fn emit(&self, chunk: OutputChunk);
}

/// Pure batching logic. Timing is owned by the output pump task; this
/// struct only handles push/flush so it can be tested deterministically.
pub struct OutputBatcher {
    session_id: String,
    agent_id: String,
    buf: Vec<u8>,
    frames: u32,
    seq: u64,
}

impl OutputBatcher {
    pub fn new(session_id: String, agent_id: String) -> Self {
        Self { session_id, agent_id, buf: Vec::new(), frames: 0, seq: 0 }
    }

    pub fn pending_bytes(&self) -> usize {
        self.buf.len()
    }

    pub fn push(&mut self, bytes: &[u8]) {
        self.buf.extend_from_slice(bytes);
        self.frames += 1;
    }

    /// Emits on time-window/size-cap trigger. A trailing byte sequence cut
    /// at a UTF-8 boundary is carried over to the next flush.
    pub fn flush(&mut self, sink: &dyn FlushSink) {
        self.flush_inner(sink, false);
    }

    /// Forces emission of remaining bytes on exit/dispose (prevents losing
    /// the final output). An incomplete trailing sequence is lossily
    /// converted rather than held forever.
    pub fn flush_final(&mut self, sink: &dyn FlushSink) {
        self.flush_inner(sink, true);
    }

    fn flush_inner(&mut self, sink: &dyn FlushSink, final_: bool) {
        if self.buf.is_empty() {
            return;
        }

        // Walk the buffer, decoding valid UTF-8 runs and lossily replacing
        // *truly* invalid bytes (U+FFFD) so a single bad byte can't stall the
        // pipeline forever (the pre-fix bug: `valid_up_to()==0` parked a
        // truly-invalid lead byte at pos 0, take==0 every flush, buffer grew
        // unboundedly). An *incomplete* multibyte tail is still carried to the
        // next flush (or lossily emitted when `final_`).
        let mut out = String::new();
        let mut consumed = 0usize;
        loop {
            let rest = &self.buf[consumed..];
            if rest.is_empty() {
                break;
            }
            match std::str::from_utf8(rest) {
                Ok(s) => {
                    out.push_str(s);
                    consumed = self.buf.len();
                    break;
                }
                Err(e) => {
                    let valid = e.valid_up_to();
                    // Safe: [..valid] is a validated UTF-8 boundary.
                    out.push_str(std::str::from_utf8(&rest[..valid]).unwrap());
                    consumed += valid;
                    match e.error_len() {
                        // Some(n): `n` bytes are genuinely invalid (never the
                        // start of a valid sequence) -> emit a replacement char
                        // and keep going past them.
                        Some(n) => {
                            out.push('\u{FFFD}');
                            consumed += n;
                        }
                        // None: an incomplete multibyte tail. Carry it (wait for
                        // more bytes) unless this is the final flush.
                        None => {
                            if final_ {
                                out.push('\u{FFFD}');
                                consumed = self.buf.len();
                            }
                            break;
                        }
                    }
                }
            }
        }

        if consumed == 0 {
            // Nothing emittable yet: the buffer is a pure incomplete tail on a
            // non-final flush. Keep it and wait for more bytes.
            return;
        }
        self.buf.drain(..consumed);
        // §#49: raw 스트림 바이트 회계엔 UTF-8 문자열 길이(data.len())가 아니라
        // buf에서 실제로 drain한 raw 바이트 수(consumed)를 실어야 한다 — 잘못된
        // 바이트가 U+FFFD(3바이트)로 치환되면 둘이 어긋나고, 데몬 ring은 raw PTY
        // 바이트를 세므로 raw만 offset과 정합한다.
        self.emit(out, consumed, sink);
    }

    /// 스트림 바이트로 계수하지 않는 청크를 방출한다(§#49 함정 2): adopt 복원
    /// 스냅샷(화면 이미지)은 실제 스트림 바이트가 아니라 base가 이미 그 지점을
    /// 가리키므로, `bytes = 0`으로 실어 렌더러 누적에 안 잡히게 한다. seq 단조성은
    /// 이 경로도 batcher가 관리해 자연히 보존된다.
    pub fn emit_uncounted(&mut self, data: String, sink: &dyn FlushSink) {
        self.emit(data, 0, sink);
    }

    fn emit(&mut self, data: String, bytes: usize, sink: &dyn FlushSink) {
        let chunk = OutputChunk {
            session_id: self.session_id.clone(),
            agent_id: self.agent_id.clone(),
            data,
            frames: self.frames,
            seq: self.seq,
            bytes: bytes as u64,
        };
        self.seq += 1;
        self.frames = 0;
        sink.emit(chunk);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    #[derive(Default, Clone)]
    struct RecordingSink {
        chunks: Arc<Mutex<Vec<OutputChunk>>>,
    }

    impl RecordingSink {
        fn len(&self) -> usize {
            self.chunks.lock().unwrap().len()
        }

        fn at(&self, i: usize) -> OutputChunk {
            self.chunks.lock().unwrap()[i].clone()
        }
    }

    impl FlushSink for RecordingSink {
        fn emit(&self, chunk: OutputChunk) {
            self.chunks.lock().unwrap().push(chunk);
        }
    }

    fn batcher() -> OutputBatcher {
        OutputBatcher::new("s1".into(), "a1".into())
    }

    #[test]
    fn flush_concatenates_pushes_and_counts_frames() {
        let sink = RecordingSink::default();
        let mut b = batcher();
        b.push(b"abc");
        b.push(b"def");
        b.flush(&sink);

        assert_eq!(sink.len(), 1);
        let chunk = sink.at(0);
        assert_eq!(chunk.session_id, "s1");
        assert_eq!(chunk.agent_id, "a1");
        assert_eq!(chunk.data, "abcdef");
        assert_eq!(chunk.frames, 2);
        assert_eq!(chunk.seq, 0);
    }

    #[test]
    fn flush_on_empty_buffer_emits_nothing() {
        let sink = RecordingSink::default();
        let mut b = batcher();
        b.flush(&sink);
        assert_eq!(sink.len(), 0);
    }

    #[test]
    fn flush_resets_frames_and_pending_bytes() {
        let sink = RecordingSink::default();
        let mut b = batcher();
        b.push(b"hello");
        assert_eq!(b.pending_bytes(), 5);
        b.flush(&sink);
        assert_eq!(b.pending_bytes(), 0);

        // Frame count starts fresh for the next batch, not carried over.
        b.push(b"x");
        b.flush(&sink);
        assert_eq!(sink.at(1).frames, 1);
    }

    #[test]
    fn seq_increments_monotonically_per_flush() {
        let sink = RecordingSink::default();
        let mut b = batcher();
        for i in 0..3u8 {
            b.push(&[b'a' + i]);
            b.flush(&sink);
        }
        assert_eq!(sink.at(0).seq, 0);
        assert_eq!(sink.at(1).seq, 1);
        assert_eq!(sink.at(2).seq, 2);
    }

    #[test]
    fn pending_bytes_tracks_buffer_size_for_the_caller_size_cap_decision() {
        // OutputBatcher itself doesn't truncate at MAX_BYTES -- the output
        // pump task polls pending_bytes() and calls flush() once
        // it reaches MAX_BYTES. Verify the seam reports size accurately,
        // including past the cap when the caller hasn't flushed yet.
        let sink = RecordingSink::default();
        let mut b = batcher();
        let chunk_a = vec![b'x'; MAX_BYTES];
        let chunk_b = vec![b'y'; 10];
        b.push(&chunk_a);
        assert_eq!(b.pending_bytes(), MAX_BYTES);
        b.push(&chunk_b);
        assert_eq!(b.pending_bytes(), MAX_BYTES + 10);

        b.flush(&sink);
        assert_eq!(sink.len(), 1);
        assert_eq!(sink.at(0).data.len(), MAX_BYTES + 10);
        assert_eq!(sink.at(0).frames, 2);
    }

    #[test]
    fn multibyte_char_split_across_two_pushes_is_carried_and_reassembled() {
        // '한' = U+D55C = ED 95 9C in UTF-8.
        let sink = RecordingSink::default();
        let mut b = batcher();
        b.push(&[0xED, 0x95]); // incomplete lead bytes
        b.flush(&sink);
        assert_eq!(sink.len(), 0, "incomplete multibyte tail must not be emitted");
        assert_eq!(b.pending_bytes(), 2, "incomplete bytes stay carried in the buffer");

        b.push(&[0x9C]); // completes the character
        b.flush(&sink);
        assert_eq!(sink.len(), 1);
        assert_eq!(sink.at(0).data, "한");
        assert_eq!(sink.at(0).seq, 0);
    }

    #[test]
    fn multibyte_char_mixed_with_ascii_carries_only_the_incomplete_tail() {
        let sink = RecordingSink::default();
        let mut b = batcher();
        // "hi" + incomplete lead byte of '한'.
        b.push(&[b'h', b'i', 0xED]);
        b.flush(&sink);
        assert_eq!(sink.len(), 1);
        assert_eq!(sink.at(0).data, "hi");
        assert_eq!(b.pending_bytes(), 1, "the lone 0xED lead byte is carried");

        b.push(&[0x95, 0x9C]);
        b.flush(&sink);
        assert_eq!(sink.at(1).data, "한");
    }

    #[test]
    fn four_byte_emoji_split_across_three_pushes_is_reassembled() {
        // '🎉' = U+1F389 = F0 9F 8E 89 in UTF-8.
        let sink = RecordingSink::default();
        let mut b = batcher();
        b.push(&[0xF0]);
        b.flush(&sink);
        assert_eq!(sink.len(), 0);

        b.push(&[0x9F, 0x8E]);
        b.flush(&sink);
        assert_eq!(sink.len(), 0, "still incomplete after 3 of 4 bytes");

        b.push(&[0x89]);
        b.flush(&sink);
        assert_eq!(sink.len(), 1);
        assert_eq!(sink.at(0).data, "🎉");
    }

    #[test]
    fn flush_final_forces_emission_of_complete_trailing_bytes() {
        let sink = RecordingSink::default();
        let mut b = batcher();
        b.push(b"bye");
        b.flush_final(&sink);
        assert_eq!(sink.len(), 1);
        assert_eq!(sink.at(0).data, "bye");
        assert_eq!(b.pending_bytes(), 0);
    }

    #[test]
    fn flush_final_lossily_emits_an_incomplete_trailing_multibyte_sequence() {
        // Session exits mid-multibyte-char: flush_final must not drop the
        // trailing bytes silently -- it emits a lossy conversion instead of
        // waiting forever for bytes that will never arrive.
        let sink = RecordingSink::default();
        let mut b = batcher();
        b.push(&[b'o', b'k', 0xED, 0x95]); // "ok" + incomplete '한' lead
        b.flush_final(&sink);

        assert_eq!(sink.len(), 1);
        let data = sink.at(0).data;
        assert!(data.starts_with("ok"));
        assert!(data.contains('\u{FFFD}'), "incomplete tail becomes the replacement character");
        assert_eq!(b.pending_bytes(), 0, "flush_final must drain the entire buffer");
    }

    #[test]
    fn flush_final_on_empty_buffer_emits_nothing() {
        let sink = RecordingSink::default();
        let mut b = batcher();
        b.flush_final(&sink);
        assert_eq!(sink.len(), 0);
    }

    // ---- truly-invalid bytes must not stall the flush pipeline ----

    #[test]
    fn flush_lossily_emits_a_truly_invalid_byte_and_keeps_flowing() {
        // 0xFF is never a valid UTF-8 byte (error_len == Some(1)). The pre-fix
        // code parked it at pos 0 forever (take==0 each flush) and the buffer
        // grew unboundedly. It must instead be emitted as U+FFFD and drained.
        let sink = RecordingSink::default();
        let mut b = batcher();
        b.push(&[0xFF]);
        b.flush(&sink);

        assert_eq!(sink.len(), 1, "invalid byte must be flushed, not stalled");
        assert!(sink.at(0).data.contains('\u{FFFD}'));
        assert_eq!(b.pending_bytes(), 0, "invalid byte must be consumed, not parked forever");

        // Subsequent valid output still flows on the next flush.
        b.push(b"ok");
        b.flush(&sink);
        assert_eq!(sink.at(1).data, "ok");
    }

    #[test]
    fn flush_emits_valid_prefix_replacement_and_continues_in_one_chunk() {
        // "hi" + invalid 0xFF + "yo" -> a single chunk "hi<FFFD>yo".
        let sink = RecordingSink::default();
        let mut b = batcher();
        b.push(&[b'h', b'i', 0xFF, b'y', b'o']);
        b.flush(&sink);

        assert_eq!(sink.len(), 1);
        assert_eq!(sink.at(0).data, "hi\u{FFFD}yo");
        assert_eq!(b.pending_bytes(), 0);
    }

    #[test]
    fn flush_lossy_invalid_then_incomplete_tail_carries_only_the_tail() {
        // 'a' + invalid 0xFF + incomplete '한' lead (0xED,0x95): emit
        // "a<FFFD>" now, carry the 2-byte incomplete tail for reassembly.
        let sink = RecordingSink::default();
        let mut b = batcher();
        b.push(&[b'a', 0xFF, 0xED, 0x95]);
        b.flush(&sink);

        assert_eq!(sink.len(), 1);
        assert_eq!(sink.at(0).data, "a\u{FFFD}");
        assert_eq!(b.pending_bytes(), 2, "incomplete tail is carried, not lossily emitted");

        b.push(&[0x9C]); // completes '한'
        b.flush(&sink);
        assert_eq!(sink.at(1).data, "한");
    }

    #[test]
    fn repeated_invalid_bytes_never_grow_the_buffer_unboundedly() {
        // Regression for the stall/growth bug: feeding invalid bytes across
        // many flushes must always drain to zero pending bytes.
        let sink = RecordingSink::default();
        let mut b = batcher();
        for _ in 0..100 {
            b.push(&[0xFF, 0xFE]);
            b.flush(&sink);
            assert_eq!(b.pending_bytes(), 0, "buffer must never accumulate invalid bytes");
        }
        assert_eq!(sink.len(), 100);
    }

    // ---- §#49: raw 스트림 바이트 회계(bytes 필드) ----

    #[test]
    fn flush_reports_raw_consumed_bytes_not_utf8_len() {
        // 유효 바이트: bytes == data.len().
        let sink = RecordingSink::default();
        let mut b = batcher();
        b.push(b"abc");
        b.flush(&sink);
        let chunk = sink.at(0);
        assert_eq!(chunk.data, "abc");
        assert_eq!(chunk.bytes, chunk.data.len() as u64);
        assert_eq!(chunk.bytes, 3);

        // 잘못된 바이트(0xFF): raw consumed==1이지만 data는 U+FFFD(3바이트)라
        // bytes != data.len(). offset은 반드시 raw(consumed)를 따라야 한다.
        let mut b = batcher();
        b.push(&[0xFF]);
        b.flush(&sink);
        let chunk = sink.at(1);
        assert_eq!(chunk.data, "\u{FFFD}");
        assert_eq!(chunk.bytes, 1, "raw consumed 바이트는 1");
        assert_ne!(chunk.bytes, chunk.data.len() as u64, "U+FFFD는 3바이트라 어긋남");
    }

    #[test]
    fn flush_reports_raw_bytes_for_multibyte_char() {
        // '한' = ED 95 9C (3 raw bytes), UTF-8 문자열 길이도 3.
        let sink = RecordingSink::default();
        let mut b = batcher();
        b.push("한".as_bytes());
        b.flush(&sink);
        let chunk = sink.at(0);
        assert_eq!(chunk.data, "한");
        assert_eq!(chunk.bytes, 3);
    }

    #[test]
    fn carried_incomplete_tail_counts_in_the_flush_that_completes_it() {
        // 캐리된 불완전 tail은 그 flush의 bytes에 안 들어가고, 완성되는 flush에서
        // 잡혀 누계가 raw와 일치한다. '한' = ED 95 9C를 두 push로 쪼갠다.
        let sink = RecordingSink::default();
        let mut b = batcher();
        b.push(&[0xED, 0x95]); // 불완전 lead — 방출 없음
        b.flush(&sink);
        assert_eq!(sink.len(), 0);

        b.push(&[0x9C]); // 완성
        b.flush(&sink);
        assert_eq!(sink.len(), 1);
        let chunk = sink.at(0);
        assert_eq!(chunk.data, "한");
        // 첫 flush는 아무것도 안 실었고, 완성 flush가 raw 3바이트 전부를 계수.
        assert_eq!(chunk.bytes, 3);
    }

    #[test]
    fn emit_uncounted_carries_zero_bytes() {
        // 복원 스냅샷 주입 경로(§#49 C-3): bytes=0으로 방출해 offset 누적 제외.
        let sink = RecordingSink::default();
        let mut b = batcher();
        b.emit_uncounted("복원화면".to_string(), &sink);
        assert_eq!(sink.len(), 1);
        let chunk = sink.at(0);
        assert_eq!(chunk.data, "복원화면");
        assert_eq!(chunk.bytes, 0, "복원 청크는 스트림 바이트로 계수하지 않음");
        assert_eq!(chunk.seq, 0);

        // seq 단조성 보존: 이후 일반 flush는 seq 1부터.
        b.push(b"x");
        b.flush(&sink);
        assert_eq!(sink.at(1).seq, 1);
        assert_eq!(sink.at(1).bytes, 1);
    }
}
