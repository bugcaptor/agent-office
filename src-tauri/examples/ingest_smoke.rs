// 임시 스모크: 실제 에이전트 JSONL 전사를 렌더러에 통과시켜 눈으로 확인한다.
//   cargo run --example ingest_smoke -- claude <~/.claude/projects/…/<sid>.jsonl>
//   cargo run --example ingest_smoke -- codex  <~/.codex/sessions/…/rollout-….jsonl>
use std::path::PathBuf;
use std::sync::Arc;

use agent_office_lib::session_log::agent_transcript::{
    claude::ClaudeSource, codex::CodexSource, AgentSessionLookup, TranscriptSource,
    TranscriptTailer,
};

struct NoLookup;
impl AgentSessionLookup for NoLookup {
    fn latest_session(&self, _agent_id: &str) -> Option<(String, Option<String>)> {
        None
    }
}

fn main() {
    let kind = std::env::args().nth(1).expect("claude|codex|locate");
    // locate 모드: 실제 claude-resume.json + ~/.claude/projects 로 경로 해석만
    // 확인한다. `locate <claude-resume.json> <projects-root> <agentId> <cwd>`
    if kind == "locate" {
        let args: Vec<String> = std::env::args().skip(2).collect();
        let store = Arc::new(
            agent_office_lib::persistence::claude_resume_store::ClaudeResumeStore::new(
                PathBuf::from(&args[0]),
            ),
        );
        let mut source = ClaudeSource::new(PathBuf::from(&args[1]), store);
        println!("{:?}", source.locate(&args[2], &args[3]));
        return;
    }
    // watch 모드: 실제 스토어·전사로 수집기를 돌려 새 대화가 흘러오는지 본다.
    // `watch <claude-resume.json> <projects-root> <agentId> <cwd> <초>`
    if kind == "watch" {
        let args: Vec<String> = std::env::args().skip(2).collect();
        let store = Arc::new(
            agent_office_lib::persistence::claude_resume_store::ClaudeResumeStore::new(
                PathBuf::from(&args[0]),
            ),
        );
        let source = Box::new(ClaudeSource::new(PathBuf::from(&args[1]), store));
        let mut tailer = TranscriptTailer::new(&args[2], &args[3], vec![source]);
        let secs: u64 = args[4].parse().unwrap_or(20);
        for _ in 0..secs / 2 {
            for line in tailer.tick() {
                println!("| {line}");
            }
            std::thread::sleep(std::time::Duration::from_secs(2));
        }
        println!("=== watch 끝 ===");
        return;
    }
    let path = std::env::args().nth(2).expect("jsonl path");
    let source: Box<dyn TranscriptSource> = match kind.as_str() {
        "claude" => Box::new(ClaudeSource::new(PathBuf::from("/nowhere"), Arc::new(NoLookup))),
        "codex" => Box::new(CodexSource::new(PathBuf::from("/nowhere"))),
        other => panic!("알 수 없는 종류: {other}"),
    };
    let body = std::fs::read_to_string(&path).unwrap();
    let mut lines = 0usize;
    for raw in body.lines() {
        for out in source.render(raw) {
            println!("| {out}");
            lines += 1;
        }
    }
    println!("=== {path}: {} 항목 → {lines} 줄 ===", body.lines().count());
}
