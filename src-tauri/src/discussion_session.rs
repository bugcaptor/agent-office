//! 토의 참가자 세션: 참가자마다 캐릭터 본 터미널이 아닌 토의 전용 탭을 열고 첫 프롬프트
//! (역할 카드 + 토의 규칙 + 주제·맥락)를 넣는다. 태스크 대화와 달리 관찰 훅을 붙여 턴 종료를
//! 받고(`control::turns::TurnTracker`, 렌더러의 알림·사용량 이벤트), 읽기 전용은 CLI가
//! 강제할 수 있는 만큼 플래그로 건다. 발언은 `office-discuss say`로 제출받고, 제출 명령을
//! 못 쓰는 Codex(읽기 전용 샌드박스)는 Stop 훅의 마지막 응답을 받는다.
//! 진행(누가 언제 말하나)은 실행기가 쥔다. 여기는 열기·차례 보내기·발언 수집까지다.

use crate::i18n::Lang;
use crate::observer::WrapperArg;
use crate::types::{
    CollectedUtterance, Discussion, DiscussionCollection, DiscussionReadOnly, DiscussionRole, DiscussionSessionRequest,
    DiscussionSessionResult, DiscussionStance, DiscussionUtteranceSource, WorkCli, WorkSendRequest,
    DISCUSSION_SESSION_PREFIX,
};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};

/// 동시에 살아 있는 토의 세션 수(사회자 포함 최대 5명 × 동시 토의 1개).
pub const MAX_DISCUSSION_SESSIONS: usize = 5;
/// 제출 명령이 어느 참가자인지 아는 env. 값은 토의 세션 agentId.
pub const SESSION_ENV: &str = "AGENT_OFFICE_DISCUSSION_SESSION";
/// `office-discuss` shim 경로 env.
pub const SHIM_ENV: &str = "AGENT_OFFICE_DISCUSS";
/// 첫 프롬프트 상한. 보내기(`work_prompt`)와 같다.
const MAX_PROMPT_BYTES: usize = 256 * 1024;
/// 맥락 칸 상한(문자). 넘으면 뒤를 자른다.
const MAX_CONTEXT_CHARS: usize = 64 * 1024;
/// 발언 상한(문자). 넘으면 앞을 자른다 — 결론은 끝에 있다.
pub const MAX_UTTERANCE_CHARS: usize = crate::auto_run::MAX_REPLY_CHARS;
/// 기억해 둘 Codex 마지막 응답 수. 넘으면 오래된 것부터 버린다.
const MAX_REPLIES: usize = 64;

/// 토의 id·참가자 id. 파일 이름과 agentId 경계(`.`)에 쓰므로 `[A-Za-z0-9_-]{1,64}`만 받는다.
pub fn valid_id(id: &str) -> bool {
    crate::persistence::work_inbox::valid_task_id(id)
}

/// 토의 세션 agentId(`discuss-<토의 id>.<참가자>`).
pub fn session_agent_id(discussion_id: &str, participant: &str) -> Result<String, String> {
    if !valid_id(discussion_id) || !valid_id(participant) {
        return Err("discussion-session-invalid-id".into());
    }
    Ok(format!("{DISCUSSION_SESSION_PREFIX}{discussion_id}.{participant}"))
}

/// 토의 세션 agentId → (토의 id, 참가자 id). 형식이 아니면 None.
pub fn parse_session_agent_id(agent_id: &str) -> Option<(&str, &str)> {
    let rest = agent_id.strip_prefix(DISCUSSION_SESSION_PREFIX)?;
    let (discussion, participant) = rest.split_once('.')?;
    (valid_id(discussion) && valid_id(participant)).then_some((discussion, participant))
}

/// 발언 제출 파일이 사는 폴더.
pub fn turns_dir(app_data: &Path) -> PathBuf {
    app_data.join("discussions").join("turns")
}

/// 그 토의 세션의 발언 파일. agentId가 형식이 아니면 None(경로 탈출 방지).
pub fn turn_file(app_data: &Path, agent_id: &str) -> Option<PathBuf> {
    parse_session_agent_id(agent_id)?;
    Some(turns_dir(app_data).join(format!("{agent_id}.md")))
}

/// 읽기 전용을 어떻게 지키는가. 근거는 각 CLI의 `--help`(2026-11-20, claude 2.1.329 ·
/// codex-cli 0.170.0 · agy · kilo 7.8.3 · pi 1.5.0)와 `-p`·`exec` 실측이다.
pub fn read_only_mode(cli: &WorkCli) -> DiscussionReadOnly {
    match cli {
        // `--disallowed-tools Edit Write NotebookEdit`(거절이 허용보다 앞선다) + `dontAsk`(허용 안 된
        // 도구는 묻지 않고 거절). 다만 사용자·프로젝트 설정과 `--settings`의 allow 규칙은 dontAsk를
        // 통과한다(실측: `Bash(touch:*)` 허용 시 touch 성공). 그래서 셸 명령은 완전히 막히지 않는다.
        WorkCli::Claude => DiscussionReadOnly::Partial,
        // `--sandbox read-only`: 셸 명령의 파일 쓰기를 OS 샌드박스가 막는다(실측: Operation not permitted).
        WorkCli::Codex => DiscussionReadOnly::Flags,
        // `--exclude-tools edit,write`: 편집 도구만 뺀다(실측: 대신 bash로 파일을 만들었다). bash는
        // 제출 명령에 필요해 남긴다.
        WorkCli::Pi => DiscussionReadOnly::Partial,
        // agy는 업무 보내기가 `--dangerously-skip-permissions`로 띄우고 `--mode plan`은 쓰기 차단이
        // 아니다. kilo는 `--auto`로 띄우며 `--agent ask`(edit·write 거절)는 이 환경에서 실행 검증을
        // 못 했다. 둘 다 프롬프트로만 제한한다.
        WorkCli::Agy | WorkCli::Kilo => DiscussionReadOnly::Prompt,
    }
}

/// 발언을 어떻게 받는가.
pub fn collection_mode(cli: &WorkCli) -> DiscussionCollection {
    match cli {
        // 읽기 전용 샌드박스는 제출 명령의 앱 데이터 쓰기도 막는다(실측: Operation not permitted).
        WorkCli::Codex => DiscussionCollection::LastReply,
        _ => DiscussionCollection::Submit,
    }
}

