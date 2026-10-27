//! 태스크 대화(docs/task-chat-design.md): 태스크 하나를 주제로 한 캐릭터 없는 대화 세션을 열고
//! 첫 프롬프트(맥락 묶음)를 넣는다. 맥락은 렌더러가 넘긴 태스크·루트 스냅샷에 네이티브가
//! 작업자 보고·세션 로그 경로·Git 기록을 붙여 만든다. 대화 세션은 업무 상태를 건드리지 않는다.

use crate::i18n::Lang;
use crate::proc_runner::{self, ProcOutcome, ProcSpec};
use crate::types::{
    TaskChatContext, TaskChatRequest, TaskChatResult, WorkAgent, WorkCli, WorkSendRequest, WorkStatus,
    TASK_CHAT_PREFIX,
};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

/// 첫 프롬프트 상한. 보내기(`work_prompt`)와 같다.
pub const MAX_CONTEXT_BYTES: usize = 256 * 1024;
/// 동시에 살아 있는 대화 세션 수(PTY·스레드 수 보호).
pub const MAX_TASK_CHATS: usize = 4;
const MAX_COMMITS: &str = "30";
const GIT_TIMEOUT: Duration = Duration::from_secs(5);

/// 맥락 묶음의 고정 문구. 칸 순서는 설계 문서 §첫 프롬프트의 1~7번이다.
struct Text {
    role: &'static str,
    task: &'static str,
    id: &'static str,
    title: &'static str,
    status: &'static str,
    statuses: [&'static str; 4],
    assignee: &'static str,
    unassigned: &'static str,
    agent: &'static str,
    body: &'static str,
    depends_on: &'static str,
    dependents: &'static str,
    none: &'static str,
    plan: &'static str,
    no_plan: &'static str,
    plan_root: &'static str,
    run_state: &'static str,
    run_states: [&'static str; 4],
    pause_reason: &'static str,
    pause_detail: &'static str,
    memo: &'static str,
    log: &'static str,
    no_log: &'static str,
    log_kinds: [(&'static str, &'static str); 4],
    notes: &'static str,
    report: &'static str,
    no_report: &'static str,
    transcripts: &'static str,
    transcripts_hint: &'static str,
    no_sessions: &'static str,
    log_missing: &'static str,
    started: &'static str,
    commits: &'static str,
    no_cwd: &'static str,
    not_git: &'static str,
    never_sent: &'static str,
    no_commits: &'static str,
    truncated: &'static str,
}

const KO: Text = Text {
    role: "# 태스크 대화\n\n이 세션은 Agent Office 태스크 하나에 대해 사람과 이야기하는 질의응답용입니다. 사람이 시키지 않으면 파일을 고치거나, 커밋하거나, `office-task`로 태스크 상태를 바꾸지 마세요. 먼저 아래 맥락을 읽고 이 태스크가 지금 어떤 상태인지 쉬운 말로 짧게 요약한 뒤 질문을 기다리세요.",
    task: "## 태스크",
    id: "ID",
    title: "제목",
    status: "상태",
    statuses: ["할 일", "진행 중", "검토 대기", "완료"],
    assignee: "담당",
    unassigned: "미배정",
    agent: "에이전트",
    body: "본문",
    depends_on: "선행 작업",
    dependents: "후속 작업",
    none: "없음",
    plan: "## 플랜",
    no_plan: "이 태스크는 자동 진행 플랜에 속하지 않습니다.",
    plan_root: "루트",
    run_state: "실행 상태",
    run_states: ["진행 중", "일시정지", "끝남", "시작 안 함"],
    pause_reason: "일시정지 사유",
    pause_detail: "판단자 메시지",
    memo: "플랜 메모(루트 본문)",
    log: "## 자동 진행 기록",
    no_log: "이 태스크의 기록이 없습니다.",
    log_kinds: [("send", "보냄"), ("followup", "후속 지시"), ("next", "인계 메모"), ("human-done", "사람이 완료 처리")],
    notes: "앞 태스크에서 넘어온 인계 메모",
    report: "## 작업자 보고",
    no_report: "작업자 보고가 없습니다.",
    transcripts: "## 세션 전사 위치",
    transcripts_hint: "필요하면 아래 파일을 읽으세요. 내용이 커서 여기에는 넣지 않았습니다.",
    no_sessions: "이 태스크를 보낸 세션이 없습니다.",
    log_missing: "로그 파일 없음",
    started: "시작",
    commits: "## 최근 커밋",
    no_cwd: "작업 폴더가 정해지지 않아 커밋 기록을 찾지 않았습니다.",
    not_git: "작업 폴더가 Git 저장소가 아닙니다.",
    never_sent: "태스크를 보낸 적이 없어 커밋 기록을 찾지 않았습니다.",
    no_commits: "태스크를 처음 보낸 뒤의 커밋이 없습니다.",
    truncated: "…(길어서 잘림)",
};

const EN: Text = Text {
    role: "# Task chat\n\nThis session is for answering a person's questions about one Agent Office task. Unless the person asks, do not edit files, commit, or change task state with `office-task`. First read the context below, briefly summarize in plain words where this task stands, then wait for questions.",
    task: "## Task",
    id: "ID",
    title: "Title",
    status: "Status",
    statuses: ["To do", "In progress", "In review", "Done"],
    assignee: "Assignee",
    unassigned: "unassigned",
    agent: "Agent",
    body: "Description",
    depends_on: "Prerequisites",
    dependents: "Follow-up tasks",
    none: "none",
    plan: "## Plan",
    no_plan: "This task is not part of an automatic run plan.",
    plan_root: "Root",
    run_state: "Run state",
    run_states: ["running", "paused", "finished", "not started"],
    pause_reason: "Pause reason",
    pause_detail: "Supervisor message",
    memo: "Plan notes (root description)",
    log: "## Automatic run log",
    no_log: "No entries for this task.",
    log_kinds: [("send", "sent"), ("followup", "follow-up"), ("next", "hand-off note"), ("human-done", "marked done by a person")],
    notes: "Hand-off notes passed on from earlier tasks",
    report: "## Worker report",
    no_report: "No worker report.",
    transcripts: "## Session transcripts",
    transcripts_hint: "Read these files if you need them. They are large, so their contents are not included here.",
    no_sessions: "This task has not been sent to any session.",
    log_missing: "no log file",
    started: "started",
    commits: "## Recent commits",
    no_cwd: "No working folder, so commits were not looked up.",
    not_git: "The working folder is not a Git repository.",
    never_sent: "The task was never sent, so commits were not looked up.",
    no_commits: "No commits since the task was first sent.",
    truncated: "…(truncated)",
};

fn text(lang: Lang) -> &'static Text {
    match lang {
        Lang::Ko => &KO,
        Lang::En => &EN,
    }
}

/// 네이티브가 붙이는 칸의 출처. 테스트가 임시 폴더를 넘긴다.
pub struct ContextSources<'a> {
    /// `<app_data>/work-inbox`
    pub inbox: &'a Path,
    /// `<app_data>/session-logs/v1`
    pub session_log_root: &'a Path,
    pub cwd: Option<&'a Path>,
}

