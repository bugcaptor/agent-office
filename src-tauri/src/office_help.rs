//! `office-help` shim(`agent-office help`): 세션 안 에이전트가 지금 켜진 사무실 기능과 쓸 명령을 한 번에 본다.
//! 세션 안내문은 띄울 때 정해지지만, 이 명령은 실행 시점의 설정(업무 패널·머신 연동·에이전트끼리 대화)을
//! 읽어 같은 목차를 다시 찍고, 에이전트끼리 대화가 켜져 있으면 머신·캐릭터 상태(`office-peer agents`)를 붙인다.
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Duration;

const USAGE: &str = "usage: office-help
Shows the Agent Office commands this session can use right now, which features are on, and (with
agent talk on) every linked machine and character with its state.
";

pub fn shim_path() -> PathBuf {
    crate::work_inbox_cli::office_shim_path("office-help")
}

pub fn run(args: Vec<String>) -> i32 {
    let var = |name| std::env::var_os(name).filter(|v| !v.is_empty());
    let data = var("AGENT_OFFICE_APP_DATA").map(PathBuf::from).or_else(crate::control::client::default_app_data);
    let env = crate::peer::cli::PeerEnv {
        app_data: data.clone(),
        agent_id: var("AGENT_OFFICE_AGENT_ID").map(|v| v.to_string_lossy().into_owned()),
        in_session: var("AGENT_OFFICE_SESSION").is_some(),
    };
    run_with(&args, data.as_deref(), &env, &mut std::io::stdout(), &mut std::io::stderr())
}

/// 이 머신에서 지금 켜진 기능.
#[derive(Debug, PartialEq, Eq)]
struct Features {
    task: bool,
    file: bool,
    peer: bool,
}

fn features(data: &Path) -> Features {
    let task = std::fs::read(data.join("settings.json"))
        .ok()
        .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
        .and_then(|v| v["workPanelsEnabled"].as_bool())
        .unwrap_or(false);
    Features { task, file: crate::files::cli::instructions_wanted(data), peer: crate::peer::instructions_wanted(data) }
}

fn run_with(args: &[String], data: Option<&Path>, env: &crate::peer::cli::PeerEnv, out: &mut dyn Write, err: &mut dyn Write) -> i32 {
    if args.iter().any(|a| matches!(a.as_str(), "-h" | "--help" | "help")) {
        let _ = write!(out, "{USAGE}");
        return 0;
    }
    if let Some(extra) = args.first() {
        let _ = writeln!(err, "office-help: unexpected argument {extra:?}\n{USAGE}");
        return 2;
    }
    let Some(data) = data else {
        let _ = writeln!(err, "office-help: Agent Office data folder not found (AGENT_OFFICE_APP_DATA is not set)");
        return 1;
    };
    let f = features(data);
    let (task, md, file, peer, help) = (
        crate::work_inbox_cli::shim_path(),
        crate::doc_view_cli::shim_path(),
        crate::files::cli::shim_path(),
        crate::peer::cli::shim_path(),
        shim_path(),
    );
    let guide = crate::office_instructions::guide(&crate::office_instructions::Guide {
        task: f.task.then_some(task.as_path()),
        md: Some(md.as_path()),
        file: f.file.then_some(file.as_path()),
        peer: f.peer.then_some(peer.as_path()),
        help: Some(help.as_path()),
    })
    .unwrap_or_default();
    let _ = writeln!(out, "{guide}\n");
    let on = |b: bool| if b { "on" } else { "off" };
    let _ = writeln!(out, "## Right now on this machine");
    let _ = writeln!(out, "- task board: {}{}", on(f.task), if f.task { "" } else { " (a person turns on work panels in Settings)" });
    let _ = writeln!(out, "- file transfer: {}{}", on(f.file), if f.file { "" } else { " (needs a machine link or file receiving in Settings)" });
    let _ = writeln!(
        out,
        "- agent talk: {}{}",
        on(f.peer),
        if f.peer { "" } else { " (other characters cannot be reached; a person turns it on in Settings › Machine link › Agent talk)" }
    );
    if !f.peer {
        return 0;
    }
    let _ = writeln!(out, "\n## Characters (office-peer agents)");
    let timing = crate::peer::cli::Timing { pickup: Duration::from_secs(6), answer: Duration::from_secs(20) };
    let code = crate::peer::cli::run_with(&["agents".to_string()], env, "", out, err, timing);
    if code == 0 {
        let _ = writeln!(out, "\n\"no session\" or \"shell prompt\" is fine for `office-peer ask`: it opens the terminal and starts the CLI. An offline machine cannot be reached.");
    }
    code
}

#[cfg(test)]
mod tests {
    use super::*;

    fn call(args: &[&str], data: Option<&Path>) -> (i32, String, String) {
        let env = crate::peer::cli::PeerEnv { app_data: data.map(Path::to_path_buf), agent_id: None, in_session: false };
        let (mut out, mut err) = (Vec::new(), Vec::new());
        let args: Vec<String> = args.iter().map(|s| s.to_string()).collect();
        let code = run_with(&args, data, &env, &mut out, &mut err);
        (code, String::from_utf8(out).unwrap(), String::from_utf8(err).unwrap())
    }

    #[test]
    fn reads_features_at_run_time() {
        let tmp = tempfile::tempdir().unwrap();
        let (code, out, _) = call(&[], Some(tmp.path()));
        assert_eq!(code, 0);
        assert!(out.starts_with("# Agent Office"));
        assert!(out.contains("office-md") && !out.contains("`office-task` ="), "{out}");
        assert!(out.contains("- task board: off") && out.contains("- agent talk: off"), "{out}");

        std::fs::write(tmp.path().join("settings.json"), r#"{"workPanelsEnabled":true}"#).unwrap();
        let (_, out, _) = call(&[], Some(tmp.path()));
        assert!(out.contains("`office-task` =") && out.contains("- task board: on"), "{out}");
    }

    #[test]
    fn usage_and_missing_data() {
        assert_eq!(call(&["--help"], None).0, 0);
        assert_eq!(call(&["bogus"], None).0, 2);
        assert_eq!(call(&[], None).0, 1);
    }
}