/// 토의 세션 셸 래퍼에 붙일 읽기 전용 인자(명령별). 탭에서 어느 CLI를 띄워도 걸리게
/// 참가자 CLI와 무관하게 모두 싣는다. `shim`은 Claude 허용 목록에 넣을 제출 명령 경로다.
pub fn read_only_args(shim: Option<&Path>) -> Vec<(&'static str, Vec<WrapperArg>)> {
    let lit = |s: &str| WrapperArg::Literal(s.to_string());
    let mut allowed = "Read Grep Glob WebFetch WebSearch".to_string();
    if let Some(shim) = shim {
        allowed.push_str(&format!(" Bash({}:*)", shim.display()));
    }
    vec![
        (
            "claude",
            vec![
                lit("--allowed-tools"),
                WrapperArg::Literal(allowed),
                // 사용자 설정의 allow 규칙이 편집 도구를 열어 두었어도 막는다.
                lit("--disallowed-tools"),
                lit("Edit Write NotebookEdit"),
                // 값 하나짜리 옵션을 끝에 둔다. 앞의 두 옵션은 값을 여럿 받으므로 뒤에 위치 인자가 오면 삼킨다.
                lit("--permission-mode"),
                lit("dontAsk"),
            ],
        ),
        // 승인 프롬프트에서 멈추지 않고(`never`), 웹 조사는 `--search`로 연다.
        ("codex", vec![lit("--sandbox"), lit("read-only"), lit("--ask-for-approval"), lit("never"), lit("--search")]),
        ("pi", vec![lit("--exclude-tools"), lit("edit,write")]),
    ]
}

/// 열기 상한. `live`는 살아 있는 토의 세션 agentId들이다. 다른 토의의 세션이 남아 있으면
/// 동시 토의 1개 제한에 걸린다.
pub fn limit_problem(live: &[String], agent_id: &str) -> Option<&'static str> {
    let discussion = parse_session_agent_id(agent_id).map(|(d, _)| d);
    if live.iter().any(|id| parse_session_agent_id(id).map(|(d, _)| d) != discussion) {
        return Some("discussion-session-other");
    }
    if live.iter().filter(|id| id.as_str() != agent_id).count() >= MAX_DISCUSSION_SESSIONS {
        return Some("discussion-session-limit");
    }
    None
}

// ── 첫 프롬프트 ──────────────────────────────────────────────────────

