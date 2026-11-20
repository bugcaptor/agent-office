//! 토의 참가자 세션: 참가자마다 캐릭터 본 터미널이 아닌 토의 전용 탭을 열고 첫 프롬프트
//! (역할 카드 + 토의 규칙 + 주제·맥락)를 넣는다. 태스크 대화와 달리 관찰 훅을 붙여 턴 종료를
//! 받고(`control::turns::TurnTracker`, 렌더러의 알림·사용량 이벤트), 읽기 전용은 프롬프트 규칙으로만
//! 지킨다. 권한 플래그로 막으면 제출 명령까지 막혀 진행이 멈추기 때문이다(아래 `session_args`).
//! Claude·Codex는 턴의 마지막 응답(Stop 훅이 실어 온 것, 없으면 Claude 전사)을 발언으로 받고,
//! 나머지는 `office-discuss say`로 제출받는다.
//! 진행(누가 언제 말하나)은 실행기가 쥔다. 여기는 열기·차례 보내기·발언 수집까지다.

use crate::i18n::Lang;
use crate::observer::WrapperArg;
use crate::types::{
    CollectedUtterance, Discussion, DiscussionCollection, DiscussionRole, DiscussionSessionRequest,
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
/// 기억해 둘 Stop 훅 마지막 응답 수. 넘으면 오래된 것부터 버린다.
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

/// 발언을 어떻게 받는가. 마지막 응답을 읽을 수 있는 CLI는 그것을 쓴다. 제출 명령은 셸 호출이라
/// 사용자 권한 설정에 걸릴 수 있다(실측: Claude `dontAsk`는 본문에 `{"`가 있는 heredoc을
/// 허용 규칙과 무관하게 거절해 사회자 JSON 지시가 매번 막혔다). Codex 기본 workspace-write
/// 샌드박스도 앱 데이터 쓰기를 막는다.
pub fn collection_mode(cli: &WorkCli) -> DiscussionCollection {
    match cli {
        WorkCli::Claude | WorkCli::Codex => DiscussionCollection::LastReply,
        _ => DiscussionCollection::Submit,
    }
}

/// 토의 세션 셸 래퍼에 붙일 인자(명령별). 권한·도구를 좁히는 플래그는 싣지 않는다. 읽기 전용은
/// 첫 프롬프트 규칙으로 지킨다. Claude는 `auto`로 띄워 사람 없이 도는 차례가 승인 대기로 멈추지 않게 하고
/// (`claude` 기본 모드는 웹 조사·셸마다 묻는다), Codex는 웹 조사만 연다. agy·kilo·pi는 업무 보내기
/// 명령(`launch_command_for`)이 이미 자동 승인 플래그를 붙인다.
pub fn session_args() -> Vec<(&'static str, Vec<WrapperArg>)> {
    let lit = |s: &str| WrapperArg::Literal(s.to_string());
    vec![
        ("claude", vec![lit("--permission-mode"), lit("auto")]),
        ("codex", vec![lit("--search")]),
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
    submit_exception: &'static str,
    flow: &'static str,
    moderator_rules: &'static str,
    debater_rules: &'static str,
    submit: &'static str,
    submit_placeholder: &'static str,
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
    read_only: "- **읽기 전용**: 이 토의에서는 코드·문서를 읽고 웹을 조사하는 것만 합니다. 이 세션의 CLI는 평소 권한 그대로 돌기 때문에 이 규칙은 스스로 지켜야 합니다.\n  - 파일을 만들거나 고치거나 지우지 마세요. 편집 도구뿐 아니라 셸로 쓰는 것(`>`·`>>` 리다이렉션, `tee`, `sed -i`, `mv`, `rm`, `touch`, `mkdir` 등)도 하지 마세요.\n  - 커밋·푸시·브랜치 전환·`git stash`·패키지 설치·빌드·서버 실행처럼 저장소나 환경을 바꾸는 명령을 실행하지 마세요. `office-task`로 업무를 만들거나 바꾸지 마세요.\n  - 셸은 `cat`·`rg`·`ls`·`git log`·`git diff`·`git show`처럼 읽기만 하는 명령에 쓰세요. 무언가를 바꿔야 결론이 난다면 직접 하지 말고 발언에서 제안하세요.",
    submit_exception: "  - 아래 발언 제출 명령은 이 규칙의 예외입니다.",
    flow: "- **진행**: 사회자가 진행합니다. 첫 라운드는 토론자 모두가 서로의 의견을 보지 않고 동시에 발언합니다. 그다음부터는 사회자가 지목한 사람이 차례로 말합니다. 다른 참가자의 새 발언은 앱이 차례마다 전해 줍니다. 차례 지시가 오기 전에는 발언하지 마세요.",
    moderator_rules: "- **사회자**: 중립을 지키고 자기 의견을 내지 마세요. 차례마다 다음 발언자와 질문을 정하거나, 사람에게 물을 것을 정하거나, 토의를 정리합니다. 지시 형식은 차례 지시를 따르세요.",
    debater_rules: "- **토론자**: 역할 카드의 관점과 스탠스로 말하세요. 근거를 대고 짧게(대략 300단어 이내) 말하세요. 동의만 하지 말고 새 정보나 이유를 보태세요.",
    submit: "## 발언 내는 법\n\n차례마다 발언(사회자는 지시)을 정한 뒤 아래 명령으로 제출하고 차례를 마치세요. 같은 차례에 다시 내면 바뀝니다. 제출한 내용만 다른 참가자에게 전해집니다.",
    submit_placeholder: "<발언>",
    last_reply: "## 발언 내는 법\n\n따로 제출하는 명령은 없습니다. 차례마다 마지막 답변 전체가 그대로 발언(사회자는 지시)으로 전해집니다. 조사는 도구로 하고, 마지막 답변에는 발언만 쓰세요. 작업 과정 설명, 인사, \"제출했습니다\" 같은 말은 붙이지 마세요.",
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
    read_only: "- **Read only**: in this discussion you only read code and documents and research the web. This session's CLI runs with its usual permissions, so you must keep this rule yourself.\n  - Do not create, edit or delete files. That covers the editing tools and also writing from the shell (`>`/`>>` redirection, `tee`, `sed -i`, `mv`, `rm`, `touch`, `mkdir` and so on).\n  - Do not run commands that change the repository or the environment, such as commit, push, switching branches, `git stash`, installing packages, building or starting servers. Do not create or change tasks with `office-task`.\n  - Use the shell only for commands that just read, such as `cat`, `rg`, `ls`, `git log`, `git diff` and `git show`. If reaching a conclusion would need a change, propose it in your utterance instead of making it.",
    submit_exception: "  - The submit command below is the one exception to this rule.",
    flow: "- **Flow**: the moderator runs the discussion. In the first round every debater speaks at the same time without seeing the others. After that, people speak in turn when the moderator calls on them. The app passes on the other participants' new utterances each turn. Do not speak before a turn instruction arrives.",
    moderator_rules: "- **Moderator**: stay neutral and give no opinion of your own. Each turn, choose the next speaker and question, choose what to ask the person, or wrap up the discussion. Follow the format in the turn instruction.",
    debater_rules: "- **Debater**: speak from the perspective and stance on your role card. Give reasons and keep it short (about 300 words at most). Do not just agree; add new information or reasons.",
    submit: "## How to speak\n\nEach turn, decide your utterance (the moderator: your instruction), submit it with the command below and end your turn. Submitting again in the same turn replaces it. Only what you submit reaches the other participants.",
    submit_placeholder: "<utterance>",
    last_reply: "## How to speak\n\nThere is no command to submit with. Each turn your whole final reply is passed on as it is as your utterance (the moderator: your instruction). Do your research with tools and write only the utterance in your final reply. Do not add notes about your work, greetings or remarks like \"submitted\".",
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
    /// 실행기가 붙인 이번 차례 지시.
    pub turn: &'a str,
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

    let collection = collection_mode(&cli);
    let exception = match collection {
        DiscussionCollection::Submit => format!("\n{}", t.submit_exception),
        DiscussionCollection::LastReply => String::new(),
    };
    sections.push(format!(
        "{}\n\n{}{exception}\n{}\n{}",
        t.rules,
        t.read_only,
        t.flow,
        if moderator { t.moderator_rules } else { t.debater_rules },
    ));

    sections.push(match collection {
        DiscussionCollection::Submit => format!(
            "{}\n\n```sh\n{} say - <<'EOF'\n{}\nEOF\n```",
            t.submit,
            input.shim.display(),
            t.submit_placeholder,
        ),
        DiscussionCollection::LastReply => t.last_reply.to_string(),
    });

    let topic = format!(
        "{}\n\n{}\n\n- {}: {}\n\n{}\n\n",
        t.topic,
        d.topic.trim(),
        t.cwd,
        d.cwd.as_deref().unwrap_or(t.home),
        t.context,
    );
    // 맥락은 나머지 칸이 다 정해진 뒤에 남는 자리만큼 싣는다(아래).
    let context_at = sections.len();
    sections.push(topic);

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

    sections.push(format!("{}\n\n{}", t.turn, input.turn.trim()));

    // 상한을 넘으면 맥락을 줄인다. 이번 차례(기록·지시)는 맨 끝 칸이라 뒤를 자르면 지시가 사라진다.
    let sanitized: Vec<String> = sections.iter().map(|s| crate::task_chat::sanitize(s)).collect();
    let fixed = sanitized.iter().map(String::len).sum::<usize>() + 2 * (sanitized.len() - 1);
    let context = crate::task_chat::sanitize(d.context.trim());
    let room = MAX_PROMPT_BYTES.saturating_sub(fixed);
    let context = if context.is_empty() {
        t.no_context.to_string()
    } else if context.chars().count() <= MAX_CONTEXT_CHARS && context.len() <= room {
        context
    } else {
        let budget = room.saturating_sub(t.truncated.len() + 1);
        let mut kept = String::new();
        for c in context.chars().take(MAX_CONTEXT_CHARS) {
            if kept.len() + c.len_utf8() > budget {
                break;
            }
            kept.push(c);
        }
        format!("{kept}\n{}", t.truncated)
    };
    let mut sanitized = sanitized;
    sanitized[context_at].push_str(&context);
    let out = sanitized.join("\n\n");
    if out.len() <= MAX_PROMPT_BYTES {
        return out;
    }
    // 맥락을 비워도 넘치면(실행기가 차례를 예산 안에 맞추므로 드물다) 차례 칸은 두고 그 앞을 자른다.
    let turn = sanitized.pop().unwrap_or_default();
    let head = sanitized.join("\n\n");
    let mut cut = MAX_PROMPT_BYTES.saturating_sub(turn.len() + 2).min(head.len());
    while !head.is_char_boundary(cut) {
        cut -= 1;
    }
    let out = format!("{}\n\n{turn}", &head[..cut]);
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
/// 것만 돌려준다(Stop 훅, Claude는 훅에 없으면 전사).
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

/// 토의 세션의 Stop 훅 마지막 응답(세션 id별 최신 1건, Codex·Claude). 훅은 세션별로 오고 턴 종료
/// 신호보다 먼저 기록된다. 전사로 읽으면 Codex는 cwd의 최신 스레드라 같은 폴더의 다른 Codex와 섞이고,
/// Claude는 턴 종료 신호 직후 전사에 마지막 응답이 아직 안 써져 있을 수 있다.
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

/// Claude 전사의 마지막 응답(Stop 훅에 응답이 없을 때 — 필드가 없는 옛 CLI). 전사가 `since` 전에
/// 마지막으로 쓰였으면 이번 차례 응답이 없다.
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
    if matches!(discussion.state, crate::types::DiscussionState::Finished | crate::types::DiscussionState::Review) {
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
        // 실행기가 차례 지시와 같은 언어를 실어 보낸다. 없으면(옛 렌더러) 화면 언어.
        let lang = request.lang.as_deref().map(crate::i18n::resolve_lang).unwrap_or_else(|| crate::i18n::ui_lang(&settings));
        (lang, settings.observer_enabled)
    };
    let shim = crate::discussion_cli::shim_path();
    let prompt = build_briefing(
        &Briefing {
            discussion: &discussion,
            participant: &request.agent_id,
            members: &members,
            roles: &roles,
            shim: &shim,
            turn: &request.turn,
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

/// 비었을 때 다시 모으기 전 기다림. 전사·제출 파일이 턴 종료 신호보다 늦게 써지는 경우를 넘긴다.
const RECOLLECT_DELAY: std::time::Duration = std::time::Duration::from_millis(1_500);

/// 한 차례의 발언을 모은다. 없으면 잠깐 기다려 한 번 더 본다. 그래도 없으면 None(아직 말하지 않았거나
/// 응답을 못 읽었다). 실행기는 None이면 재촉하고, 또 None이면 멈추므로 여기서 한 번 더 보는 값이 크다.
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
    let attempt = || {
        let (app_data, replies, request, cwd) = (app_state.app_data_dir.clone(), app_state.discussion_replies.clone(), request.clone(), cwd.clone());
        let lookup: Arc<dyn crate::session_log::agent_transcript::AgentSessionLookup> = app_state.claude_resume_store.clone();
        tauri::async_runtime::spawn_blocking(move || {
            collect(&app_data, &request.agent_id, request.since, || match request.cli {
                WorkCli::Codex => replies.since(&request.session_id, request.since),
                WorkCli::Claude => replies
                    .since(&request.session_id, request.since)
                    .or_else(|| claude_reply_since(lookup, &request.agent_id, &cwd, request.since)),
                _ => None,
            })
        })
    };
    if let Some(got) = attempt().await.map_err(|e| e.to_string())? {
        return Ok(Some(got));
    }
    tokio::time::sleep(RECOLLECT_DELAY).await;
    attempt().await.map_err(|e| e.to_string())
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
            cwd: Some("//office/work/repo".into()),
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
    fn brief(participant: &str, turn: &str, lang: Lang) -> String {
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
    fn collection_follows_the_cli_and_no_permission_flags_are_added() {
        for cli in [WorkCli::Claude, WorkCli::Codex] {
            assert_eq!(collection_mode(&cli), DiscussionCollection::LastReply);
        }
        for cli in [WorkCli::Agy, WorkCli::Kilo, WorkCli::Pi] {
            assert_eq!(collection_mode(&cli), DiscussionCollection::Submit);
        }
        // 권한·도구를 좁히는 플래그는 제출 명령까지 막는다. Claude auto와 Codex 웹 조사만 남는다.
        let lit = |s: &str| WrapperArg::Literal(s.into());
        assert_eq!(session_args(), vec![("claude", vec![lit("--permission-mode"), lit("auto")]), ("codex", vec![lit("--search")])]);
    }

    #[test]
    fn debater_briefing_has_role_card_rules_how_to_speak_topic_and_voice_in_order() {
        let out = brief("hana", "Speak.", Lang::En);
        let at = |needle: &str| out.find(needle).unwrap_or_else(|| panic!("missing {needle:?} in\n{out}"));
        let order = [at("# Team discussion"), at("## Role card"), at("## Discussion rules"), at("## How to speak"), at("## Topic"), at("## Participants"), at("## Voice"), at("## This turn")];
        assert!(order.windows(2).all(|w| w[0] < w[1]), "{order:?}");
        assert!(out.contains("**Hana**") && out.contains("**debater**"));
        assert!(out.contains("- Role: Critic") && out.contains("Critical —") && out.contains("Find risks."));
        assert!(!out.contains("### Criteria"), "empty criteria is left out");
        // Claude는 마지막 응답이 발언이라 제출 명령도, 그 예외 문구도 없다.
        assert!(out.contains(EN.read_only) && out.contains(EN.last_reply));
        assert!(!out.contains("office-discuss say") && !out.contains(EN.submit_exception));
        assert!(out.contains("Pick a storage format") && out.contains("We store tasks as JSON today.") && out.contains("/work/repo"));
        assert!(out.contains("- Mina (Moderator, moderator)") && out.contains("- Hana (Critic, you)") && out.contains("- Jun\n"));
        // 지워진 역할·삭제된 팀원은 이름 없이 표시한다.
        assert!(out.contains("- (deleted member)"));
        assert!(out.contains("Talks like a pirate."));
        assert!(out.contains(EN.debater_rules) && !out.contains(EN.moderator_rules));
    }

    #[test]
    fn moderator_briefing_is_neutral_and_only_submit_clis_get_the_command() {
        let out = brief("mod", "Open the discussion.", Lang::En);
        assert!(out.contains("**moderator**") && out.contains(EN.moderator_rules));
        assert!(out.ends_with("## This turn\n\nOpen the discussion."));
        assert!(!out.contains("## Voice"));

        let codex = brief("jun", "Speak.", Lang::En);
        assert!(codex.contains(EN.last_reply) && !codex.contains("office-discuss say"));
        assert!(codex.contains(EN.read_only) && codex.contains(EN.no_role));
        // 마지막 응답을 못 읽는 CLI는 제출 명령으로 내고, 그 명령만 읽기 전용의 예외다.
        let kilo = brief("gone", "Speak.", Lang::En);
        assert!(kilo.contains("/tmp/ao/office-discuss say - <<'EOF'") && kilo.contains(EN.submit_exception));
        assert!(kilo.contains(EN.no_role), "a deleted role reads as no role");
    }

    #[test]
    fn korean_wording_and_control_characters_are_cleaned() {
        let mut d = discussion();
        d.context = "line\r\nnext \x1b[201~echo injected\0".into();
        let (m, r) = (members(), roles());
        let out = build_briefing(&Briefing { discussion: &d, participant: "hana", members: &m, roles: &r, shim: Path::new("/s"), turn: "Speak." }, Lang::Ko);
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
        let out = build_briefing(&Briefing { discussion: &d, participant: "hana", members: &m, roles: &r, shim: Path::new("/s"), turn: "Speak." }, Lang::En);
        assert!(out.len() <= MAX_PROMPT_BYTES);
        assert!(out.contains(EN.truncated) && out.contains("## Participants"));
    }

    #[test]
    fn a_long_turn_shrinks_the_context_and_keeps_the_turn_whole() {
        let mut d = discussion();
        d.context = "가".repeat(MAX_CONTEXT_CHARS);
        let (m, r) = (members(), roles());
        // 기록이 긴 새 세션 차례(렌더러 예산 192KB). 끝은 차례 지시다.
        let turn = format!("{}\n\nNOW SPEAK", "나".repeat(64 * 1024));
        let out = build_briefing(&Briefing { discussion: &d, participant: "hana", members: &m, roles: &r, shim: Path::new("/s"), turn: &turn }, Lang::En);
        assert!(out.len() <= MAX_PROMPT_BYTES);
        assert!(out.ends_with("NOW SPEAK"));
        assert!(out.contains(EN.truncated) && out.contains("## Participants") && out.contains(&turn));
        // 차례만으로 상한을 넘어도 차례 칸은 끝까지 남긴다.
        let huge = format!("{}NOW", "x".repeat(MAX_PROMPT_BYTES - 1024));
        let out = build_briefing(&Briefing { discussion: &d, participant: "hana", members: &m, roles: &r, shim: Path::new("/s"), turn: &huge }, Lang::En);
        assert!(out.len() <= MAX_PROMPT_BYTES && out.ends_with("NOW"));
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
        serde_json::from_value(serde_json::json!({ "discussionId": discussion_id, "agentId": agent_id, "turn": "Speak." })).unwrap()
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
        // 결론 확인을 기다리는 토의도 탭을 다시 열지 않는다.
        let mut review = discussion();
        review.state = DiscussionState::Review;
        assert_eq!(validate(&request("d1", "hana"), vec![review]).unwrap_err(), "discussion-session-finished");
        let mut relative = discussion();
        relative.cwd = Some("repo".into());
        assert_eq!(validate(&request("d1", "hana"), vec![relative]).unwrap_err(), "discussion-session-invalid-cwd");
        let mut bad_model = discussion();
        bad_model.participants[1].agent.model = Some("x\ny".into());
        assert_eq!(validate(&request("d1", "hana"), vec![bad_model]).unwrap_err(), "work-send-model-invalid");
        // 렌더러가 모르는 칸을 보내거나 차례 지시를 빼면 거절한다.
        assert!(serde_json::from_value::<DiscussionSessionRequest>(serde_json::json!({ "discussionId": "d1", "agentId": "a", "turn": "t", "prompt": "raw" })).is_err());
        assert!(serde_json::from_value::<DiscussionSessionRequest>(serde_json::json!({ "discussionId": "d1", "agentId": "a" })).is_err());
    }

    #[test]
    fn same_session_cannot_be_opened_twice_at_once() {
        let first = OpeningGuard::acquire("discuss-guard.test").unwrap();
        assert_eq!(OpeningGuard::acquire("discuss-guard.test").err().unwrap(), "discussion-session-busy");
        drop(first);
        assert!(OpeningGuard::acquire("discuss-guard.test").is_ok());
    }
}
