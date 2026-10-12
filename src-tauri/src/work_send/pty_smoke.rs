//! 실제 PTY + CLI 대역 검증. 설치된 CLI나 인증 계정을 사용하지 않는다.
use super::*;
use crate::state::BotPromptArms;
use portable_pty::{native_pty_system, CommandBuilder, PtySize};
use std::{
    io::{Read, Write},
    os::unix::fs::PermissionsExt,
};
struct PtySink {
    writer: Mutex<Box<dyn Write + Send>>,
}
impl InputSink for PtySink {
    fn write(&self, _: &str, data: &str) {
        let mut writer = self.writer.lock().unwrap();
        writer.write_all(data.as_bytes()).unwrap();
        writer.flush().unwrap();
    }
    fn session_id_for(&self, _: &str) -> Option<String> {
        Some("work-pty".into())
    }
    fn is_running(&self, _: &str) -> bool {
        true
    }
    fn shell_path_for(&self, _: &str) -> Option<String> {
        Some("/bin/bash".into())
    }
    fn is_shell_prompt(&self, _: &str) -> Option<bool> {
        Some(true)
    }
}
struct Shell {
    child: Box<dyn portable_pty::Child + Send + Sync>,
    _master: Box<dyn portable_pty::MasterPty + Send>,
}
impl Drop for Shell {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[tokio::test]
#[ignore = "실제 PTY와 기본 10초 기동 대기를 검증하는 명시적 스모크 검사"]
async fn work_send_multiline_in_real_pty_with_cli_double() {
    let root = tempfile::tempdir().unwrap();
    let bin = root.path().join("bin");
    std::fs::create_dir(&bin).unwrap();
    let cli = bin.join("claude");
    std::fs::write(
        &cli,
        r#"#!/usr/bin/env python3
import os, sys, tty, termios, json
fd=sys.stdin.fileno()
old=termios.tcgetattr(fd)
try:
    tty.setraw(fd)
    os.write(sys.stdout.fileno(), b'WORK_CLI_READY\r\n')
    data=b''
    while not data.endswith(b'\x1b[201~\r'):
        byte=os.read(fd, 1)
        if not byte: sys.exit(1)
        data+=byte
    assert data.startswith(b'\x1b[200~'), repr(data)
    text=data[len(b'\x1b[200~'):-len(b'\x1b[201~\r')].decode('utf-8')
    with open('received.json', 'w') as f: json.dump({'text': text, 'argv': sys.argv[1:]}, f)
    while os.read(fd, 1): pass
finally:
    termios.tcsetattr(fd, termios.TCSANOW, old)
"#,
    )
    .unwrap();
    std::fs::set_permissions(&cli, std::fs::Permissions::from_mode(0o755)).unwrap();
    let pair = native_pty_system()
        .openpty(PtySize {
            rows: 24,
            cols: 100,
            pixel_width: 0,
            pixel_height: 0,
        })
        .unwrap();
    let mut command = CommandBuilder::new("/bin/bash");
    command.args(["--noprofile", "--norc"]);
    command.cwd(root.path());
    command.env(
        "PATH",
        format!(
            "{}:{}",
            bin.display(),
            std::env::var("PATH").unwrap_or_default()
        ),
    );
    command.env("PS1", "WORK_SHELL_READY> ");
    command.env("TERM", "xterm-256color");
    let child = pair.slave.spawn_command(command).unwrap();
    drop(pair.slave);
    let sink = Arc::new(PtySink {
        writer: Mutex::new(pair.master.take_writer().unwrap()),
    });
    let mut reader = pair.master.try_clone_reader().unwrap();
    let output = Arc::new(Mutex::new(String::new()));
    let collect = output.clone();
    std::thread::spawn(move || {
        let mut bytes = [0u8; 4096];
        while let Ok(n) = reader.read(&mut bytes) {
            if n == 0 {
                break;
            }
            collect
                .lock()
                .unwrap()
                .push_str(&String::from_utf8_lossy(&bytes[..n]));
        }
    });
    let _shell = Shell {
        child,
        _master: pair.master,
    };
    tokio::time::timeout(Duration::from_secs(5), async {
        while !output.lock().unwrap().contains("WORK_SHELL_READY>") {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    let gate = InjectGate::new(sink, Arc::new(BotPromptArms::default()));
    let runtime = WorkSendRuntime::default();
    let title = "첫 업무 🙂";
    let body = "첫 줄\n\n```text\n둘째 줄\n```";
    let request = WorkSendRequest {
        agent_id: "a".into(),
        session_id: "work-pty".into(),
        human_input_revision: 0,
        fresh_session: true,
        agent: WorkAgent {
            cli: WorkCli::Claude,
            model: Some("custom/model".into()),
            effort: Some("high".into()),
        },
        title: title.into(),
        body: body.into(),
    };
    let started = std::time::Instant::now();
    let result = runtime
        .send(&gate, root.path().into(), request)
        .await
        .unwrap();
    assert!(started.elapsed() >= Duration::from_secs(10));
    assert_eq!(result.session_id, "work-pty");
    let received = root.path().join("received.json");
    tokio::time::timeout(Duration::from_secs(3), async {
        while !received.exists() {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap();
    let value: serde_json::Value =
        serde_json::from_slice(&std::fs::read(received).unwrap()).unwrap();
    assert_eq!(value["text"], format!("{title}\n\n{body}"));
    assert_eq!(
        value["argv"],
        serde_json::json!(["--model", "custom/model", "--effort", "high"])
    );
    assert!(output.lock().unwrap().contains("WORK_CLI_READY"));
    assert!(runtime.inspect(&gate, "a").unwrap().restart_required);
}