struct Text {
    title: &'static str,
    you_are: &'static str,
    seat_moderator: &'static str,
    seat_debater: &'static str,
    role: &'static str,
    role_name: &'static str,
    stance: &'static str,
    perspective: &'static str,
    criteria: &'static str,
    no_role: &'static str,
    stances: [&'static str; 7],
    rules: &'static str,
    read_only: &'static str,
    enforced: [&'static str; 3],
    flow: &'static str,
    moderator_rules: &'static str,
    debater_rules: &'static str,
    submit: &'static str,
    submit_placeholder: &'static str,
    submit_after: &'static str,
    last_reply: &'static str,
    topic: &'static str,
    context: &'static str,
    no_context: &'static str,
    cwd: &'static str,
    home: &'static str,
    participants: &'static str,
    moderator_mark: &'static str,
    you_mark: &'static str,
    missing_member: &'static str,
    persona: &'static str,
    persona_hint: &'static str,
    turn: &'static str,
    wait: &'static str,
    truncated: &'static str,
}

const KO: Text = Text {
    title: "# 팀 토의",
    you_are: "당신은 팀원 **{name}**입니다. 이 세션은 Agent Office 팀 토의에서 당신 자리입니다.",
    seat_moderator: "당신은 이 토의의 **사회자**입니다.",
    seat_debater: "당신은 이 토의의 **토론자**입니다.",
    role: "## 역할 카드",
    role_name: "역할",
    stance: "토의 스탠스",
    perspective: "관점·책임",
    criteria: "판단 기준·금지사항",
    no_role: "따로 맡은 역할이 없습니다. 당신의 관점에서 자유롭게 의견을 내세요.",
    stances: [
        "사회자(중립) — 의견을 내지 않고 발언자 지목, 수렴 판단, 결정 메모 작성만 합니다.",
        "자유 의견 — 정해진 태도 없이 자기 관점에서 의견을 냅니다.",
        "동조·보강 — 좋은 제안을 받아 구체화하고 보강합니다.",
        "비판 — 제안의 약점·위험·빠진 근거를 찾아 짚습니다.",
        "악마의 대변인 — 합의된 쪽의 반대 입장을 일부러 맡아 끝까지 따져 봅니다.",
        "아이디어 발산 — 판단을 미루고 다양한 대안을 넓게 내놓습니다.",
        "종합 — 나온 의견을 묶어 절충안과 남은 쟁점을 정리합니다.",
    ],
    rules: "## 토의 규칙",
    read_only: "- **읽기 전용**: 코드·문서를 읽고 웹을 조사할 수 있지만, 파일을 만들거나 고치거나 지우지 마세요. 커밋·푸시·패키지 설치를 하지 말고, `office-task`로 업무를 만들거나 바꾸지 마세요.",
    enforced: [
        "  이 세션은 CLI 설정으로 수정 도구가 막혀 있습니다.",
        "  이 세션은 CLI 설정으로 편집 도구만 막혀 있고 셸 명령은 다 막히지 않습니다. 셸 명령으로도 아무것도 고치지 마세요.",
        "  이 세션은 CLI 설정으로 막혀 있지 않습니다. 이 규칙을 스스로 지켜야 합니다.",
    ],
    flow: "- **진행**: 사회자가 진행합니다. 첫 라운드는 토론자 모두가 서로의 의견을 보지 않고 동시에 발언합니다. 그다음부터는 사회자가 지목한 사람이 차례로 말합니다. 다른 참가자의 새 발언은 앱이 차례마다 전해 줍니다. 차례 지시가 오기 전에는 발언하지 마세요.",
    moderator_rules: "- **사회자**: 중립을 지키고 자기 의견을 내지 마세요. 차례마다 다음 발언자와 질문을 정하거나, 사람에게 물을 것을 정하거나, 토의를 정리합니다. 지시 형식은 차례 지시를 따르세요.",
    debater_rules: "- **토론자**: 역할 카드의 관점과 스탠스로 말하세요. 근거를 대고 짧게(대략 300단어 이내) 말하세요. 동의만 하지 말고 새 정보나 이유를 보태세요.",
    submit: "## 발언 내는 법\n\n차례마다 발언(사회자는 지시)을 정한 뒤 아래 명령으로 제출하고 차례를 마치세요. 같은 차례에 다시 내면 바뀝니다. 제출한 내용만 다른 참가자에게 전해집니다.",
    submit_placeholder: "<발언>",
    submit_after: "명령을 쓸 수 없으면 마지막 답변에 발언만 쓰세요.",
    last_reply: "## 발언 내는 법\n\n이 세션은 샌드박스가 쓰기를 막아 제출 명령을 쓸 수 없습니다. 차례마다 마지막 답변 전체가 발언(사회자는 지시)으로 전해지니, 마지막 답변에는 발언만 쓰세요.",
    topic: "## 주제",
    context: "### 맥락",
    no_context: "따로 적힌 맥락이 없습니다.",
    cwd: "작업 폴더",
    home: "홈 폴더",
    participants: "## 참가자",
    moderator_mark: "사회자",
    you_mark: "나",
    missing_member: "(삭제된 팀원)",
    persona: "## 말투",
    persona_hint: "아래는 이 캐릭터의 성격·말투입니다. 말투에만 반영하고, 무엇을 말할지는 역할 카드와 토의 규칙을 따르세요.",
    turn: "## 이번 차례",
    wait: "## 지금\n\n아직 발언하지 마세요. 위 내용을 읽었으면 제출하지 말고 \"준비됐습니다\" 한 줄로만 답한 뒤 차례 지시를 기다리세요.",
    truncated: "…(길어서 잘림)",
};

const EN: Text = Text {
    title: "# Team discussion",
    you_are: "You are team member **{name}**. This session is your seat in an Agent Office team discussion.",
    seat_moderator: "You are the **moderator** of this discussion.",
    seat_debater: "You are a **debater** in this discussion.",
    role: "## Role card",
    role_name: "Role",
    stance: "Stance",
    perspective: "Perspective and responsibility",
    criteria: "Criteria and things not to do",
    no_role: "You have no assigned role. Speak freely from your own perspective.",
    stances: [
        "Moderator (neutral) — gives no opinion; only picks speakers, judges convergence and writes the decision memo.",
        "Free — no set attitude; gives opinions from their own perspective.",
        "Supportive — takes good proposals and makes them concrete and stronger.",
        "Critical — finds weaknesses, risks and missing evidence in proposals.",
        "Devil's advocate — deliberately argues against the side that is agreed and tests it to the end.",
        "Divergent — holds off judgment and offers a wide range of alternatives.",
        "Synthesizer — combines the opinions into a compromise and lists the open issues.",
    ],
    rules: "## Discussion rules",
    read_only: "- **Read only**: you may read code and documents and research the web, but do not create, edit or delete files. Do not commit, push or install packages, and do not create or change tasks with `office-task`.",
    enforced: [
        "  In this session the CLI settings block editing tools.",
        "  In this session the CLI settings block only the editing tools; shell commands are not fully blocked. Do not change anything with shell commands either.",
        "  In this session the CLI settings do not block anything. You must keep this rule yourself.",
    ],
    flow: "- **Flow**: the moderator runs the discussion. In the first round every debater speaks at the same time without seeing the others. After that, people speak in turn when the moderator calls on them. The app passes on the other participants' new utterances each turn. Do not speak before a turn instruction arrives.",
    moderator_rules: "- **Moderator**: stay neutral and give no opinion of your own. Each turn, choose the next speaker and question, choose what to ask the person, or wrap up the discussion. Follow the format in the turn instruction.",
    debater_rules: "- **Debater**: speak from the perspective and stance on your role card. Give reasons and keep it short (about 300 words at most). Do not just agree; add new information or reasons.",
    submit: "## How to speak\n\nEach turn, decide your utterance (the moderator: your instruction), submit it with the command below and end your turn. Submitting again in the same turn replaces it. Only what you submit reaches the other participants.",
    submit_placeholder: "<utterance>",
    submit_after: "If you cannot run the command, write only your utterance in your final reply.",
    last_reply: "## How to speak\n\nIn this session the sandbox blocks writes, so the submit command cannot be used. Each turn your whole final reply is passed on as your utterance (the moderator: your instruction), so write only that in your final reply.",
    topic: "## Topic",
    context: "### Context",
    no_context: "No context was given.",
    cwd: "Working folder",
    home: "home folder",
    participants: "## Participants",
    moderator_mark: "moderator",
    you_mark: "you",
    missing_member: "(deleted member)",
    persona: "## Voice",
    persona_hint: "Below is this character's personality and way of speaking. Use it only for your tone; what you say follows the role card and the discussion rules.",
    turn: "## This turn",
    wait: "## Now\n\nDo not speak yet. Once you have read the above, do not submit anything; reply with the single line \"Ready\" and wait for a turn instruction.",
    truncated: "…(truncated)",
};

fn text(lang: Lang) -> &'static Text {
    match lang {
        Lang::Ko => &KO,
        Lang::En => &EN,
    }
}

fn stance_index(stance: DiscussionStance) -> usize {
    match stance {
        DiscussionStance::Moderator => 0,
        DiscussionStance::Free => 1,
        DiscussionStance::Supportive => 2,
        DiscussionStance::Critical => 3,
        DiscussionStance::DevilsAdvocate => 4,
        DiscussionStance::Divergent => 5,
        DiscussionStance::Synthesizer => 6,
    }
}

/// 팀원 한 명의 표시 정보(프로필에서).
#[derive(Debug, Clone, Default)]
pub struct Member {
    pub name: String,
    pub personality: Option<String>,
}

/// 첫 프롬프트 재료.
pub struct Briefing<'a> {
    pub discussion: &'a Discussion,
    /// 이 세션의 참가자(캐릭터) agentId.
    pub participant: &'a str,
    /// agentId → 팀원. 없으면 삭제된 팀원이다.
    pub members: &'a HashMap<String, Member>,
    pub roles: &'a [DiscussionRole],
    /// `office-discuss` shim 경로. 제출 방식 안내에 쓴다.
    pub shim: &'a Path,
    /// 실행기가 붙인 이번 차례 지시. 없으면 대기 지시로 끝난다.
    pub turn: Option<&'a str>,
}

fn member_name<'a>(t: &'a Text, members: &'a HashMap<String, Member>, agent_id: &str) -> &'a str {
    members.get(agent_id).map(|m| m.name.trim()).filter(|n| !n.is_empty()).unwrap_or(t.missing_member)
}

