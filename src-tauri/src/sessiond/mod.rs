// src-tauri/src/sessiond/mod.rs
//
// 세션 핸드오프 데몬(unix 전용). 앱 종료 시 실행 중이던 PTY 세션들을
// 넘겨받아 존속시키고, 앱 재시작 시 되돌려준다. docs/session-handoff-design.md
// 참조. `main.rs`가 `--sessiond <socket_path>` 인자로 이 자신의 실행 파일을
// 재실행해 데몬 프로세스로 띄운다(기존 observer forwarder 분기와 동일 패턴).
#![cfg(unix)]

pub mod client;
pub mod daemon;
pub mod protocol;