/// 첫 프롬프트(맥락 묶음). 칸 1~7을 순서대로 잇고, 상한을 넘으면 뒤쪽 칸부터 줄인다.
pub fn build_context(
    task_id: &str,
    agent: &WorkAgent,
    context: &TaskChatContext,
    sources: &ContextSources,
    lang: Lang,
) -> String {
    let t = text(lang);
    let sections = vec![
        t.role.to_string(),
        task_section(t, task_id, agent, context),
        plan_section(t, context),
        log_section(t, context),
        report_section(t, sources.inbox, task_id),
        transcripts_section(t, sources.session_log_root, context),
        commits_section(t, sources.cwd, context),
    ];
    fit(sections.into_iter().map(|s| sanitize(&s)).collect(), MAX_CONTEXT_BYTES, t.truncated)
}

fn status_name(t: &Text, status: &WorkStatus) -> &'static str {
    match status {
        WorkStatus::Todo => t.statuses[0],
        WorkStatus::InProgress => t.statuses[1],
        WorkStatus::Review => t.statuses[2],
        WorkStatus::Done => t.statuses[3],
    }
}

fn agent_label(agent: &WorkAgent) -> String {
    let cli = match agent.cli {
        WorkCli::Claude => "claude",
        WorkCli::Codex => "codex",
        WorkCli::Agy => "agy",
        WorkCli::Kilo => "kilo",
        WorkCli::Pi => "pi",
    };
    let extra: Vec<&str> = [agent.model.as_deref(), agent.effort.as_deref()].into_iter().flatten().collect();
    if extra.is_empty() { cli.to_string() } else { format!("{cli} ({})", extra.join(", ")) }
}