fn role_of<'a>(roles: &'a [DiscussionRole], role_id: Option<&str>) -> Option<&'a DiscussionRole> {
    role_id.and_then(|id| roles.iter().find(|r| r.id == id))
}

/// 첫 프롬프트. 칸 순서: 자리 → 역할 카드 → 규칙 → 발언 내는 법 → 주제 → 참가자 → 말투 → 이번 차례.
pub fn build_briefing(input: &Briefing, lang: Lang) -> String {
    let t = text(lang);
    let d = input.discussion;
    let me = d.participants.iter().find(|p| p.agent_id == input.participant);
    let moderator = d.moderator_agent_id == input.participant;
    let cli = me.map_or(WorkCli::Claude, |p| p.agent.cli.clone());
    let role = role_of(input.roles, me.and_then(|p| p.role_id.as_deref()));
    let mut sections = vec![];

    sections.push(format!(
        "{}\n\n{}\n{}",
        t.title,
        t.you_are.replace("{name}", member_name(t, input.members, input.participant)),
        if moderator { t.seat_moderator } else { t.seat_debater },
    ));

    let mut card = t.role.to_string();
    match role {
        Some(role) => {
            card.push_str(&format!("\n\n- {}: {}\n- {}: {}", t.role_name, role.name.trim(), t.stance, t.stances[stance_index(role.stance)]));
            for (label, value) in [(t.perspective, &role.perspective), (t.criteria, &role.criteria)] {
                if !value.trim().is_empty() {
                    card.push_str(&format!("\n\n### {label}\n\n{}", value.trim()));
                }
            }
        }
        None if moderator => card.push_str(&format!("\n\n- {}: {}", t.stance, t.stances[0])),
        None => card.push_str(&format!("\n\n{}", t.no_role)),
    }
    sections.push(card);

    let enforced = match read_only_mode(&cli) {
        DiscussionReadOnly::Flags => t.enforced[0],
        DiscussionReadOnly::Partial => t.enforced[1],
        DiscussionReadOnly::Prompt => t.enforced[2],
    };
    sections.push(format!(
        "{}\n\n{}\n{enforced}\n{}\n{}",
        t.rules,
        t.read_only,
        t.flow,
        if moderator { t.moderator_rules } else { t.debater_rules },
    ));

    sections.push(match collection_mode(&cli) {
        DiscussionCollection::Submit => format!(
            "{}\n\n```sh\n{} say - <<'EOF'\n{}\nEOF\n```\n\n{}",
            t.submit,
            input.shim.display(),
            t.submit_placeholder,
            t.submit_after,
        ),
        DiscussionCollection::LastReply => t.last_reply.to_string(),
    });

    let context = d.context.trim();
    let context = if context.is_empty() {
        t.no_context.to_string()
    } else if context.chars().count() > MAX_CONTEXT_CHARS {
        format!("{}\n{}", context.chars().take(MAX_CONTEXT_CHARS).collect::<String>(), t.truncated)
    } else {
        context.to_string()
    };
    sections.push(format!(
        "{}\n\n{}\n\n- {}: {}\n\n{}\n\n{context}",
        t.topic,
        d.topic.trim(),
        t.cwd,
        d.cwd.as_deref().unwrap_or(t.home),
        t.context,
    ));

    let mut list = t.participants.to_string();
    list.push('\n');
    for p in &d.participants {
        let mut marks = vec![];
        if let Some(role) = role_of(input.roles, p.role_id.as_deref()) {
            marks.push(role.name.trim().to_string());
        }
        if p.agent_id == d.moderator_agent_id {
            marks.push(t.moderator_mark.to_string());
        }
        if p.agent_id == input.participant {
            marks.push(t.you_mark.to_string());
        }
        let marks = if marks.is_empty() { String::new() } else { format!(" ({})", marks.join(", ")) };
        list.push_str(&format!("\n- {}{marks}", member_name(t, input.members, &p.agent_id)));
    }
    sections.push(list);

    if let Some(persona) = input.members.get(input.participant).and_then(|m| m.personality.as_deref()).map(str::trim).filter(|p| !p.is_empty()) {
        sections.push(format!("{}\n\n{}\n\n{persona}", t.persona, t.persona_hint));
    }

    sections.push(match input.turn.map(str::trim).filter(|s| !s.is_empty()) {
        Some(turn) => format!("{}\n\n{turn}", t.turn),
        None => t.wait.to_string(),
    });

    let out = crate::task_chat::sanitize(&sections.join("\n\n"));
    if out.len() <= MAX_PROMPT_BYTES {
        return out;
    }
    let mut cut = MAX_PROMPT_BYTES;
    while !out.is_char_boundary(cut) {
        cut -= 1;
    }
    out[..cut].to_string()
}

// ── 발언 수집 ────────────────────────────────────────────────────────

/// 끝에서 `max`자만 남긴다(앞에 `…`).
fn keep_tail(text: &str, max: usize) -> String {
    let count = text.chars().count();
    if count <= max {
        return text.to_string();
    }
    format!("…{}", text.chars().skip(count - max).collect::<String>())
}

/// 제출된 발언. `since`보다 먼저 쓰인 것은 지난 차례다.
pub fn read_submitted(app_data: &Path, agent_id: &str, since_ms: u64) -> Option<String> {
    let path = turn_file(app_data, agent_id)?;
    let modified = std::fs::symlink_metadata(&path).ok().filter(|m| m.is_file())?.modified().ok()?;
    let written_ms = modified.duration_since(std::time::UNIX_EPOCH).ok()?.as_millis();
    if written_ms < u128::from(since_ms) {
        return None;
    }
    let text = std::fs::read_to_string(&path).ok()?;
    let text = text.trim();
    (!text.is_empty()).then(|| keep_tail(text, MAX_UTTERANCE_CHARS))
}

/// 한 차례의 발언: 제출 → 마지막 응답 순. `last_reply`는 그 CLI의 마지막 응답을 `since` 이후의
/// 것만 돌려준다(Codex는 Stop 훅, Claude는 전사).
pub fn collect(
    app_data: &Path,
    agent_id: &str,
    since_ms: u64,
    last_reply: impl FnOnce() -> Option<String>,
) -> Option<CollectedUtterance> {
    if let Some(text) = read_submitted(app_data, agent_id, since_ms) {
        return Some(CollectedUtterance { text, source: DiscussionUtteranceSource::Submitted });
    }
    let text = last_reply()?;
    let text = text.trim();
    (!text.is_empty()).then(|| CollectedUtterance {
        text: keep_tail(text, MAX_UTTERANCE_CHARS),
        source: DiscussionUtteranceSource::LastReply,
    })
}

