//! Windows ConPTY + CLI 대역 검증. 설치된 CLI나 인증 계정을 사용하지 않는다.
//! node 대역이 Claude Code처럼 bracketed paste를 켜고 raw 모드로 입력을 받는다.
use super::*;
use crate::state::BotPromptArms;
use portable_pty::{native_pty_system, CommandBuilder, PtySize};
use std::io::{Read, Write};
use std::path::Path;

struct PtySink {
    writer: Mutex<Box<dyn Write + Send>>,
    shell_path: String,
    pid: u32,
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
        Some(self.shell_path.clone())
    }
    fn is_shell_prompt(&self, _: &str) -> Option<bool> {
        crate::session::win_process::shell_is_idle(self.pid, &self.shell_path)
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

const CLI_DOUBLE: &str = r#"
const fs = require('fs');
const start = Buffer.from('\x1b[200~'), end = Buffer.from('\x1b[201~\r');
process.stdout.write('\x1b[?2004h');
process.stdin.setRawMode(true);
process.stdout.write('WORK_CLI_READY\r\n');
let data = Buffer.alloc(0), done = false;
process.stdin.on('data', (chunk) => {
  if (done) return;
  data = Buffer.concat([data, chunk]);
  if (data.length >= end.length && data.subarray(data.length - end.length).equals(end)) {
    done = true;
    const ok = data.subarray(0, start.length).equals(start);
    const text = data.subarray(start.length, data.length - end.length).toString('utf8');
    fs.writeFileSync('received.json', JSON.stringify({ ok, text, argv: process.argv.slice(2) }));
  }
});
"#;

fn programs() -> Vec<String> {
    let mut programs = vec![];
    if let Ok(root) = std::env::var("SystemRoot") {
        programs.push(format!(r"{root}\System32\WindowsPowerShell\v1.0\powershell.exe"));
    }
    if let Ok(pf) = std::env::var("ProgramFiles") {
        programs.push(format!(r"{pf}\PowerShell\7\pwsh.exe"));
        programs.push(format!(r"{pf}\Git\bin\bash.exe"));
    }
    programs.into_iter().filter(|p| Path::new(p).is_file()).collect()
}

fn shell_command(program: &str, root: &Path, cli: &Path) -> CommandBuilder {
    let mut command = CommandBuilder::new(program);
    let cli = cli.to_string_lossy();
    if program.to_ascii_lowercase().ends_with("bash.exe") {
        let rc = root.join("rc.sh");
        std::fs::write(
            &rc,
            format!(
                "claude() {{ node '{}' \"$@\"; }}\nPS1='WORK_SHELL_READY> '\n",
                cli.replace('\\', "/")
            ),
        )
        .unwrap();
        command.args(["--noprofile", "--rcfile"]);
        command.arg(rc.to_string_lossy().replace('\\', "/"));
        command.arg("-i");
    } else {
        command.args(["-NoLogo", "-NoProfile", "-NoExit", "-Command"]);
        command.arg(format!(
            "function global:claude {{ node '{cli}' @args }}; function global:prompt {{ 'WORK_SHELL_READY> ' }}"
        ));
    }
    command.cwd(root);
    command
}

async fn wait_until(what: &str, timeout: Duration, mut ready: impl FnMut() -> bool) {
    tokio::time::timeout(timeout, async {
        while !ready() {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .unwrap_or_else(|_| panic!("timed out waiting for {what}"));
}

#[tokio::test]
#[ignore = "실제 ConPTY와 기본 10초 기동 대기를 검증하는 명시적 스모크 검사"]
async fn work_send_multiline_in_real_conpty_with_cli_double() {
    for program in programs() {
        let root = tempfile::tempdir().unwrap();
        let cli = root.path().join("claude.js");
        std::fs::write(&cli, CLI_DOUBLE).unwrap();
        let pair = native_pty_system()
            .openpty(PtySize { rows: 24, cols: 100, pixel_width: 0, pixel_height: 0 })
            .unwrap();
        let child = pair
            .slave
            .spawn_command(shell_command(&program, root.path(), &cli))
            .unwrap();
        drop(pair.slave);
        let sink = Arc::new(PtySink {
            writer: Mutex::new(pair.master.take_writer().unwrap()),
            shell_path: program.clone(),
            pid: child.process_id().unwrap(),
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
                collect.lock().unwrap().push_str(&String::from_utf8_lossy(&bytes[..n]));
            }
        });
        let _shell = Shell { child, _master: pair.master };
        wait_until("shell prompt", Duration::from_secs(20), || {
            output.lock().unwrap().contains("WORK_SHELL_READY>")
        })
        .await;
        assert_eq!(sink.is_shell_prompt("a"), Some(true), "{program}: idle shell");

        let gate = InjectGate::new(sink.clone(), Arc::new(BotPromptArms::default()));
        let runtime = WorkSendRuntime::default();
        let title = "첫 업무 🙂";
        let body = "첫 줄\n\n```text\n둘째 줄\t탭\n```";
        let request = WorkSendRequest {
            agent_id: "a".into(),
            session_id: "work-pty".into(),
            human_input_revision: 0,
            // 기존 빈 셸 경로: 셸 자손 프로세스 조회로 프롬프트 대기를 확인한다.
            fresh_session: false,
            agent: WorkAgent {
                cli: WorkCli::Claude,
                model: Some("custom/model's".into()),
                effort: Some("high".into()),
            },
            title: title.into(),
            body: body.into(),
        };
        let started = std::time::Instant::now();
        let result = runtime
            .send(&gate, root.path().into(), request)
            .await
            .unwrap_or_else(|e| panic!("{program}: {e}\n{}", output.lock().unwrap()));
        assert!(started.elapsed() >= Duration::from_secs(10));
        assert_eq!(result.session_id, "work-pty");
        let received = root.path().join("received.json");
        wait_until("received.json", Duration::from_secs(5), || received.exists()).await;
        let value: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&received).unwrap()).unwrap();
        assert_eq!(value["ok"], true, "{program}");
        assert_eq!(value["text"], format!("{title}\n\n{body}"), "{program}");
        assert_eq!(
            value["argv"],
            serde_json::json!(["--model", "custom/model's", "--effort", "high"]),
            "{program}"
        );
        assert!(output.lock().unwrap().contains("WORK_CLI_READY"));
        assert_eq!(sink.is_shell_prompt("a"), Some(false), "{program}: CLI running");
        assert!(runtime.inspect(&gate, "a").unwrap().restart_required);
    }
}