fn task_section(t: &Text, task_id: &str, agent: &WorkAgent, context: &TaskChatContext) -> String {
    let task = &context.task;
    let links = |links: &[crate::types::TaskChatLink]| {
        if links.is_empty() {
            format!(" {}", t.none)
        } else {
            links
                .iter()
                .map(|l| format!("\n- {} ({}, {})", l.title.trim(), l.id, status_name(t, &l.status)))
                .collect()
        }
    };
    let body = if task.body.trim().is_empty() { t.none.to_string() } else { task.body.trim().to_string() };
    format!(
        "{}\n\n- {}: {task_id}\n- {}: {}\n- {}: {}\n- {}: {}\n- {}: {}\n\n{}:{}\n\n{}:{}\n\n### {}\n\n{body}",
        t.task,
        t.id,
        t.title,
        task.title.trim(),
        t.status,
        status_name(t, &task.status),
        t.assignee,
        task.assignee.as_deref().map(str::trim).filter(|a| !a.is_empty()).unwrap_or(t.unassigned),
        t.agent,
        agent_label(agent),
        t.depends_on,
        links(&task.depends_on),
        t.dependents,
        links(&task.dependents),
        t.body,
    )
}

fn plan_section(t: &Text, context: &TaskChatContext) -> String {
    let Some(plan) = &context.plan else {
        return format!("{}\n\n{}", t.plan, t.no_plan);
    };
    let state = match plan.state.as_deref() {
        Some("running") => t.run_states[0],
        Some("paused") => t.run_states[1],
        Some("finished") => t.run_states[2],
        Some(other) => other,
        None => t.run_states[3],
    };
    let mut out = format!(
        "{}\n\n- {}: {} ({})\n- {}: {state}",
        t.plan,
        t.plan_root,
        plan.title.trim(),
        plan.id,
        t.run_state,
    );
    if let Some(reason) = plan.pause_reason.as_deref().filter(|r| !r.trim().is_empty()) {
        out.push_str(&format!("\n- {}: {}", t.pause_reason, reason.trim()));
    }
    if let Some(detail) = plan.pause_detail.as_deref().filter(|d| !d.trim().is_empty()) {
        out.push_str(&format!("\n\n### {}\n\n{}", t.pause_detail, detail.trim()));
    }
    if !plan.body.trim().is_empty() {
        out.push_str(&format!("\n\n### {}\n\n{}", t.memo, plan.body.trim()));
    }
    out
}