/// 토의 세션의 Codex Stop 훅 마지막 응답(세션 id별 최신 1건). Codex 전사 찾기는 cwd의
/// 최신 스레드를 고르므로 같은 폴더의 다른 Codex와 섞인다. 훅은 세션별로 온다.
pub struct DiscussionReplies {
    registry: Arc<crate::state::SessionRegistry>,
    replies: Mutex<HashMap<String, (u64, String)>>,
}

impl DiscussionReplies {
    pub fn new(registry: Arc<crate::state::SessionRegistry>) -> Self {
        Self { registry, replies: Mutex::new(HashMap::new()) }
    }

    /// `since` 이후에 온 그 세션의 마지막 응답.
    pub fn since(&self, session_id: &str, since_ms: u64) -> Option<String> {
        let replies = self.replies.lock().unwrap();
        let (at, text) = replies.get(session_id)?;
        (*at >= since_ms).then(|| text.clone())
    }

    fn record_at(&self, session_id: &str, text: &str, at: u64) {
        let is_discussion = self.registry.resolve_agent(session_id).is_some_and(|a| crate::types::is_discussion_session_id(&a));
        if !is_discussion || text.trim().is_empty() {
            return;
        }
        let mut replies = self.replies.lock().unwrap();
        replies.insert(session_id.to_string(), (at, keep_tail(text, MAX_UTTERANCE_CHARS)));
        while replies.len() > MAX_REPLIES {
            let Some(oldest) = replies.iter().min_by_key(|(_, (at, _))| *at).map(|(k, _)| k.clone()) else { break };
            replies.remove(&oldest);
        }
    }
}

impl crate::observer::StopReplySink for DiscussionReplies {
    fn record(&self, session_id: &str, text: &str) {
        self.record_at(session_id, text, crate::types::now_ms());
    }
}

/// Claude 전사의 마지막 응답. 전사가 `since` 전에 마지막으로 쓰였으면 이번 차례 응답이 없다.
fn claude_reply_since(
    lookup: Arc<dyn crate::session_log::agent_transcript::AgentSessionLookup>,
    agent_id: &str,
    cwd: &str,
    since_ms: u64,
) -> Option<String> {
    let mut source = crate::session_log::agent_transcript::claude::source(lookup)?;
    let path = source.locate(agent_id, cwd)?;
    let modified = std::fs::metadata(&path).ok()?.modified().ok()?;
    if modified.duration_since(std::time::UNIX_EPOCH).ok()?.as_millis() < u128::from(since_ms) {
        return None;
    }
    crate::auto_run::last_reply(source.as_mut(), agent_id, cwd)
}

// ── 열기·차례 보내기 ─────────────────────────────────────────────────

/// 같은 세션의 열기가 겹치지 않게 한다.
fn opening() -> &'static Mutex<HashSet<String>> {
    static OPENING: OnceLock<Mutex<HashSet<String>>> = OnceLock::new();
    OPENING.get_or_init(Default::default)
}
struct OpeningGuard(String);
impl OpeningGuard {
    fn acquire(agent_id: &str) -> Result<Self, String> {
        if !opening().lock().unwrap().insert(agent_id.to_string()) {
            return Err("discussion-session-busy".into());
        }
        Ok(Self(agent_id.to_string()))
    }
}
impl Drop for OpeningGuard {
    fn drop(&mut self) {
        opening().lock().unwrap().remove(&self.0);
    }
}

/// 열기 요청 검증. 통과하면 (세션 agentId, 토의).
pub fn validate(request: &DiscussionSessionRequest, discussions: Vec<Discussion>) -> Result<(String, Discussion), String> {
    let agent_id = session_agent_id(&request.discussion_id, &request.agent_id)?;
    let discussion = discussions
        .into_iter()
        .find(|d| d.id == request.discussion_id)
        .ok_or("discussion-session-missing")?;
    if discussion.state == crate::types::DiscussionState::Finished {
        return Err("discussion-session-finished".into());
    }
    let participant = discussion
        .participants
        .iter()
        .find(|p| p.agent_id == request.agent_id)
        .ok_or("discussion-session-not-participant")?;
    if discussion.cwd.as_deref().is_some_and(|cwd| !Path::new(cwd).is_absolute()) {
        return Err("discussion-session-invalid-cwd".into());
    }
    crate::work_send::validated_command(&participant.agent, crate::work_send::cli_transition::CommandSyntax::Posix)?;
    Ok((agent_id, discussion))
}

