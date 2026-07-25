// 임시 스모크: 실제 PTY 캡처(raw)를 전사 필터에 통과시켜 눈으로 확인한다.
//   cargo run --example transcript_smoke -- <raw-file>
use agent_office_lib::session_log::transcript::TranscriptFilter;

fn main() {
    let path = std::env::args().nth(1).expect("raw file path");
    let bytes = std::fs::read(&path).unwrap();
    let mut f = TranscriptFilter::new(40);
    let mut lines = Vec::new();
    // 실제 스트림처럼 잘게 나눠 먹인다(청크 경계 견고성도 함께 본다).
    for chunk in bytes.chunks(37) {
        lines.extend(f.feed(chunk));
    }
    lines.extend(f.flush_all());
    println!("=== {path} ({} bytes) ===", bytes.len());
    for l in &lines {
        println!("| {l}");
    }
    println!("=== {} lines ===", lines.len());
}