fn log_section(t: &Text, context: &TaskChatContext) -> String {
    let Some(plan) = &context.plan else {
        return format!("{}\n\n{}", t.log, t.no_log);
    };
    let mut out = t.log.to_string();
    if plan.log.is_empty() {
        out.push_str(&format!("\n\n{}", t.no_log));
    } else {
        out.push('\n');
        for entry in &plan.log {
            let kind = t.log_kinds.iter().find(|(k, _)| *k == entry.kind).map_or(entry.kind.as_str(), |(_, v)| *v);
            out.push_str(&format!("\n- {} {kind}", format_time(entry.at)));
            if let Some(text) = entry.text.as_deref().map(str::trim).filter(|s| !s.is_empty()) {
                out.push_str(&format!(": {text}"));
            }
        }
    }
    if !plan.handoff_notes.is_empty() {
        out.push_str(&format!("\n\n### {}\n", t.notes));
        for note in &plan.handoff_notes {
            out.push_str(&format!("\n- {} ({}): {}", note.title.trim(), note.task_id, note.text.trim()));
        }
    }
    out
}

fn report_section(t: &Text, inbox: &Path, task_id: &str) -> String {
    // since 없이 최신본을 읽는다 — 대화는 이번 턴이 아니라 태스크 전체를 본다.
    match crate::auto_run::read_report(inbox, task_id, 0, std::time::SystemTime::now()) {
        Some(report) => format!("{}\n\n{report}", t.report),
        None => format!("{}\n\n{}", t.report, t.no_report),
    }
}

fn transcripts_section(t: &Text, root: &Path, context: &TaskChatContext) -> String {
    let sessions = &context.task.sessions;
    if sessions.is_empty() {
        return format!("{}\n\n{}", t.transcripts, t.no_sessions);
    }
    let mut out = format!("{}\n\n{}\n", t.transcripts, t.transcripts_hint);
    for session in sessions {
        match crate::session_log::store::find_session_log(root, &session.agent_id, &session.session_id) {
            Some((path, started)) => {
                out.push_str(&format!("\n- {}", path.display()));
                if let Some(at) = started.or(session.sent_at) {
                    out.push_str(&format!(" ({} {})", t.started, format_time(at)));
                }
            }
            None => out.push_str(&format!("\n- {} {}: {}", t.log_missing, session.agent_id, session.session_id)),
        }
    }
    out
}

fn commits_section(t: &Text, cwd: Option<&Path>, context: &TaskChatContext) -> String {
    let note = |s: &str| format!("{}\n\n{s}", t.commits);
    let Some(cwd) = cwd else { return note(t.no_cwd) };
    let Some(first_sent) = context.task.sessions.iter().filter_map(|s| s.sent_at).min() else {
        return note(t.never_sent);
    };
    let Some(inside) = git(cwd, &["rev-parse", "--is-inside-work-tree"]) else { return note(t.not_git) };
    if inside.trim() != "true" {
        return note(t.not_git);
    }
    let since = format!("--since=@{}", first_sent / 1000);
    match git(cwd, &["log", "--oneline", "--no-color", "-n", MAX_COMMITS, &since]) {
        Some(log) if !log.trim().is_empty() => format!("{}\n\n```text\n{}\n```", t.commits, log.trim_end()),
        Some(_) => note(t.no_commits),
        None => note(t.not_git),
    }
}

fn git(cwd: &Path, args: &[&str]) -> Option<String> {
    let run = proc_runner::run(ProcSpec {
        program: "git",
        args,
        cwd: Some(cwd),
        envs: &[],
        timeout: GIT_TIMEOUT,
        max_stdout_bytes: Some(64 * 1024),
        cancel: None,
    });
    matches!(run.outcome, ProcOutcome::Exited { success: true }).then(|| String::from_utf8_lossy(&run.stdout).into_owned())
}

fn format_time(ms: u64) -> String {
    chrono::DateTime::from_timestamp_millis(ms as i64)
        .map(|t| t.with_timezone(&chrono::Local).format("%Y-%m-%d %H:%M").to_string())
        .unwrap_or_else(|| ms.to_string())
}