/// 참가자 세션을 만들고 첫 프롬프트를 넣는다. 실패하면 만든 세션을 버린다.
pub async fn open(app_state: &crate::state::AppState, request: DiscussionSessionRequest) -> Result<DiscussionSessionResult, String> {
    let (agent_id, discussion) = validate(&request, app_state.discussion_store.load()?)?;
    let _guard = OpeningGuard::acquire(&agent_id)?;
    let manager = &app_state.manager;
    if manager.is_running(&agent_id) {
        return Err("discussion-session-running".into());
    }
    if let Some(problem) = limit_problem(&manager.live_discussion_session_ids(), &agent_id) {
        return Err(problem.into());
    }
    let participant = discussion.participants.iter().find(|p| p.agent_id == request.agent_id).cloned().ok_or("discussion-session-not-participant")?;
    let profiles = app_state.store.load().agents;
    let members: HashMap<String, Member> = profiles
        .iter()
        .map(|p| (p.id.clone(), Member { name: p.name.clone(), personality: p.personality_prompt.clone() }))
        .collect();
    // 역할 목록이 없거나(아직 시드 전) 읽지 못하면 역할 없이 연다.
    let roles = app_state.role_store.load().ok().flatten().map(|f| f.roles).unwrap_or_default();
    let (lang, observed) = {
        let settings = app_state.settings.read().unwrap();
        (crate::i18n::ui_lang(&settings), settings.observer_enabled)
    };
    let shim = crate::discussion_cli::shim_path();
    let prompt = build_briefing(
        &Briefing {
            discussion: &discussion,
            participant: &request.agent_id,
            members: &members,
            roles: &roles,
            shim: &shim,
            turn: request.turn.as_deref(),
        },
        lang,
    );
    let profile = profiles.iter().find(|p| p.id == request.agent_id);
    let name = members.get(&request.agent_id).map(|m| m.name.clone()).unwrap_or_else(|| request.agent_id.clone());
    let role_name = role_of(&roles, participant.role_id.as_deref()).map(|r| r.name.clone());
    // 지난 토의에서 남은 제출 파일을 이번 차례로 읽지 않게 지운다.
    if let Some(path) = turn_file(&app_state.app_data_dir, &agent_id) {
        let _ = std::fs::remove_file(path);
    }
    let created = crate::ipc::commands::spawn_session(
        manager,
        &app_state.observer,
        &app_state.observer_server,
        &app_state.settings,
        crate::types::CreateSessionRequest {
            agent_id: agent_id.clone(),
            cols: request.cols,
            rows: request.rows,
            cwd: discussion.cwd.clone(),
            shell: profile.and_then(|p| p.shell.clone()),
            startup_command: None,
            personality_prompt: None,
            autostart_claude: None,
            task_chat: false,
        },
        crate::session_events::types::AgentEventProfile { name, role: role_name },
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
                agent: participant.agent.clone(),
                title: String::new(),
                body: prompt,
            },
        )
        .await;
    let submitted_at = match sent {
        Ok(sent) => sent.submitted_at,
        Err(error) => {
            let _ = manager.dispose_matching(&agent_id, Some(&session_id));
            return Err(error);
        }
    };
    Ok(DiscussionSessionResult {
        agent_id,
        session_id,
        submitted_at,
        read_only: read_only_mode(&participant.agent.cli),
        collection: collection_mode(&participant.agent.cli),
        observed,
    })
}

/// 실행 중인 참가자 CLI에 차례 지시 하나를 넣는다. 돌려준 `submitted_at`이 수집의 `since`다.
pub async fn send_turn(
    app_state: &crate::state::AppState,
    request: crate::types::WorkFollowUpRequest,
) -> Result<crate::types::WorkSendResult, String> {
    if parse_session_agent_id(&request.agent_id).is_none() {
        return Err("discussion-session-invalid-id".into());
    }
    app_state.work_send_runtime.follow_up(&app_state.gate, request).await
}