/// 붙여넣기에 실을 수 없는 제어 문자를 뺀다(`work_prompt`가 거절하는 것). 줄바꿈·탭은 둔다.
fn sanitize(text: &str) -> String {
    text.replace("\r\n", "\n")
        .chars()
        .filter(|c| !c.is_control() || matches!(c, '\n' | '\t'))
        .collect()
}

const SEPARATOR: &str = "\n\n";

/// 칸을 잇되 `limit` 바이트를 넘으면 뒤쪽 칸부터 끝을 잘라 `marker`를 단다. 첫 칸(역할)은 자르지 않는다.
fn fit(mut sections: Vec<String>, limit: usize, marker: &str) -> String {
    let total = |s: &[String]| s.iter().map(String::len).sum::<usize>() + SEPARATOR.len() * s.len().saturating_sub(1);
    for i in (1..sections.len()).rev() {
        let over = total(&sections).saturating_sub(limit);
        if over == 0 {
            break;
        }
        let section = &sections[i];
        // 칸 제목 줄은 남긴다. 제목만 남겨도 넘치면 다음(앞쪽) 칸으로 넘어간다.
        let heading = section.find('\n').unwrap_or(section.len());
        let keep = section.len().saturating_sub(over + marker.len() + 1).max(heading);
        if keep >= section.len() {
            continue;
        }
        let mut cut = keep;
        while !section.is_char_boundary(cut) {
            cut -= 1;
        }
        sections[i] = format!("{}\n{marker}", section[..cut].trim_end());
    }
    let mut out = sections.join(SEPARATOR);
    if out.len() > limit {
        let mut cut = limit;
        while !out.is_char_boundary(cut) {
            cut -= 1;
        }
        out.truncate(cut);
    }
    out
}

/// 열기 요청 검증. 통과하면 대화 세션의 agentId(`task-chat-<taskId>`).
pub fn validate(request: &TaskChatRequest) -> Result<String, String> {
    if !crate::persistence::work_inbox::valid_task_id(&request.task_id) {
        return Err("task-chat-invalid-task".into());
    }
    if request.cwd.as_deref().is_some_and(|cwd| !Path::new(cwd).is_absolute()) {
        return Err("task-chat-invalid-cwd".into());
    }
    crate::work_send::validated_command(&request.agent, crate::work_send::cli_transition::CommandSyntax::Posix)?;
    Ok(format!("{TASK_CHAT_PREFIX}{}", request.task_id))
}

/// 같은 태스크의 열기가 겹치지 않게 한다. 실패한 열기가 남이 연 세션을 버리지 않게 하려는 것.
fn opening() -> &'static Mutex<HashSet<String>> {
    static OPENING: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
    OPENING.get_or_init(Default::default)
}
struct OpeningGuard(String);
impl OpeningGuard {
    fn acquire(agent_id: &str) -> Result<Self, String> {
        if !opening().lock().unwrap().insert(agent_id.to_string()) {
            return Err("task-chat-busy".into());
        }
        Ok(Self(agent_id.to_string()))
    }
}
impl Drop for OpeningGuard {
    fn drop(&mut self) {
        opening().lock().unwrap().remove(&self.0);
    }
}

/// 대화 세션을 만들고 기존 보내기 경로(기동 ACK → 대기 → bracketed paste + CR)로 맥락 묶음을 넣는다.
/// 실패하면 만든 세션을 버린다(렌더러에 아직 탭이 없다). 업무 `sessionIds`·상태는 렌더러도 바꾸지 않는다.
pub async fn open(app_state: &crate::state::AppState, request: TaskChatRequest) -> Result<TaskChatResult, String> {
    let agent_id = validate(&request)?;
    let _guard = OpeningGuard::acquire(&agent_id)?;
    let manager = &app_state.manager;
    if manager.is_running(&agent_id) {
        return Err("task-chat-running".into());
    }
    if manager.live_task_chat_count() >= MAX_TASK_CHATS {
        return Err("task-chat-limit".into());
    }
    let lang = crate::i18n::ui_lang(&app_state.settings.read().unwrap());
    let prompt = {
        let inbox = app_state.app_data_dir.join("work-inbox");
        let log_root = app_state.session_log_root.clone();
        let cwd = request.cwd.clone().map(PathBuf::from);
        let (task_id, agent, context) = (request.task_id.clone(), request.agent.clone(), request.context.clone());
        tauri::async_runtime::spawn_blocking(move || {
            let sources = ContextSources { inbox: &inbox, session_log_root: &log_root, cwd: cwd.as_deref() };
            build_context(&task_id, &agent, &context, &sources, lang)
        })
        .await
        .map_err(|e| e.to_string())?
    };
    let created = crate::ipc::commands::spawn_session(
        manager,
        &app_state.observer,
        &app_state.observer_server,
        &app_state.settings,
        crate::types::CreateSessionRequest {
            agent_id: agent_id.clone(),
            cols: request.cols,
            rows: request.rows,
            cwd: request.cwd.clone(),
            shell: request.shell.clone(),
            startup_command: None,
            personality_prompt: None,
            autostart_claude: None,
            task_chat: true,
        },
        crate::session_events::types::AgentEventProfile { name: request.context.task.title.clone(), role: None },
    )
    .await?;
    let session_id = created.session_id;
    if created.state != crate::types::SessionState::Running {
        let _ = manager.dispose_matching(&agent_id, Some(&session_id));
        return Err("work-send-create-failed".into());
    }
    let sent = app_state
        .work_send_runtime
        .send(
            &app_state.gate,
            app_state.app_data_dir.clone(),
            WorkSendRequest {
                agent_id: agent_id.clone(),
                session_id: session_id.clone(),
                human_input_revision: app_state.gate.human_input_revision(&agent_id),
                fresh_session: true,
                agent: request.agent,
                title: String::new(),
                body: prompt,
            },
        )
        .await;
    if let Err(error) = sent {
        let _ = manager.dispose_matching(&agent_id, Some(&session_id));
        return Err(error);
    }
    Ok(TaskChatResult { agent_id, session_id })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{TaskChatLink, TaskChatLogEntry, TaskChatNote, TaskChatPlan, TaskChatSession, TaskChatTask};

    fn agent() -> WorkAgent {
        WorkAgent { cli: WorkCli::Claude, model: Some("opus".into()), effort: None }
    }
    fn context() -> TaskChatContext {
        TaskChatContext {
            task: TaskChatTask {
                title: "Build the parser".into(),
                status: WorkStatus::InProgress,
                assignee: Some("Hana".into()),
                body: "Parse the inbox files.".into(),
                depends_on: vec![TaskChatLink { id: "t-prev".into(), title: "Design".into(), status: WorkStatus::Done }],
                dependents: vec![],
                sessions: vec![TaskChatSession { session_id: "abcdef12-3456".into(), agent_id: "hana".into(), sent_at: Some(1_000) }],
            },
            plan: Some(TaskChatPlan {
                id: "t-root".into(),
                title: "Inbox plan".into(),
                body: "Plan memo".into(),
                state: Some("paused".into()),
                pause_reason: Some("judge-stop".into()),
                pause_detail: Some("Which format?".into()),
                log: vec![TaskChatLogEntry { at: 1_000, kind: "send".into(), text: None }],
                handoff_notes: vec![TaskChatNote { task_id: "t-prev".into(), title: "Design".into(), text: "Use JSON".into() }],
            }),
        }
    }
    struct Dirs {
        _tmp: tempfile::TempDir,
        inbox: PathBuf,
        logs: PathBuf,
        cwd: PathBuf,
    }
    fn dirs() -> Dirs {
        let tmp = tempfile::tempdir().unwrap();
        let (inbox, logs, cwd) = (tmp.path().join("work-inbox"), tmp.path().join("logs"), tmp.path().join("cwd"));
        std::fs::create_dir_all(&cwd).unwrap();
        Dirs { _tmp: tmp, inbox, logs, cwd }
    }
    fn build(d: &Dirs, ctx: &TaskChatContext) -> String {
        let sources = ContextSources { inbox: &d.inbox, session_log_root: &d.logs, cwd: Some(&d.cwd) };
        build_context("t-1", &agent(), ctx, &sources, Lang::En)
    }
    fn git_in(dir: &Path, args: &[&str]) {
        let ok = std::process::Command::new("git")
            .args(["-c", "user.name=t", "-c", "user.email=t@t", "-c", "commit.gpgsign=false"])
            .args(args)
            .current_dir(dir)
            .stdout(std::process::Stdio::null())
            .status()
            .unwrap()
            .success();
        assert!(ok, "git {args:?}");
    }

    #[test]
    fn sections_come_in_design_order_with_native_parts_attached() {
        let d = dirs();
        std::fs::create_dir_all(d.inbox.join("reports")).unwrap();
        std::fs::write(d.inbox.join("reports/t-1.md"), "status: done\nchanged: parser").unwrap();
        let log = crate::session_log::store::SessionLogWriter::open(&d.logs, "hana", "abcdef12-3456", "/w", std::time::SystemTime::now())
            .unwrap();
        let log_path = log.path().to_path_buf();
        git_in(&d.cwd, &["init", "-q"]);
        git_in(&d.cwd, &["commit", "-q", "--allow-empty", "-m", "parser commit"]);

        let out = build(&d, &context());
        let at = |needle: &str| out.find(needle).unwrap_or_else(|| panic!("missing {needle:?} in\n{out}"));
        let order = [
            at("# Task chat"),
            at("## Task"),
            at("## Plan"),
            at("## Automatic run log"),
            at("## Worker report"),
            at("## Session transcripts"),
            at("## Recent commits"),
        ];
        assert!(order.windows(2).all(|w| w[0] < w[1]), "{order:?}");
        assert!(out.contains("- ID: t-1"));
        assert!(out.contains("claude (opus)"));
        assert!(out.contains("Design (t-prev, Done)"));
        assert!(out.contains("judge-stop") && out.contains("Which format?") && out.contains("Use JSON"));
        assert!(out.contains("changed: parser"));
        assert!(out.contains(&log_path.display().to_string()));
        assert!(out.contains("parser commit"));
    }

    #[test]
    fn missing_report_non_git_and_missing_session_log_are_stated() {
        let d = dirs();
        let out = build(&d, &context());
        assert!(out.contains(EN.no_report));
        assert!(out.contains(EN.not_git));
        assert!(out.contains(&format!("{} hana: abcdef12-3456", EN.log_missing)));

        let mut ctx = context();
        ctx.task.sessions.clear();
        ctx.plan = None;
        let out = build(&d, &ctx);
        assert!(out.contains(EN.no_sessions) && out.contains(EN.never_sent) && out.contains(EN.no_plan));
    }

    #[test]
    fn commits_before_the_first_send_are_left_out() {
        let d = dirs();
        git_in(&d.cwd, &["init", "-q"]);
        git_in(&d.cwd, &["commit", "-q", "--allow-empty", "-m", "old commit"]);
        let mut ctx = context();
        ctx.task.sessions[0].sent_at = Some(crate::types::now_ms() + 60_000);
        let out = build(&d, &ctx);
        assert!(!out.contains("old commit"));
        assert!(out.contains(EN.no_commits));
    }

    #[test]
    fn over_limit_cuts_later_sections_first_and_keeps_headings() {
        let d = dirs();
        std::fs::create_dir_all(d.inbox.join("reports")).unwrap();
        std::fs::write(d.inbox.join("reports/t-1.md"), "report body").unwrap();
        let mut ctx = context();
        // 플랜 메모(3번 칸)가 상한을 넘긴다. 뒤쪽 칸은 제목만 남고, 앞쪽 태스크 칸은 그대로다.
        ctx.plan.as_mut().unwrap().body = "가".repeat(MAX_CONTEXT_BYTES);
        let out = build(&d, &ctx);
        assert!(out.len() <= MAX_CONTEXT_BYTES);
        assert!(out.contains("Parse the inbox files."));
        for heading in ["## Automatic run log", "## Worker report", "## Session transcripts", "## Recent commits"] {
            assert!(out.contains(&format!("{heading}\n{}", EN.truncated)), "{heading}");
        }
        assert!(!out.contains("report body"));
        assert!(out.contains("## Plan\n\n- Root"));
    }

    #[test]
    fn small_overflow_only_trims_the_last_section() {
        let sections = vec!["role".to_string(), "## A\nkeep".into(), "## B\n".to_string() + &"x".repeat(100)];
        let out = fit(sections, 60, "…");
        assert!(out.len() <= 60);
        assert!(out.starts_with("role\n\n## A\nkeep\n\n## B\nxxx"), "{out}");
        assert!(out.ends_with('…'));
    }

    #[test]
    fn control_sequences_from_snapshots_are_dropped() {
        let d = dirs();
        let mut ctx = context();
        ctx.task.body = "line\r\nnext \x1b[201~echo injected\0".into();
        let out = build(&d, &ctx);
        assert!(out.contains("line\nnext [201~echo injected"));
        assert!(!out.chars().any(|c| c.is_control() && !matches!(c, '\n' | '\t')));
    }

    #[test]
    fn korean_wording_follows_the_language() {
        let d = dirs();
        let sources = ContextSources { inbox: &d.inbox, session_log_root: &d.logs, cwd: None };
        let out = build_context("t-1", &agent(), &context(), &sources, Lang::Ko);
        assert!(out.starts_with("# 태스크 대화"));
        assert!(out.contains("진행 중") && out.contains(KO.no_cwd));
    }

    fn request(json: serde_json::Value) -> Result<TaskChatRequest, serde_json::Error> {
        serde_json::from_value(json)
    }
    fn minimal() -> serde_json::Value {
        serde_json::json!({
            "taskId": "t-1",
            "agent": { "cli": "claude" },
            "context": { "task": { "title": "T", "status": "todo", "body": "" } }
        })
    }

    #[test]
    fn request_validation_builds_the_chat_id_and_rejects_bad_input() {
        let ok = request(minimal()).unwrap();
        assert_eq!(validate(&ok).unwrap(), "task-chat-t-1");
        assert!(crate::types::is_task_chat_id(&validate(&ok).unwrap()));

        for (field, value, code) in [
            ("taskId", serde_json::json!("../x"), "task-chat-invalid-task"),
            ("taskId", serde_json::json!(""), "task-chat-invalid-task"),
            ("cwd", serde_json::json!("relative/dir"), "task-chat-invalid-cwd"),
            ("agent", serde_json::json!({ "cli": "claude", "model": "x\ny" }), "work-send-model-invalid"),
        ] {
            let mut json = minimal();
            json[field] = value;
            assert_eq!(validate(&request(json).unwrap()).unwrap_err(), code, "{field}");
        }
        // 렌더러가 모르는 칸을 보내면 조용히 버리지 않고 거절한다.
        let mut json = minimal();
        json["prompt"] = serde_json::json!("raw");
        assert!(request(json).is_err());
        let mut json = minimal();
        json["agent"] = serde_json::json!({ "cli": "bash" });
        assert!(request(json).is_err());
    }

    #[test]
    fn same_task_cannot_be_opened_twice_at_once() {
        let first = OpeningGuard::acquire("task-chat-guard-test").unwrap();
        assert_eq!(OpeningGuard::acquire("task-chat-guard-test").err().unwrap(), "task-chat-busy");
        drop(first);
        assert!(OpeningGuard::acquire("task-chat-guard-test").is_ok());
    }
}