/// 한 차례의 발언을 모은다. 없으면 None(아직 말하지 않았거나 응답을 못 읽었다).
pub async fn collect_utterance(
    app_state: &crate::state::AppState,
    request: crate::types::DiscussionCollectRequest,
) -> Result<Option<CollectedUtterance>, String> {
    let (discussion_id, _) = parse_session_agent_id(&request.agent_id).ok_or("discussion-session-invalid-id")?;
    let cwd = app_state
        .discussion_store
        .load()?
        .into_iter()
        .find(|d| d.id == discussion_id)
        .and_then(|d| d.cwd)
        .unwrap_or_default();
    let app_data = app_state.app_data_dir.clone();
    let replies = app_state.discussion_replies.clone();
    let lookup: Arc<dyn crate::session_log::agent_transcript::AgentSessionLookup> = app_state.claude_resume_store.clone();
    tauri::async_runtime::spawn_blocking(move || {
        collect(&app_data, &request.agent_id, request.since, || match request.cli {
            WorkCli::Codex => replies.since(&request.session_id, request.since),
            WorkCli::Claude => claude_reply_since(lookup, &request.agent_id, &cwd, request.since),
            _ => None,
        })
    })
    .await
    .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::{DiscussionLimits, DiscussionParticipant, DiscussionState, WorkAgent};

    fn participant(agent_id: &str, role: Option<&str>, cli: WorkCli) -> DiscussionParticipant {
        DiscussionParticipant { agent_id: agent_id.into(), role_id: role.map(Into::into), agent: WorkAgent { cli, model: None, effort: None } }
    }
    fn discussion() -> Discussion {
        Discussion {
            id: "d1".into(),
            topic: "Pick a storage format".into(),
            context: "We store tasks as JSON today.".into(),
            cwd: Some("/work/repo".into()),
            participants: vec![
                participant("mod", Some("preset-moderator"), WorkCli::Claude),
                participant("hana", Some("critic"), WorkCli::Claude),
                participant("jun", None, WorkCli::Codex),
                participant("gone", Some("deleted-role"), WorkCli::Kilo),
            ],
            moderator_agent_id: "mod".into(),
            limits: DiscussionLimits::default(),
            state: DiscussionState::Ready,
            round: 0,
            transcript: vec![],
            decision_memo_path: None,
            cost_usd: 0.0,
            created_at: 1,
            updated_at: 1,
            started_at: None,
            ended_at: None,
            active_ms: None,
            active_since: None,
            run: None,
        }
    }
    fn roles() -> Vec<DiscussionRole> {
        vec![
            DiscussionRole { id: "preset-moderator".into(), name: "Moderator".into(), perspective: "Keep it on track.".into(), stance: DiscussionStance::Moderator, criteria: "No opinions.".into(), agent: None },
            DiscussionRole { id: "critic".into(), name: "Critic".into(), perspective: "Find risks.".into(), stance: DiscussionStance::Critical, criteria: "".into(), agent: None },
        ]
    }
    fn members() -> HashMap<String, Member> {
        HashMap::from([
            ("mod".to_string(), Member { name: "Mina".into(), personality: None }),
            ("hana".to_string(), Member { name: "Hana".into(), personality: Some("Talks like a pirate.".into()) }),
            ("jun".to_string(), Member { name: "Jun".into(), personality: None }),
        ])
    }
    fn brief(participant: &str, turn: Option<&str>, lang: Lang) -> String {
        let (d, m, r) = (discussion(), members(), roles());
        build_briefing(&Briefing { discussion: &d, participant, members: &m, roles: &r, shim: Path::new("/tmp/ao/office-discuss"), turn }, lang)
    }

    #[test]
    fn session_ids_round_trip_and_reject_unsafe_parts() {
        let id = session_agent_id("d1", "V1StGXR8_Z5jdHi6B-myT").unwrap();
        assert_eq!(id, "discuss-d1.V1StGXR8_Z5jdHi6B-myT");
        assert!(crate::types::is_discussion_session_id(&id) && crate::types::is_side_session_id(&id));
        assert!(!crate::types::is_task_chat_id(&id));
        assert_eq!(parse_session_agent_id(&id), Some(("d1", "V1StGXR8_Z5jdHi6B-myT")));
        // 토의 id에 `-`가 있어도 경계는 `.` 하나다.
        assert_eq!(parse_session_agent_id("discuss-a-b.c-d"), Some(("a-b", "c-d")));
        for (d, p) in [("", "a"), ("a", ""), ("a.b", "c"), ("a", "../x"), ("a", "b c")] {
            assert_eq!(session_agent_id(d, p).unwrap_err(), "discussion-session-invalid-id", "{d:?} {p:?}");
        }
        for bad in ["discuss-a", "discuss-a.b.c", "discuss-.b", "task-chat-a.b", "discuss-a./x"] {
            assert_eq!(parse_session_agent_id(bad), None, "{bad}");
            assert_eq!(turn_file(Path::new("/data"), bad), None, "{bad}");
        }
        assert_eq!(turn_file(Path::new("/data"), &id).unwrap(), PathBuf::from("/data/discussions/turns/discuss-d1.V1StGXR8_Z5jdHi6B-myT.md"));
    }

    #[test]
    fn limit_allows_five_of_one_discussion_and_blocks_another() {
        let live: Vec<String> = ["a", "b", "c", "d"].iter().map(|p| session_agent_id("d1", p).unwrap()).collect();
        assert_eq!(limit_problem(&live, &session_agent_id("d1", "e").unwrap()), None);
        let mut full = live.clone();
        full.push(session_agent_id("d1", "e").unwrap());
        assert_eq!(limit_problem(&full, &session_agent_id("d1", "f").unwrap()), Some("discussion-session-limit"));
        // 이미 떠 있는 자기 자신은 세지 않는다(같은 세션 다시 열기는 실행 중 검사가 막는다).
        assert_eq!(limit_problem(&full, &session_agent_id("d1", "e").unwrap()), None);
        assert_eq!(limit_problem(&live, &session_agent_id("d2", "a").unwrap()), Some("discussion-session-other"));
        assert_eq!(limit_problem(&[], &session_agent_id("d2", "a").unwrap()), None);
    }

    #[test]
    fn read_only_and_collection_follow_the_cli() {
        assert_eq!(read_only_mode(&WorkCli::Claude), DiscussionReadOnly::Partial);
        assert_eq!(read_only_mode(&WorkCli::Codex), DiscussionReadOnly::Flags);
        assert_eq!(read_only_mode(&WorkCli::Pi), DiscussionReadOnly::Partial);
        assert_eq!(read_only_mode(&WorkCli::Agy), DiscussionReadOnly::Prompt);
        assert_eq!(read_only_mode(&WorkCli::Kilo), DiscussionReadOnly::Prompt);
        assert_eq!(collection_mode(&WorkCli::Codex), DiscussionCollection::LastReply);
        for cli in [WorkCli::Claude, WorkCli::Agy, WorkCli::Kilo, WorkCli::Pi] {
            assert_eq!(collection_mode(&cli), DiscussionCollection::Submit);
        }
        let args = read_only_args(Some(Path::new("/tmp/ao/office-discuss")));
        let literal = |cmd: &str| -> Vec<String> {
            args.iter().find(|(c, _)| *c == cmd).unwrap().1.iter().map(|a| match a { WrapperArg::Literal(s) | WrapperArg::Env(s) => s.clone() }).collect()
        };
        assert_eq!(literal("claude")[4..], ["--permission-mode", "dontAsk"], "값 하나짜리 옵션이 끝");
        assert!(literal("claude")[1].ends_with("Bash(/tmp/ao/office-discuss:*)"));
        assert_eq!(literal("claude")[2..4], ["--disallowed-tools", "Edit Write NotebookEdit"]);
        assert_eq!(literal("codex")[..4], ["--sandbox", "read-only", "--ask-for-approval", "never"]);
        assert_eq!(literal("pi"), ["--exclude-tools", "edit,write"]);
        assert!(args.iter().all(|(c, _)| !matches!(*c, "agy" | "kilo")));
    }

    #[test]
    fn debater_briefing_has_role_card_rules_submit_command_topic_and_voice_in_order() {
        let out = brief("hana", None, Lang::En);
        let at = |needle: &str| out.find(needle).unwrap_or_else(|| panic!("missing {needle:?} in\n{out}"));
        let order = [at("# Team discussion"), at("## Role card"), at("## Discussion rules"), at("## How to speak"), at("## Topic"), at("## Participants"), at("## Voice"), at("## Now")];
        assert!(order.windows(2).all(|w| w[0] < w[1]), "{order:?}");
        assert!(out.contains("**Hana**") && out.contains("**debater**"));
        assert!(out.contains("- Role: Critic") && out.contains("Critical —") && out.contains("Find risks."));
        assert!(!out.contains("### Criteria"), "empty criteria is left out");
        assert!(out.contains(EN.enforced[1]), "claude blocks only the editing tools");
        assert!(out.contains("/tmp/ao/office-discuss say - <<'EOF'"));
        assert!(out.contains("Pick a storage format") && out.contains("We store tasks as JSON today.") && out.contains("/work/repo"));
        assert!(out.contains("- Mina (Moderator, moderator)") && out.contains("- Hana (Critic, you)") && out.contains("- Jun\n"));
        // 지워진 역할·삭제된 팀원은 이름 없이 표시한다.
        assert!(out.contains("- (deleted member)"));
        assert!(out.contains("Talks like a pirate."));
        assert!(out.contains(EN.debater_rules) && !out.contains(EN.moderator_rules));
    }

    #[test]
    fn moderator_briefing_is_neutral_and_codex_speaks_by_final_reply() {
        let out = brief("mod", Some("Open the discussion."), Lang::En);
        assert!(out.contains("**moderator**") && out.contains(EN.moderator_rules));
        assert!(out.ends_with("## This turn\n\nOpen the discussion."));
        assert!(!out.contains("## Voice"));

        let codex = brief("jun", None, Lang::En);
        assert!(codex.contains(EN.last_reply) && !codex.contains("office-discuss say"));
        assert!(codex.contains(EN.enforced[0]), "codex sandbox blocks writes");
        assert!(codex.contains(EN.no_role));
        let kilo = brief("gone", None, Lang::En);
        assert!(kilo.contains(EN.enforced[2]), "kilo is prompt-only");
        assert!(kilo.contains(EN.no_role), "a deleted role reads as no role");
    }

    #[test]
    fn korean_wording_and_control_characters_are_cleaned() {
        let mut d = discussion();
        d.context = "line\r\nnext \x1b[201~echo injected\0".into();
        let (m, r) = (members(), roles());
        let out = build_briefing(&Briefing { discussion: &d, participant: "hana", members: &m, roles: &r, shim: Path::new("/s"), turn: None }, Lang::Ko);
        assert!(out.starts_with("# 팀 토의"));
        assert!(out.contains("비판 —") && out.contains("## 말투"));
        assert!(out.contains("line\nnext [201~echo injected"));
        assert!(!out.chars().any(|c| c.is_control() && !matches!(c, '\n' | '\t')));
    }

    #[test]
    fn long_context_is_cut_and_prompt_stays_under_the_limit() {
        let mut d = discussion();
        d.context = "가".repeat(MAX_CONTEXT_CHARS * 2);
        let (m, r) = (members(), roles());
        let out = build_briefing(&Briefing { discussion: &d, participant: "hana", members: &m, roles: &r, shim: Path::new("/s"), turn: None }, Lang::En);
        assert!(out.len() <= MAX_PROMPT_BYTES);
        assert!(out.contains(EN.truncated) && out.contains("## Participants"));
    }

    fn set_mtime(path: &Path, ms: u64) {
        let time = std::time::UNIX_EPOCH + std::time::Duration::from_millis(ms);
        std::fs::File::options().write(true).open(path).unwrap().set_modified(time).unwrap();
    }

    #[test]
    fn collect_prefers_the_submission_of_this_turn_then_the_last_reply() {
        let tmp = tempfile::tempdir().unwrap();
        let id = session_agent_id("d1", "hana").unwrap();
        let file = turn_file(tmp.path(), &id).unwrap();
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(&file, "  my point\n").unwrap();
        set_mtime(&file, 5_000);

        let got = collect(tmp.path(), &id, 4_000, || panic!("not needed")).unwrap();
        assert_eq!(got, CollectedUtterance { text: "my point".into(), source: DiscussionUtteranceSource::Submitted });
        // 지난 차례의 제출은 읽지 않고 마지막 응답으로 넘어간다.
        let got = collect(tmp.path(), &id, 6_000, || Some(" reply ".into())).unwrap();
        assert_eq!(got, CollectedUtterance { text: "reply".into(), source: DiscussionUtteranceSource::LastReply });
        assert_eq!(collect(tmp.path(), &id, 6_000, || None), None);
        assert_eq!(collect(tmp.path(), &id, 6_000, || Some("  ".into())), None);
        // 다른 참가자의 파일은 섞이지 않는다.
        assert_eq!(collect(tmp.path(), &session_agent_id("d1", "jun").unwrap(), 0, || None), None);
    }

    #[test]
    fn long_utterance_keeps_its_end() {
        let tmp = tempfile::tempdir().unwrap();
        let id = session_agent_id("d1", "hana").unwrap();
        let long = format!("{}END", "x".repeat(MAX_UTTERANCE_CHARS));
        let got = collect(tmp.path(), &id, 0, || Some(long)).unwrap();
        assert!(got.text.starts_with('…') && got.text.ends_with("END"));
        assert_eq!(got.text.chars().count(), MAX_UTTERANCE_CHARS + 1);
    }

    #[test]
    fn codex_replies_are_kept_per_discussion_session_only() {
        let registry = Arc::new(crate::state::SessionRegistry::new());
        registry.insert("s-disc", &session_agent_id("d1", "jun").unwrap(), crate::types::SessionState::Running);
        registry.insert("s-char", "jun", crate::types::SessionState::Running);
        let replies = DiscussionReplies::new(registry);
        replies.record_at("s-disc", "first", 1_000);
        replies.record_at("s-char", "character turn", 1_000);
        replies.record_at("s-unknown", "x", 1_000);
        assert_eq!(replies.since("s-disc", 1_000).as_deref(), Some("first"));
        assert_eq!(replies.since("s-disc", 1_001), None, "older than this turn");
        assert_eq!(replies.since("s-char", 0), None);
        assert_eq!(replies.since("s-unknown", 0), None);
        replies.record_at("s-disc", "second", 2_000);
        assert_eq!(replies.since("s-disc", 1_500).as_deref(), Some("second"));
    }

    fn request(discussion_id: &str, agent_id: &str) -> DiscussionSessionRequest {
        serde_json::from_value(serde_json::json!({ "discussionId": discussion_id, "agentId": agent_id })).unwrap()
    }

    #[test]
    fn validation_finds_the_participant_and_rejects_bad_requests() {
        let (id, d) = validate(&request("d1", "hana"), vec![discussion()]).unwrap();
        assert_eq!(id, "discuss-d1.hana");
        assert_eq!(d.id, "d1");
        assert_eq!(validate(&request("d2", "hana"), vec![discussion()]).unwrap_err(), "discussion-session-missing");
        assert_eq!(validate(&request("d1", "zed"), vec![discussion()]).unwrap_err(), "discussion-session-not-participant");
        assert_eq!(validate(&request("d.1", "hana"), vec![discussion()]).unwrap_err(), "discussion-session-invalid-id");
        let mut finished = discussion();
        finished.state = DiscussionState::Finished;
        assert_eq!(validate(&request("d1", "hana"), vec![finished]).unwrap_err(), "discussion-session-finished");
        let mut relative = discussion();
        relative.cwd = Some("repo".into());
        assert_eq!(validate(&request("d1", "hana"), vec![relative]).unwrap_err(), "discussion-session-invalid-cwd");
        let mut bad_model = discussion();
        bad_model.participants[1].agent.model = Some("x\ny".into());
        assert_eq!(validate(&request("d1", "hana"), vec![bad_model]).unwrap_err(), "work-send-model-invalid");
        // 렌더러가 모르는 칸을 보내면 거절한다.
        assert!(serde_json::from_value::<DiscussionSessionRequest>(serde_json::json!({ "discussionId": "d1", "agentId": "a", "prompt": "raw" })).is_err());
    }

    #[test]
    fn same_session_cannot_be_opened_twice_at_once() {
        let first = OpeningGuard::acquire("discuss-guard.test").unwrap();
        assert_eq!(OpeningGuard::acquire("discuss-guard.test").err().unwrap(), "discussion-session-busy");
        drop(first);
        assert!(OpeningGuard::acquire("discuss-guard.test").is_ok());
    }
}
