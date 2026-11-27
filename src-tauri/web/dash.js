"use strict";
(() => {
const L = (navigator.language || "ko").toLowerCase().startsWith("ko") ? "ko" : "en";
const S = {
  ko: {
    tabs: { chars: "캐릭터", tasks: "업무", talks: "토의", usage: "사용량" },
    theme: { auto: "◐ 자동", light: "☀ 밝게", dark: "☾ 어둡게", hint: (now) => `화면 테마: ${now} · 눌러서 바꾸기(자동은 기기 설정을 따름)` },
    phase: { working: "작업 중", "needs-human": "답을 기다림", idle: "입력 대기", ended: "종료됨", starting: "시작 중", off: "꺼짐", running: "켜짐" },
    clockedOut: "퇴근", back: "← 목록", log: "대화 로그", noLog: "이 터미널에는 읽을 대화 로그가 없습니다(Claude·Codex·Antigravity 전사가 없는 세션).", noItems: "아직 대화가 없습니다.",
    restart: "터미널 재시작", resume: "이어하기", terminate: "종료", interrupt: "작업 중단(Esc)", ctrlc: "Ctrl-C", launch: "CLI 실행", launchCli: "띄울 CLI",
    again: "한 번 더 눌러 확인", done: "보냈습니다", saved: "저장했습니다", deleted: "삭제했습니다", result: "결과", lines: "줄",
    sentTasks: "보낸 업무", taskState: { in_progress: "진행 중", review: "끝남 · 확인 대기" }, markDone: "완료", markedDone: "완료로 옮겼습니다",
    workNow: "업무 진행 중", workStep: (n, total) => `단계 ${n}/${total}`, workDone: (d, total) => `${d}/${total} 완료`, workAuto: { running: "자동 진행", paused: "자동 진행 멈춤" }, workOpen: "눌러서 업무 탭에서 이 업무 보기",
    sendTask: "업무 보내기", titlePh: "할 일(제목)", bodyPh: "자세한 내용(선택)", cli: "CLI", send: "보내기", restartSend: "재시작하고 보내기",
    office: "일반 에이전트", officeSub: "캐릭터 없음", officeHelp: "캐릭터 없이 Agent Office 안내가 다 들어간 에이전트입니다. 업무·토의를 만들거나 앱을 다루는 일을 맡길 수 있습니다.",
    instruct: "지시하기", instructPh: "에이전트에게 시킬 일", folder: "작업 폴더", home: "홈",
    followUp: "이어서 보내기", newSession: "새 세션으로 보내기", newWarn: "새 세션으로 보내면 지금 세션은 끊기고 고른 작업 폴더에서 새로 엽니다.",
    officeWorking: "작업 중입니다. 지금 쓰면 대기열에 쌓았다가 턴이 끝나면 이어서 보냅니다.", officeHuman: "답을 기다립니다. 위 카드나 앱에서 답하세요. 지금 쓰면 대기열에 쌓습니다.", officeShellBusy: "터미널에서 다른 명령이 실행 중입니다.",
    opening: "여는 중…", openFailed: "지난 열기 실패",
    message: "직접 입력", messageHelp: "업무 보드에 남기지 않고 지금 세션의 CLI에 바로 넣습니다. Claude가 작업 중이면 바로 넣어 다음 도구 호출 뒤에 반영되고, 답을 기다리거나 다른 CLI면 대기열에 쌓았다가 턴이 끝날 때마다 하나씩 보냅니다. 터미널이 셸 프롬프트에 있으면 업무 보내기처럼 위에서 고른 CLI를 띄워 첫 프롬프트로 넣습니다.", messagePh: "에이전트에게 보낼 말",
    launchSend: (cli) => `${cli} 띄워 보내기`, launchedSend: "CLI를 띄워 보냈습니다", launchSending: "CLI를 띄우는 중…",
    queueBtn: "대기열에 넣기", queued: "대기열에 넣었습니다. 턴이 끝나면 보냅니다.", queueTitle: (n) => `보낼 대기열 ${n}건 · 턴이 끝날 때마다 하나씩 보냅니다`, queueCancel: "빼기", queueRemoved: "대기열에서 뺐습니다",
    taskHelp: "업무 보드에 남김",
    split: "스플릿뷰", splitHint: "진행 중인 캐릭터들의 대화 로그를 한 화면에 나란히 봅니다",
    fold: "▾ 접기", foldHint: "미리보기를 접습니다(세션이 바뀌면 다시 펼침)", pvNoLog: "읽을 대화 로그가 없습니다", pvImage: "🖼 이미지",
    paneDone: "완료", paneDoneHint: "이 칸을 스플릿뷰에서 내립니다. 다시 작업을 시작하면 돌아옵니다.",
    noActive: "진행 중인 캐릭터가 없습니다. 작업을 시작하거나 답을 기다리는 캐릭터가 여기 칸으로 나옵니다.",
    others: (n) => `다른 캐릭터 ${n}`, close: "닫기", sideHide: "◀ 접기",
    cost: (tokens, cost, turns) => `이 세션 추정 사용량 · 토큰 ${tokens} · ${cost} · ${turns}턴`, costHint: "공개 API 요율로 환산한 추정치입니다. 구독제 실제 청구액과 다릅니다. CH(…)는 캐시 포함 전체 토큰.",
    live: { thinking: (t) => `생각 중… ${t}`, tool: (tool, t) => `${tool} 실행 중… ${t}`, stalled: (t) => `작업 중이지만 ${t}째 화면 출력이 없습니다 — 멈췄을 수 있습니다`, human: "답을 기다리는 중 — 멈춘 게 아닙니다", idle: "턴이 끝났습니다 · 입력을 기다림", off: "CLI가 돌고 있지 않습니다", sec: (s) => `${s}초`, minSec: (m, s) => `${m}분 ${s}초` },
    stalled: "응답 없음",
    askPermission: (tool) => `권한 요청 · ${tool}`, askQuestion: "질문에 답해 주세요", askPlan: "계획 승인 요청",
    askNote: "터미널에도 같은 대화상자가 떠 있습니다. 먼저 답한 쪽이 반영되고, 터미널에서 답하면 이 카드는 잠시 뒤 사라집니다.",
    allow: "허용", allowAlways: "허용하고 다시 묻지 않기", deny: "거부", dismiss: "웹에서 닫기", denyPh: "거부 사유(선택, 에이전트에게 전함)",
    askOther: "직접 입력", askSend: "답 보내기", askPick: "모든 질문에 답을 고르거나 입력하세요", askContent: "내용 보기",
    askGone: "이미 답했거나 사라진 요청입니다", askCount: (n) => `답을 기다리는 요청 ${n}건`, imageGone: "🖼 이미지를 더 볼 수 없습니다(오래되어 비웠음)",
    copyMessage: "메시지 복사", copyText: "텍스트 복사", copyImage: "이미지 복사", copied: "클립보드에 복사했습니다",
    copyFailed: "복사하지 못했습니다. 텍스트를 선택해 복사해 주세요.",
    imageCopyHelp: "이미지를 길게 누르거나 마우스 오른쪽 버튼을 눌러 복사해 주세요.",
    viewImage: "이미지 확대", download: "다운로드", downloadImage: "이미지 다운로드", downloadFailed: "파일을 다운로드할 수 없습니다",
    cliRunning: (n) => `${n} 실행 중`, cliBusy: "터미널에서 명령이 실행 중입니다", restartNeeded: "이 터미널은 재시작해야 업무를 보낼 수 있습니다",
    restartWarn: (what) => `${what}. 보내면 터미널을 재시작하므로 지금 세션은 끊깁니다.`,
    talkState: { pending: "준비 중", ready: "준비됨", running: "진행 중", waiting_user: "답을 기다림", wrapping_up: "정리 중", review: "검토", finished: "끝남", paused: "멈춤", "waiting-usage": "사용량 대기" },
    start: "시작", answer: "답하기", answerPh: "사회자에게 보낼 답", noTalks: "토의가 없습니다.", question: "사회자 질문",
    win: { session: "5시간", weekly: "주간", session_model: "5시간", weekly_model: "주간", monthly: "월간", unknown: "기타" },
    winShort: { session: "5h", weekly: "주", monthly: "월" },
    usageNone: "사용량 정보가 없습니다.", resets: "초기화", login: "접속 토큰이 없습니다", loginHelp: "앱 설정 › 제어 › 웹 상태판에서 접속 주소를 복사해 이 기기에서 여세요.",
    unauthorized: "토큰이 맞지 않습니다. 앱에서 접속 주소를 다시 받아 오세요.", offline: "앱에 연결할 수 없습니다",
    err: { "office-agent-busy": "일반 에이전트를 여는 중입니다", "office-agent-running": "일반 에이전트가 이미 돌고 있습니다", "office-agent-working": "작업 중이라 지금은 이어 보낼 수 없습니다", "office-agent-needs-human": "답을 기다리는 중입니다(앱·CLI 리모트에서 답하세요)", "office-agent-shell-busy": "터미널에서 다른 명령이 실행 중입니다", "office-agent-invalid-cwd": "고를 수 없는 작업 폴더입니다", "too-large": "글이 너무 깁니다", "control-char": "보낼 수 없는 문자가 있습니다", "work-send-human-input": "앱 터미널에 쓰다 만 입력이 있습니다", "work-send-cli-returned": "CLI가 끝났습니다", "work-send-launch-timeout": "CLI가 제때 뜨지 않았습니다", "work-send-shell-unsupported": "지원하지 않는 셸입니다",
      "ask-gone": "이미 답했거나 사라진 요청입니다", "ask-unanswered": "모든 질문에 답하세요", "ask-invalid": "답을 보낼 수 없는 질문입니다", "ask-no-rule": "다시 묻지 않기를 고를 수 없는 요청입니다", "bad-request": "잘못된 요청입니다",
      "not-running": "터미널이 꺼져 있습니다", "not-at-shell-prompt": "셸 프롬프트가 아닙니다(CLI가 이미 실행 중이거나 입력 중)", "human-input-pending": "앱에 쓰다 만 입력이 있습니다", "renderer-unavailable": "앱 화면이 응답하지 않습니다", "busy": "다른 입력이 진행 중입니다", "shell-unsupported": "지원하지 않는 셸입니다", "empty": "내용이 비었습니다",
      "queue-full": "대기열이 가득 찼습니다(최대 20건)", "queue-gone": "이미 보냈거나 사라진 메시지입니다", "queue-dropped-session": "세션이 바뀌어 대기열을 비웠습니다", "chat-view-no-cli": "이 터미널에서 Claude·Codex가 입력을 기다리는지 알 수 없습니다", "work-send-session-changed": "세션이 바뀌었습니다",
      "restart-required": "터미널에서 CLI가 실행 중입니다. 재시작 경고를 확인하고 다시 보내세요", "work-send-busy": "이 캐릭터에 보내는 중인 업무가 있습니다", "work-send-clocked-out": "퇴근한 캐릭터입니다", "work-send-auto-run": "자동 진행 중인 캐릭터입니다", "work-send-profile-unsupported": "시작 명령이 있는 캐릭터에는 보낼 수 없습니다", "work-send-agent-missing": "담당 캐릭터를 먼저 고르세요", "work-send-blocked": "선행 작업이 모두 완료된 뒤에 보낼 수 있습니다", "work-send-plan-root": "플랜 루트는 메모 전용입니다. 아래 자동 진행으로 실행하세요", "work-send-schedule-template": "반복 예약 템플릿은 회차마다 복사해 실행합니다", "work-not-loaded": "앱이 아직 업무를 불러오지 않았습니다", "task-not-found": "업무를 찾을 수 없습니다", "unknown-agent": "알 수 없는 캐릭터입니다", "unknown-cli": "지원하지 않는 CLI입니다",
      "auto-run-root-done": "완료된 업무는 자동 진행할 수 없습니다", "auto-run-no-assignee": "루트 업무의 담당 캐릭터를 먼저 지정하세요", "auto-run-no-plan": "진행할 남은 태스크가 없습니다", "auto-run-root-busy": "루트만 남았습니다. 다시 진행하려면 상태를 할 일로 바꾸세요", "auto-run-schedule-template": "반복 예약 템플릿은 직접 자동 진행할 수 없습니다", "auto-run-runner-busy": "담당 캐릭터가 다른 자동 진행을 실행 중입니다", "auto-run-observer-off": "설정에서 에이전트 관찰을 켜야 자동 진행할 수 있습니다", "auto-run-foreign-task": "다른 캐릭터에게 배정된 단계가 있어 시작할 수 없습니다", "auto-run-already-running": "이미 자동 진행 중입니다", "auto-run-no-run": "진행 중인 자동 실행 기록이 없습니다", "auto-run-not-paused": "일시정지 상태가 아닙니다", "auto-run-judge-output": "판단자 응답을 읽지 못했습니다", "schedule-invalid": "예약 규칙이 올바르지 않습니다" },
  },
  en: {
    tabs: { chars: "Characters", tasks: "Tasks", talks: "Discussions", usage: "Usage" },
    theme: { auto: "◐ Auto", light: "☀ Light", dark: "☾ Dark", hint: (now) => `Theme: ${now} · tap to change (Auto follows the device)` },
    phase: { working: "Working", "needs-human": "Waiting for answer", idle: "Waiting for input", ended: "Ended", starting: "Starting", off: "Off", running: "On" },
    clockedOut: "Clocked out", back: "← List", log: "Conversation", noLog: "No conversation log for this terminal (no Claude/Codex/Antigravity transcript).", noItems: "No messages yet.",
    restart: "Restart terminal", resume: "Resume", terminate: "Terminate", interrupt: "Interrupt (Esc)", ctrlc: "Ctrl-C", launch: "Run CLI", launchCli: "CLI to run",
    again: "Tap again to confirm", done: "Sent", saved: "Saved", deleted: "Deleted", result: "Result", lines: "lines",
    sentTasks: "Sent tasks", taskState: { in_progress: "In progress", review: "Finished · to review" }, markDone: "Done", markedDone: "Marked done",
    workNow: "Working on task", workStep: (n, total) => `Step ${n}/${total}`, workDone: (d, total) => `${d}/${total} done`, workAuto: { running: "Auto-run", paused: "Auto-run paused" }, workOpen: "Click to open this task in the Tasks tab",
    sendTask: "Send a task", titlePh: "Task (title)", bodyPh: "Details (optional)", cli: "CLI", send: "Send", restartSend: "Restart and send",
    office: "General agent", officeSub: "No character", officeHelp: "An agent without a character that still gets all Agent Office instructions. Ask it to create tasks or discussions, or to operate the app.",
    instruct: "Instruct", instructPh: "What should the agent do?", folder: "Folder", home: "Home",
    followUp: "Send follow-up", newSession: "Send in a new session", newWarn: "Sending in a new session ends the current one and opens a new one in the chosen folder.",
    officeWorking: "Working. What you send now is queued and follows once the turn ends.", officeHuman: "Waiting for an answer. Reply above or in the app; what you send now is queued.", officeShellBusy: "Another command is running in the terminal.",
    opening: "Opening…", openFailed: "Last open failed",
    message: "Message", messageHelp: "Goes straight to the CLI in the current session, without a task on the board. While Claude works it goes in right away and is picked up after the next tool call; while waiting for an answer, or with other CLIs, messages are queued and sent one per finished turn. At a shell prompt it starts the CLI picked above and sends this as the first prompt, like sending a task.", messagePh: "Message to the agent",
    launchSend: (cli) => `Start ${cli} and send`, launchedSend: "Started the CLI and sent it", launchSending: "Starting the CLI…",
    queueBtn: "Queue", queued: "Queued. It will be sent when the turn ends.", queueTitle: (n) => `${n} queued · one is sent each time a turn ends`, queueCancel: "Remove", queueRemoved: "Removed from the queue",
    taskHelp: "kept on the task board",
    split: "Split view", splitHint: "Watch the conversation logs of all active characters side by side",
    fold: "▾ Collapse", foldHint: "Collapse the preview (it opens again when the session changes)", pvNoLog: "No conversation log", pvImage: "🖼 Image",
    paneDone: "Done", paneDoneHint: "Take this pane off the split view. It comes back when the character starts working again.",
    noActive: "No character is active. Characters that start working or wait for an answer show up here.",
    others: (n) => `Other characters ${n}`, close: "Close", sideHide: "◀ Hide",
    cost: (tokens, cost, turns) => `Estimated session usage · ${tokens} tokens · ${cost} · ${turns} ${turns === 1 ? "turn" : "turns"}`, costHint: "Estimated at public API rates; not your subscription bill. CH(…) is the total including cache.",
    live: { thinking: (t) => `Thinking… ${t}`, tool: (tool, t) => `Running ${tool}… ${t}`, stalled: (t) => `Working, but no terminal output for ${t} — it may be stuck`, human: "Waiting for an answer — not stuck", idle: "Turn finished · waiting for input", off: "No CLI is running", sec: (s) => `${s}s`, minSec: (m, s) => `${m}m ${s}s` },
    stalled: "No response",
    askPermission: (tool) => `Permission request · ${tool}`, askQuestion: "Please answer", askPlan: "Plan approval",
    askNote: "The same dialog is open in the terminal. Whichever answers first wins; if you answer in the terminal, this card goes away shortly.",
    allow: "Allow", allowAlways: "Allow and don't ask again", deny: "Deny", dismiss: "Close here", denyPh: "Reason (optional, sent to the agent)",
    askOther: "Type an answer", askSend: "Send answer", askPick: "Pick or type an answer for every question", askContent: "Show content",
    askGone: "Already answered or gone", askCount: (n) => `${n} request(s) waiting`, imageGone: "🖼 Image no longer available (cleared)",
    copyMessage: "Copy message", copyText: "Copy text", copyImage: "Copy image", copied: "Copied to clipboard",
    copyFailed: "Could not copy. Select the text and copy it manually.",
    imageCopyHelp: "Touch and hold the image or right-click it to copy.",
    viewImage: "Enlarge image", download: "Download", downloadImage: "Download image", downloadFailed: "Could not download this file",
    cliRunning: (n) => `${n} running`, cliBusy: "A command is running in the terminal", restartNeeded: "This terminal must restart to receive the task",
    restartWarn: (what) => `${what}. Sending restarts the terminal, so the current session ends.`,
    talkState: { pending: "Pending", ready: "Ready", running: "Running", waiting_user: "Waiting for you", wrapping_up: "Wrapping up", review: "Review", finished: "Finished", paused: "Paused", "waiting-usage": "Waiting for usage" },
    start: "Start", answer: "Answer", answerPh: "Your answer to the moderator", noTalks: "No discussions.", question: "Moderator question",
    win: { session: "5 hours", weekly: "Weekly", session_model: "5 hours", weekly_model: "Weekly", monthly: "Monthly", unknown: "Other" },
    winShort: { session: "5h", weekly: "wk", monthly: "mo" },
    usageNone: "No usage data.", resets: "resets", login: "No access token", loginHelp: "Copy the address from the app: Settings › Control › Web dashboard, and open it on this device.",
    unauthorized: "Token rejected. Get a fresh address from the app.", offline: "Cannot reach the app",
    err: { "office-agent-busy": "The general agent is opening", "office-agent-running": "The general agent is already running", "office-agent-working": "It is working; follow up after the turn ends", "office-agent-needs-human": "Waiting for an answer (reply in the app or CLI remote)", "office-agent-shell-busy": "Another command is running in the terminal", "office-agent-invalid-cwd": "That folder can't be chosen", "too-large": "Text too long", "control-char": "Text contains characters that can't be sent", "work-send-human-input": "Unsent input is pending in the app terminal", "work-send-cli-returned": "The CLI exited", "work-send-launch-timeout": "The CLI didn't start in time", "work-send-shell-unsupported": "Unsupported shell",
      "ask-gone": "Already answered or gone", "ask-unanswered": "Answer every question", "ask-invalid": "This question can't take that answer", "ask-no-rule": "This request has no 'don't ask again' rule", "bad-request": "Bad request",
      "not-running": "Terminal is not running", "not-at-shell-prompt": "Not at a shell prompt (CLI running or input pending)", "human-input-pending": "Unsent input is pending in the app", "renderer-unavailable": "App window is not responding", "busy": "Another input is in progress", "shell-unsupported": "Unsupported shell", "empty": "Empty",
      "queue-full": "The queue is full (20 max)", "queue-gone": "Already sent or gone", "queue-dropped-session": "The session changed, so the queue was cleared", "chat-view-no-cli": "Can't tell whether Claude/Codex is waiting for input in this terminal", "work-send-session-changed": "The session changed",
      "restart-required": "A CLI is running in the terminal. Confirm the restart warning and send again", "work-send-busy": "A task is already being sent to this character", "work-send-clocked-out": "This character has clocked out", "work-send-auto-run": "This character is running an auto-run", "work-send-profile-unsupported": "Characters with a startup command can't receive tasks", "work-send-agent-missing": "Assign a character first", "work-send-blocked": "Complete prerequisites before sending", "work-send-plan-root": "Plan roots are notes only. Use Auto-run below", "work-send-schedule-template": "Recurring templates clone a new card on each run", "work-not-loaded": "The app hasn't loaded tasks yet", "task-not-found": "Task not found", "unknown-agent": "Unknown character", "unknown-cli": "Unsupported CLI",
      "auto-run-root-done": "Completed tasks can't start auto-run", "auto-run-no-assignee": "Assign a character to the root task first", "auto-run-no-plan": "No remaining tasks to run", "auto-run-root-busy": "Only the root remains. Move it back to To do to run again", "auto-run-schedule-template": "Recurring templates can't be auto-run directly", "auto-run-runner-busy": "This character is already running another plan", "auto-run-observer-off": "Enable agent observer in settings to use auto-run", "auto-run-foreign-task": "Some steps are assigned to another character", "auto-run-already-running": "Auto-run is already active", "auto-run-no-run": "No auto-run record found", "auto-run-not-paused": "Auto-run is not paused", "auto-run-judge-output": "Couldn't read judge output", "schedule-invalid": "Invalid schedule rule" },
  },
}[L];
document.documentElement.lang = L;

// ── 토큰 ──
const TOKEN_KEY = "agent-office-web-dash-token";
function store(k, v) { try { v == null ? localStorage.removeItem(k) : localStorage.setItem(k, v); } catch (_) {} }
function load(k) { try { return localStorage.getItem(k); } catch (_) { return null; } }
/** 이 탭 동안만 남는 값(새로고침해도 유지, 탭을 닫으면 사라짐). JSON으로 넣고 뺀다. */
function sstore(k, v) { try { v == null ? sessionStorage.removeItem(k) : sessionStorage.setItem(k, JSON.stringify(v)); } catch (_) {} }
function sload(k) { try { return JSON.parse(sessionStorage.getItem(k)); } catch (_) { return null; } }

// ── 화면 테마 ──
const THEME_KEY = "agent-office-web-dash-theme";
const THEMES = ["auto", "light", "dark"];
function themeOf() { const t = document.documentElement.dataset.theme; return t === "light" || t === "dark" ? t : "auto"; }
function paintThemeButton() {
  const b = document.getElementById("theme");
  const now = themeOf();
  b.textContent = S.theme[now];
  b.title = S.theme.hint(S.theme[now].replace(/^\S+\s/, ""));
  b.setAttribute("aria-label", b.title);
}
document.getElementById("theme").onclick = () => {
  const next = THEMES[(THEMES.indexOf(themeOf()) + 1) % THEMES.length];
  if (next === "auto") delete document.documentElement.dataset.theme;
  else document.documentElement.dataset.theme = next;
  store(THEME_KEY, next === "auto" ? null : next);
  paintThemeButton();
};
paintThemeButton();
let token = null;
{
  const m = location.hash.match(/(?:^#|&)t=([0-9a-f]+)/);
  if (m) { token = m[1]; store(TOKEN_KEY, token); history.replaceState(null, "", location.pathname); }
  else token = load(TOKEN_KEY);
}

// ── 도우미 ──
function el(tag, attrs, ...kids) {
  const n = document.createElement(tag);
  if (attrs) for (const [k, v] of Object.entries(attrs)) {
    if (v == null || v === false) continue;
    if (k === "class") n.className = v;
    else if (k.startsWith("on")) n.addEventListener(k.slice(2), v);
    else if (k === "text") n.textContent = v;
    else n.setAttribute(k, v === true ? "" : v);
  }
  for (const kid of kids.flat()) if (kid != null && kid !== false) n.append(kid instanceof Node ? kid : document.createTextNode(String(kid)));
  return n;
}
function errText(e) { return S.err[e] || e; }
let toastTimer;
function toast(text) {
  const t = document.getElementById("toast");
  t.textContent = text; t.classList.add("show");
  clearTimeout(toastTimer); toastTimer = setTimeout(() => t.classList.remove("show"), 2600);
}
async function copyText(text) {
  try {
    if (navigator.clipboard?.writeText) {
      await navigator.clipboard.writeText(text);
      return toast(S.copied);
    }
  } catch (_) { /* HTTP 또는 권한 제한에서는 선택 영역을 복사한다. */ }
  const focused = document.activeElement;
  const field = el("textarea", { readonly: true, style: "position:fixed;top:0;left:0;opacity:0;pointer-events:none" });
  field.value = text;
  document.body.append(field);
  let copied = false;
  try {
    field.focus();
    field.select();
    copied = document.execCommand("copy");
  } catch (_) {
  } finally {
    field.remove();
    focused?.focus({ preventScroll: true });
  }
  toast(copied ? S.copied : S.copyFailed);
}
function copyButton(text, label = S.copyText) {
  return el("button", { class: "b", type: "button", text: label, onclick: () => copyText(text) });
}
function textBlock(text) {
  return el("div", { class: "text-block" }, el("pre", { text }),
    text && el("div", { class: "content-actions" }, copyButton(text)));
}
function downloadButton(label, url, name, failed = S.downloadFailed) {
  return el("button", { class: "b", type: "button", text: label, onclick: async (e) => {
    const button = e.currentTarget;
    button.disabled = true;
    try {
      const res = await fetch(url, { headers: { Authorization: "Bearer " + token }, cache: "no-store" });
      if (res.status === 401) { toast(S.unauthorized); return; }
      if (!res.ok) { toast(res.status === 404 ? failed : S.downloadFailed); return; }
      const blobUrl = URL.createObjectURL(await res.blob());
      const link = el("a", { href: blobUrl, download: name, hidden: true });
      document.body.append(link);
      link.click();
      link.remove();
      setTimeout(() => URL.revokeObjectURL(blobUrl), 30000);
    } catch (_) {
      toast(S.downloadFailed);
    } finally {
      button.disabled = false;
    }
  } });
}
function setDot(cls) { document.getElementById("dot").className = "dot " + cls; }
async function api(path, opts = {}) {
  const res = await fetch("/api" + path, {
    method: opts.method || "GET",
    headers: { "Authorization": "Bearer " + token, ...(opts.body !== undefined ? { "Content-Type": "application/json" } : {}) },
    body: opts.body !== undefined ? JSON.stringify(opts.body) : undefined,
    cache: "no-store",
  }).catch(() => null);
  if (!res) { setDot("off"); throw new Error(S.offline); }
  if (res.status === 401) { setDot("off"); unauthorized = true; render(); throw new Error(S.unauthorized); }
  setDot("on");
  const j = await res.json();
  if (!j.ok) { const err = new Error(errText(j.error)); err.code = j.error; throw err; }
  return j.data;
}
async function act(path, body, okMsg) {
  try { await api(path, { method: "POST", body: body || {} }); toast(okMsg || S.done); setTimeout(refreshNow, 400); return true; }
  catch (e) { toast(e.message); return false; }
}
/** 위험한 버튼: 처음 누르면 확인 상태가 되고 3초 안에 다시 눌러야 실행한다. */
function confirmBtn(label, run, cls = "b danger") {
  const b = el("button", { class: cls, type: "button", text: label });
  let armed = null;
  b.addEventListener("click", (e) => {
    e.stopPropagation();
    if (armed) { clearTimeout(armed); armed = null; b.className = cls; b.textContent = label; run(); return; }
    b.className = "b confirm"; b.textContent = S.again;
    armed = setTimeout(() => { armed = null; b.className = cls; b.textContent = label; }, 3000);
  });
  return b;
}
function fmtTime(ms) {
  if (!ms) return "";
  const d = new Date(ms);
  return d.toLocaleString(L === "ko" ? "ko-KR" : undefined, { month: "numeric", day: "numeric", hour: "2-digit", minute: "2-digit" });
}
function setHeaderVar() {
  const hdr = document.querySelector("header");
  if (hdr) document.documentElement.style.setProperty("--hdr", hdr.offsetHeight + "px");
}

// ── 상태 ──
const view = { tab: S.tabs[load("agent-office-web-dash-tab")] ? load("agent-office-web-dash-tab") : "chars", agent: null, task: null, talk: null };
let unauthorized = false;
let chars = null, charsRev = null;
let talksData = null, talkDetail = null, usageData = null;
let tasksData = null, tasksRev = null;
let officeData = null, folders = [], homeDir = "";
const OFFICE = "office-agent";
const logs = new Map();
let outputAt = {}, serverNow = 0, serverNowAt = 0, stalledKey = "";
const STALL_MS = 20000;
function nowMs() { return serverNow ? serverNow + (Date.now() - serverNowAt) : Date.now(); }

function phaseOf(c) {
  if (!c) return ["off", S.phase.off];
  if (c.clockedOut) return ["off", S.clockedOut];
  if (!c.session) return ["off", S.phase.off];
  if (c.session.state === "starting") return ["idle", S.phase.starting];
  if (c.session.state !== "running") return ["off", S.phase.ended];
  if ((c.asks && c.asks.length) || (c.waiting && c.waiting.some((w) => w.source === "hook"))) return ["human", S.phase["needs-human"]];
  const p = c.turn && c.turn.phase;
  if (p === "working") return ["working", S.phase.working];
  if (p === "needs-human") return ["human", S.phase["needs-human"]];
  if (p === "idle") return ["idle", S.phase.idle];
  return ["idle", S.phase.running];
}
function askTitle(a) {
  if (a.toolName === "AskUserQuestion") { const q = a.toolInput && a.toolInput.questions && a.toolInput.questions[0]; return q && q.question ? "❓ " + q.question : S.askQuestion; }
  if (a.toolName === "ExitPlanMode") return S.askPlan;
  return S.askPermission(a.toolName);
}
function waitingMessage(c) {
  if (!c) return null;
  const asks = c.asks || [];
  if (asks.length) return asks.length > 1 ? `${askTitle(asks[0])} · ${S.askCount(asks.length)}` : askTitle(asks[0]);
  const hook = (c.waiting || []).filter((w) => w.source === "hook").pop();
  if (hook && hook.message) return hook.message;
  if (c.turn && c.turn.phase === "needs-human" && c.turn.message) return c.turn.message;
  return null;
}
function avatar(c) {
  if (c.office) return el("div", { class: "avatar office" }, "🤖");
  const box = el("div", { class: "avatar" }, (c.name || "?").slice(0, 1));
  if (c.portrait) {
    const img = el("img", { alt: "", src: `/api/portrait/${encodeURIComponent(c.agentId)}?t=${token}&v=${c.portrait}` });
    img.onerror = () => img.remove();
    img.onload = () => { box.textContent = ""; box.append(img); };
  }
  return box;
}
function goalOf(c) { const l = c.label || {}; return l.goal || null; }
function cwdOf(c) { return (c.label && c.label.cwd) || c.cwd || ""; }
function leaf(path) {
  const parts = (path || "").split(/[\\/]+/).filter(Boolean);
  return parts.length ? parts[parts.length - 1] : path || "";
}
function placeOf(c) {
  const branch = c.label && c.label.branch;
  return [leaf(cwdOf(c)), branch && "⎇ " + branch].filter(Boolean).join(" ");
}
const USAGE_KEYS = ["claude", "codex", "antigravity", "opencode"];
const USAGE_NAMES = { claude: "Claude", codex: "Codex", antigravity: "Antigravity", opencode: "Opencode" };
const CLI_NAMES = { claude: "Claude", codex: "Codex", pi: "Pi", kilo: "Kilo", opencode: "OpenCode", agy: "Antigravity" };
const LAUNCH_CLIS = ["claude", "codex", "pi", "kilo", "opencode", "agy"];
const launchPick = new Map();
function busyShell(c) { return !!(c && c.session && c.session.state === "running" && (c.cli || c.atPrompt === false)); }
function chips(c) {
  const [cls, label] = phaseOf(c);
  const live = liveOf(c);
  return el("div", { class: "chips" },
    el("span", { class: "chip " + cls, text: label }),
    live && live.kind === "stalled" && el("span", { class: "chip stalled", text: S.stalled }),
    c.cli && c.session && c.session.state === "running" && el("span", { class: "chip cli working", text: S.cliRunning(CLI_NAMES[c.cli] || c.cli) }));
}

// 지금 진행 중인 Agent Office 업무와 플랜 진행도(서버 `work`). 누르면 업무 탭의 그 업무로 간다.
const WORK_SEGMENTS = 12;
function workMeter(p) {
  const cur = Math.min(p.done, p.total - 1);
  if (p.total > WORK_SEGMENTS) {
    return el("div", { class: "work-m bar", "aria-hidden": "true" }, el("i", { style: `width:${Math.round((p.done / p.total) * 100)}%` }));
  }
  return el("div", { class: "work-m", "aria-hidden": "true" },
    ...Array.from({ length: p.total }, (_, i) => el("i", { class: i < p.done ? "on" : i === cur ? "cur" : "" })));
}
function workBox(c, compact) {
  const w = c && !c.office && c.work;
  if (!w) return null;
  const p = w.plan && w.plan.total > 0 ? w.plan : null;
  const auto = p && p.autoRun && S.workAuto[p.autoRun];
  return el("div", { class: "work" + (compact ? " compact" : ""), title: S.workOpen, role: "link", onclick: (e) => { e.stopPropagation(); openTask(w.id); } },
    el("div", { class: "work-h" },
      el("span", { class: "work-tag", text: "📋 " + S.workNow }),
      el("span", { class: "work-t", title: w.title, text: w.title }),
      auto && el("span", { class: "work-auto " + p.autoRun, text: auto })),
    p && el("div", { class: "work-p" },
      !compact && el("span", { class: "work-pt", title: p.title, text: `${p.title} · ${S.workStep(p.step || p.done + 1, p.total)}` }),
      workMeter(p),
      el("b", { text: S.workDone(p.done, p.total) })));
}

// ── 화면 ──
function renderNav() {
  const nav = document.getElementById("nav");
  nav.replaceChildren(...Object.entries(S.tabs).map(([k, label]) =>
    el("button", { class: view.tab === k ? "active" : "", text: label, onclick: () => {
      view.tab = k; view.agent = null; view.talk = null;
      if (k !== "tasks") view.task = null;
      store("agent-office-web-dash-tab", k);
      render(); refreshNow();
    } })));
}
function render() {
  const main = document.getElementById("main");
  if (!token || unauthorized) {
    document.getElementById("nav").replaceChildren();
    main.classList.remove("wide", "tasks-wide");
    main.replaceChildren(el("div", { class: "login" }, el("h2", { text: token ? S.unauthorized : S.login }), el("p", { text: S.loginHelp })));
    renderUsageBar();
    return;
  }
  renderNav();
  setHeaderVar();
  main.classList.toggle("wide", view.tab === "chars" && !view.agent && splitOn);
  main.classList.toggle("tasks-wide", view.tab === "tasks");
  if (view.tab === "chars") {
    main.replaceChildren(...(view.agent ? renderAgent() : splitOn ? renderSplit() : renderChars()));
    fitPanes();
  }
  else if (view.tab === "tasks" && window.Dash && window.Dash.tasks) window.Dash.tasks.render(main);
  else if (view.tab === "talks") main.replaceChildren(...renderTalks());
  else main.replaceChildren(...renderUsage());
}
function officeChar() {
  const o = officeData || {};
  return { agentId: OFFICE, office: true, name: S.office, role: S.officeSub, session: o.session, turn: o.turn, waiting: [], asks: o.asks || [], queue: o.queue, cli: o.cli, atPrompt: o.atPrompt, cwd: o.cwd, opening: o.opening, error: o.error, label: o.label };
}
function charOf(agentId) {
  if (agentId === OFFICE) return officeData ? officeChar() : null;
  return (chars || []).find((x) => x.agentId === agentId);
}
function currentChar() { return charOf(view.agent); }
function allChars() { return [...(officeData ? [officeChar()] : []), ...(chars || [])]; }
function openAgent(agentId) {
  drawerOpen = false;
  view.tab = "chars";
  view.agent = agentId;
  view.talk = null;
  store("agent-office-web-dash-tab", "chars");
  render(); refreshNow(); window.scrollTo(0, 0);
}
function openTask(taskId) {
  drawerOpen = false;
  view.tab = "tasks";
  view.agent = null;
  view.talk = null;
  view.task = taskId || null;
  if (window.Dash && window.Dash.tasks) window.Dash.tasks.onSelect(taskId || null);
  store("agent-office-web-dash-tab", "tasks");
  render(); refreshNow(); window.scrollTo(0, 0);
}
function waitText(c) {
  if (!c.office || (c.asks && c.asks.length)) return waitingMessage(c);
  return c.turn && c.turn.phase === "needs-human" ? (c.turn.message || S.officeHuman) : null;
}
function renderOfficeCard() {
  const c = officeChar();
  const [cls] = phaseOf(c);
  const wait = cls === "human" ? waitText(c) : null;
  return el("div", { class: "card clickable office", onclick: () => openAgent(OFFICE) },
    el("div", { class: "row" }, avatar(c),
      el("div", { class: "grow" }, el("div", { class: "name", text: c.name }), el("div", { class: "sub", title: c.cwd || "", text: [S.officeSub, c.session && c.cwd && leaf(c.cwd)].filter(Boolean).join(" · ") })),
      chips(c)),
    wait && el("div", { class: "wait", text: wait }),
    previewSlot(c));
}
const SORT_RANK = { human: 0, working: 1 };
function sortedChars() {
  const rank = (c) => SORT_RANK[phaseOf(c)[0]] ?? 2;
  const at = (c) => (c.turn && c.turn.at) || 0;
  return [...chars].sort((a, b) => rank(a) - rank(b) || at(b) - at(a));
}
function renderChars() {
  if (!chars) return [el("div", { class: "empty", text: "…" })];
  return [renderListBar(), renderOfficeCard(), ...sortedChars().map((c) => {
    const [cls] = phaseOf(c);
    const wait = cls === "human" ? waitingMessage(c) : null;
    const goal = goalOf(c);
    return el("div", { class: "card clickable", onclick: () => openAgent(c.agentId) },
      el("div", { class: "row" }, avatar(c),
        el("div", { class: "grow" }, el("div", { class: "name", text: c.name }), el("div", { class: "sub", title: cwdOf(c), text: [c.role, placeOf(c)].filter(Boolean).join(" · ") })),
        chips(c)),
      goal && el("div", { class: "goal", text: goal }),
      workBox(c),
      wait && el("div", { class: "wait", text: wait }),
      previewSlot(c));
  })];
}
function renderListBar(extra) {
  return el("div", { class: "listbar", id: "listbar" }, extra,
    el("button", { class: "switch", type: "button", role: "switch", "aria-checked": String(splitOn), title: S.splitHint, onclick: () => setSplit(!splitOn) }, el("i"), S.split));
}

// ── 목록 미리보기 ──
const PV_LINES = 5;
const PV_TAIL = 30;
const FOLD_KEY = "agent-office-web-dash-folded";
const folds = sload(FOLD_KEY) || {};
function sessionKey(c) { return c && c.session ? String(c.session.sessionId || "-") : null; }
function isActive(c) { const p = phaseOf(c)[0]; return p === "working" || p === "human"; }
function isFolded(c) { return folds[c.agentId] != null && folds[c.agentId] === sessionKey(c); }
function setFolded(c, on) {
  if (on) folds[c.agentId] = sessionKey(c); else delete folds[c.agentId];
  sstore(FOLD_KEY, folds);
}
function previewItems(agentId) {
  const log = logs.get(agentId);
  const blocks = [];
  for (const it of log.items.slice(-PV_TAIL)) {
    if ((it.kind === "tool_result" || it.kind === "image") && blocks.length) blocks[blocks.length - 1].push(it);
    else blocks.push([it]);
  }
  const out = [];
  for (let i = blocks.length - 1; i >= 0 && out.length < PV_LINES; i--) for (const it of blocks[i]) if (out.length < PV_LINES) out.push(it);
  return out;
}
function pvLine(it) {
  const side = it.sidechain ? " side" : "";
  if (it.kind === "tool_use") return el("div", { class: "pv-l tool" + side }, el("b", { text: "🔧 " + (it.toolName || "⤷") }), " " + firstLine(it.text, 160));
  if (it.kind === "tool_result") return el("div", { class: "pv-l res" + (it.isError ? " err" : "") + side, text: "↳ " + firstLine(it.text, 160) });
  if (it.kind === "image") return el("div", { class: "pv-l res" + side, text: S.pvImage });
  return el("div", { class: "pv-l " + it.role + side, text: (it.role === "user" ? "👤 " : "💬 ") + firstLine(it.text, 200) });
}
function previewKey(c) {
  if (!isActive(c)) return "";
  if (isFolded(c)) return "f|" + phaseOf(c)[1];
  const log = logs.get(c.agentId);
  if (!log) return "…";
  return [log.epoch, log.attached, log.items.length && previewItems(c.agentId).map((it) => it.seq).join(",")].join("|");
}
function renderPreview(c) {
  if (!isActive(c)) return null;
  const toggle = (e) => { e.stopPropagation(); setFolded(c, !isFolded(c)); refreshPreviews(); refreshNow(); };
  if (isFolded(c)) return el("button", { class: "pv folded", type: "button", "aria-expanded": "false", onclick: toggle }, "▸ " + phaseOf(c)[1]);
  const log = logs.get(c.agentId);
  const lines = !log ? [el("div", { class: "pv-empty", text: "…" })]
    : !log.attached ? [el("div", { class: "pv-empty", text: S.pvNoLog })]
    : !log.items.length ? [el("div", { class: "pv-empty", text: S.noItems })]
    : previewItems(c.agentId).map(pvLine);
  return el("div", { class: "pv" }, el("div", { class: "pv-lines" }, ...lines),
    el("button", { class: "pv-fold", type: "button", "aria-expanded": "true", title: S.foldHint, text: S.fold, onclick: toggle }));
}
function previewSlot(c) {
  const slot = el("div", { class: "pv-slot", "data-agent": c.agentId });
  slot.dataset.key = previewKey(c);
  const pv = renderPreview(c);
  if (pv) slot.append(pv);
  return slot;
}
// 업무 탭: 진행 중 업무의 담당 캐릭터가 작업 중·답 대기면 같은 미리보기를 카드에 붙인다
function taskPreviewIds() {
  const ids = new Set();
  for (const it of (tasksData && tasksData.open) || []) {
    const c = it.status === "in_progress" && it.assigneeAgentId && charOf(it.assigneeAgentId);
    if (c && isActive(c) && !isFolded(c)) ids.add(c.agentId);
  }
  return [...ids];
}
function refreshPreviews() {
  for (const slot of document.querySelectorAll("#main .pv-slot")) {
    const c = charOf(slot.dataset.agent);
    if (!c) continue;
    const key = previewKey(c);
    if (slot.dataset.key === key) continue;
    slot.dataset.key = key;
    const pv = renderPreview(c);
    slot.replaceChildren(...(pv ? [pv] : []));
  }
}

// ── 스플릿뷰 ──
const SPLIT_KEY = "agent-office-web-dash-split", SIDE_KEY = "agent-office-web-dash-side";
const PANES_KEY = "agent-office-web-dash-panes", SUPPRESS_KEY = "agent-office-web-dash-pane-off";
let splitOn = load(SPLIT_KEY) === "1";
const panes = new Map(Object.entries(sload(PANES_KEY) || {}));
const suppressed = new Set(sload(SUPPRESS_KEY) || []);
let drawerOpen = false;
const WIDE = window.matchMedia("(min-width: 900px)");
function savePanes() { sstore(PANES_KEY, Object.fromEntries(panes)); sstore(SUPPRESS_KEY, [...suppressed]); }
function syncPanes() {
  if (!chars) return false;
  let changed = false;
  const all = allChars();
  const byId = new Map(all.map((c) => [c.agentId, c]));
  for (const [id, sid] of panes) {
    if (sessionKey(byId.get(id)) !== sid) { panes.delete(id); changed = true; }
  }
  for (const c of all) {
    if (!isActive(c)) { if (suppressed.delete(c.agentId)) changed = true; continue; }
    if (!suppressed.has(c.agentId) && !panes.has(c.agentId) && sessionKey(c)) { panes.set(c.agentId, sessionKey(c)); changed = true; }
  }
  if (changed) savePanes();
  return changed;
}
function dismissPane(agentId) {
  panes.delete(agentId);
  const c = charOf(agentId);
  if (c && isActive(c)) suppressed.add(agentId);
  savePanes();
  updateSplit(true, new Map());
}
function setSplit(on) {
  splitOn = on; drawerOpen = false;
  store(SPLIT_KEY, on ? "1" : null);
  render(); refreshNow(); window.scrollTo(0, 0);
}
function sideOpen() { return WIDE.matches ? load(SIDE_KEY) !== "0" : drawerOpen; }
function toggleSide() {
  if (WIDE.matches) store(SIDE_KEY, sideOpen() ? "0" : null); else drawerOpen = !drawerOpen;
  const split = document.getElementById("split");
  if (split) split.classList.toggle("side-open", sideOpen());
}
function otherChars() { return allChars().filter((c) => !panes.has(c.agentId)); }
function renderSplitBar() {
  return renderListBar(el("button", { class: "b", type: "button", text: "☰ " + S.others(otherChars().length), onclick: toggleSide }));
}
function renderSide() {
  const others = otherChars();
  return el("aside", { class: "side", id: "side" },
    el("div", { class: "side-h" }, el("span", { text: S.others(others.length) }),
      el("button", { type: "button", text: WIDE.matches ? S.sideHide : S.close, onclick: toggleSide })),
    ...others.map((c) => el("div", { class: "card clickable", onclick: () => openAgent(c.agentId) },
      el("div", { class: "row" }, avatar(c),
        el("div", { class: "grow" }, el("div", { class: "name", text: c.name }), el("div", { class: "sub", text: c.office ? S.officeSub : c.role || "" })),
        chips(c)), workBox(c, true))));
}
function renderPaneHead(c) {
  const [cls] = phaseOf(c);
  const wait = cls === "human" ? waitText(c) : null;
  const sub = c.office ? [S.officeSub, c.cwd && leaf(c.cwd)] : [c.role, placeOf(c)];
  return el("div", { class: "pane-head" },
    el("div", { class: "row" },
      el("div", { class: "row grow clickable", onclick: () => openAgent(c.agentId) }, avatar(c),
        el("div", { class: "grow" }, el("div", { class: "name", text: c.name }), el("div", { class: "sub", title: cwdOf(c), text: sub.filter(Boolean).join(" · ") }))),
      chips(c),
      el("button", { class: "b", type: "button", title: S.paneDoneHint, text: S.paneDone, onclick: () => dismissPane(c.agentId) })),
    !c.office && goalOf(c) && el("div", { class: "goal", title: goalOf(c), text: goalOf(c) }),
    workBox(c, true),
    wait && el("div", { class: "wait", text: wait }));
}
function renderPane(c) {
  const live = el("div", { class: "live", role: "status", "data-agent": c.agentId });
  fillLive(live, c);
  return el("section", { class: "pane " + phaseOf(c)[0], "data-agent": c.agentId },
    renderPaneHead(c), live, el("div", { class: "pane-body" }, renderLog(c.agentId, null)));
}
function renderSplit() {
  if (!chars) return [el("div", { class: "empty", text: "…" })];
  const list = [...panes.keys()].map(charOf).filter(Boolean);
  return [renderSplitBar(),
    el("div", { class: "split" + (sideOpen() ? " side-open" : ""), id: "split" },
      el("div", { class: "backdrop", onclick: toggleSide }), renderSide(),
      el("div", { class: "panes", id: "panes" }, ...(list.length ? list.map(renderPane) : [el("div", { class: "empty", text: S.noActive })])))];
}
function updateSplit(changed, modes) {
  const grid = document.getElementById("panes");
  if (!grid) { render(); return; }
  if (changed) {
    const want = [...panes.keys()].map(charOf).filter(Boolean);
    const have = new Map([...grid.querySelectorAll(":scope > .pane")].map((p) => [p.dataset.agent, p]));
    for (const [id, p] of have) if (!panes.has(id) || !charOf(id)) { p.remove(); have.delete(id); }
    grid.querySelector(":scope > .empty")?.remove();
    let prev = null;
    for (const c of want) {
      let p = have.get(c.agentId);
      if (p) {
        p.className = "pane " + phaseOf(c)[0];
        p.querySelector(".pane-head").replaceWith(renderPaneHead(c));
      } else p = renderPane(c);
      const at = prev ? prev.nextElementSibling : grid.firstElementChild;
      if (at !== p) grid.insertBefore(p, at);
      prev = p;
    }
    if (!want.length) grid.append(el("div", { class: "empty", text: S.noActive }));
    document.getElementById("side")?.replaceWith(renderSide());
    document.getElementById("listbar")?.replaceWith(renderSplitBar());
  }
  for (const p of grid.querySelectorAll(":scope > .pane")) {
    const box = p.querySelector(".pane-body > .log");
    const mode = modes.get(p.dataset.agent);
    if (mode === "reset") box.replaceWith(renderLog(p.dataset.agent, null));
    else if (mode === "append") appendLog(box);
  }
  fitPanes();
}
function fitPanes() {
  setHeaderVar();
  const grid = document.getElementById("panes");
  if (!grid) return;
  const n = grid.querySelectorAll(":scope > .pane").length;
  if (!n) return;
  const cols = Math.max(1, getComputedStyle(grid).gridTemplateColumns.split(" ").length);
  const rows = Math.ceil(n / cols);
  const top = grid.getBoundingClientRect().top + window.scrollY;
  const free = window.innerHeight - Math.min(top, document.querySelector("header").offsetHeight + 60) - 56;
  const h = cols === 1 ? free * 0.8 : (free - 12 * (rows - 1)) / rows;
  grid.style.setProperty("--pane-h", Math.round(Math.max(300, h)) + "px");
}
window.addEventListener("resize", () => { setHeaderVar(); if (document.getElementById("panes")) fitPanes(); });
WIDE.addEventListener("change", () => { drawerOpen = false; if (document.getElementById("split") || view.tab === "tasks") render(); });
function renderAgent() {
  const back = el("button", { class: "back", text: S.back, onclick: () => { view.agent = null; render(); refreshNow(); } });
  const c = currentChar();
  if (!c) return [back];
  if (c.office) return [back, renderHead(c), renderAsks(c), renderOfficeForm(c), renderCost(c), renderLive(c), renderLog(c.agentId)];
  return [back, renderHead(c), renderAsks(c), renderSent(c), renderMsgForm(c), renderTaskForm(c), renderCost(c), renderLive(c), renderLog(c.agentId)];
}

// ── 권한 요청·선택지 ──
const askDrafts = new Map();
function askDraft(id) {
  let d = askDrafts.get(id);
  if (!d) { d = { picks: {}, other: {}, reason: "", busy: false }; askDrafts.set(id, d); }
  return d;
}
function renderAsks(c) {
  const asks = c.asks || [];
  for (const id of [...askDrafts.keys()]) if (!asks.some((a) => a.id === id)) askDrafts.delete(id);
  const box = el("div", { id: "asks", "data-key": JSON.stringify(asks.map((a) => [a.id, !!(askDrafts.get(a.id) || {}).busy])) });
  for (const a of asks) box.append(renderAsk(a));
  return box;
}
function refreshAsks() {
  const c = currentChar();
  const box = document.getElementById("asks");
  if (!c || !box) return;
  const next = renderAsks(c);
  if (next.dataset.key !== box.dataset.key) box.replaceWith(next);
}
function renderAsk(a) {
  const d = askDraft(a.id);
  const question = a.toolName === "AskUserQuestion";
  const title = question ? S.askQuestion : a.toolName === "ExitPlanMode" ? S.askPlan : S.askPermission(a.toolName);
  return el("div", { class: "card ask" },
    el("div", { class: "row" }, el("div", { class: "grow name", text: (a.sidechain ? "⤷ " : "") + title }), el("span", { class: "sub", text: fmtTime(a.at) })),
    question ? renderQuestions(a, d) : renderPermission(a, d),
    el("div", { class: "note", text: S.askNote }));
}
function renderQuestions(a, d) {
  const qs = (a.toolInput && a.toolInput.questions) || [];
  const wrap = el("div");
  for (const q of qs) {
    const key = q.question;
    const multi = !!q.multiSelect;
    const picks = d.picks[key] || (d.picks[key] = []);
    const opts = el("div", { class: "opts" });
    const other = el("input", { value: d.other[key] || "", placeholder: S.askOther, oninput: (e) => {
      d.other[key] = e.target.value;
      if (!multi && e.target.value.trim() && picks.length) { picks.length = 0; draw(); }
    } });
    const draw = () => opts.replaceChildren(...(q.options || []).map((o) => el("button", {
      class: "b opt" + (picks.includes(o.label) ? " on" : ""),
      onclick: () => {
        const i = picks.indexOf(o.label);
        if (multi) { if (i >= 0) picks.splice(i, 1); else picks.push(o.label); }
        else { picks.length = 0; if (i < 0) picks.push(o.label); d.other[key] = ""; other.value = ""; }
        draw();
      },
    }, el("div", { text: o.label }), o.description && el("div", { class: "opt-d", text: o.description }))));
    draw();
    wrap.append(el("div", { class: "q" }, q.header && el("span", { class: "tag", text: q.header }), el("div", { class: "qt", text: q.question }), opts, other));
  }
  const answers = () => {
    const out = {};
    for (const q of qs) {
      const parts = [...(d.picks[q.question] || [])];
      const typed = (d.other[q.question] || "").trim();
      if (typed) parts.push(typed);
      out[q.question] = parts.join(", ");
    }
    return out;
  };
  wrap.append(el("div", { class: "btns" },
    el("button", { class: "b primary", disabled: d.busy, text: S.askSend, onclick: () => {
      const ans = answers();
      if (!qs.length || Object.values(ans).some((v) => !v)) { toast(S.askPick); return; }
      sendAsk(a.id, { action: "allow", answers: ans });
    } }),
    el("button", { class: "b", disabled: d.busy, text: S.dismiss, onclick: () => sendAsk(a.id, { action: "dismiss" }) })));
  return wrap;
}
function permDetail(a) {
  const i = a.toolInput || {};
  const file = i.file_path || i.notebook_path || i.path;
  const fold = (label, text) => el("details", { class: "tool" }, el("summary", { text: label }), textBlock(text));
  const diff = (o, n) => [...String(o || "").split("\n").map((l) => "- " + l), ...String(n || "").split("\n").map((l) => "+ " + l)].join("\n");
  switch (a.toolName) {
    case "Bash": return [textBlock(i.command || ""), i.description && el("div", { class: "sub", style: "white-space:normal", text: i.description })];
    case "Write": return [el("div", { class: "sub", text: file }), fold(S.askContent, i.content || "")];
    case "Edit": return [el("div", { class: "sub", text: file }), textBlock(diff(i.old_string, i.new_string))];
    case "MultiEdit": return [el("div", { class: "sub", text: file }), textBlock((i.edits || []).map((e) => diff(e.old_string, e.new_string)).join("\n⋯\n"))];
    case "WebFetch": return [textBlock(i.url || ""), i.prompt && el("div", { class: "sub", style: "white-space:normal", text: i.prompt })];
    case "ExitPlanMode": return [textBlock(i.plan || "")];
    default: return [file && el("div", { class: "sub", text: file }), textBlock(JSON.stringify(i, null, 2))];
  }
}
function renderPermission(a, d) {
  const reason = el("input", { value: d.reason, placeholder: S.denyPh, style: "margin-top:8px", oninput: (e) => { d.reason = e.target.value; } });
  return el("div", null, ...permDetail(a), reason,
    el("div", { class: "btns" },
      el("button", { class: "b primary", disabled: d.busy, text: S.allow, onclick: () => sendAsk(a.id, { action: "allow" }) }),
      a.canAlways && confirmBtn(S.allowAlways, () => sendAsk(a.id, { action: "always" }), "b"),
      el("button", { class: "b danger", disabled: d.busy, text: S.deny, onclick: () => sendAsk(a.id, { action: "deny", message: d.reason || null }) }),
      el("button", { class: "b", disabled: d.busy, text: S.dismiss, onclick: () => sendAsk(a.id, { action: "dismiss" }) })));
}
function dropAsk(id) {
  askDrafts.delete(id);
  if (view.agent === OFFICE && officeData && officeData.asks) officeData.asks = officeData.asks.filter((x) => x.id !== id);
  for (const c of chars || []) if (c.asks) c.asks = c.asks.filter((x) => x.id !== id);
  charsRev = null;
}
async function sendAsk(id, body) {
  const d = askDraft(id);
  if (d.busy) return;
  d.busy = true; refreshAsks();
  try {
    await api(`/asks/${encodeURIComponent(id)}`, { method: "POST", body });
    toast(S.done);
    dropAsk(id);
  } catch (e) {
    toast(e.message);
    if (e.code === "ask-gone") dropAsk(id);
  } finally {
    const left = askDrafts.get(id);
    if (left) left.busy = false;
    refreshAsks(); setTimeout(refreshNow, 400);
  }
}
function renderOfficeHead(c) {
  const [cls] = phaseOf(c);
  const running = c.session && c.session.state === "running";
  const wait = cls === "human" ? waitText(c) : null;
  return el("div", { class: "card office", id: "head" },
    el("div", { class: "row" }, avatar(c),
      el("div", { class: "grow" }, el("div", { class: "name", text: c.name }), el("div", { class: "sub", title: c.cwd || "", text: running && c.cwd ? leaf(c.cwd) : S.officeSub })),
      chips(c)),
    !c.session && el("div", { class: "note", text: S.officeHelp }),
    wait && el("div", { class: "wait", text: wait }),
    c.error && el("div", { class: "warn", text: `${S.openFailed}: ${errText(c.error)}` }),
    running && el("div", { class: "btns" },
      el("button", { class: "b", text: S.interrupt, onclick: () => act(`/office-agent/interrupt`) }),
      el("button", { class: "b", text: S.ctrlc, onclick: () => act(`/office-agent/ctrl-c`) }),
      confirmBtn(S.terminate, () => act(`/office-agent/terminate`))));
}
function renderLaunch(c, running) {
  const id = encodeURIComponent(c.agentId);
  const def = LAUNCH_CLIS.includes(c.defaultCli) ? c.defaultCli : "claude";
  const pick = launchPick.get(c.agentId) || def;
  const off = !running || busyShell(c);
  const sel = el("select", { class: "launch-cli", "aria-label": S.launchCli, disabled: off, onchange: (e) => { launchPick.set(c.agentId, e.target.value); refreshMsg(); } },
    ...LAUNCH_CLIS.map((v) => el("option", { value: v, text: CLI_NAMES[v], selected: pick === v })));
  return el("span", { class: "launch" }, sel,
    el("button", { class: "b", disabled: off, text: S.launch, onclick: () => act(`/agents/${id}/launch`, { cli: launchPick.get(c.agentId) || def }) }));
}
function renderHead(c) {
  if (c.office) return renderOfficeHead(c);
  const [cls] = phaseOf(c);
  const running = c.session && c.session.state === "running";
  const id = encodeURIComponent(c.agentId);
  const wait = cls === "human" ? waitingMessage(c) : null;
  return el("div", { class: "card", id: "head" },
    el("div", { class: "row" }, avatar(c),
      el("div", { class: "grow" }, el("div", { class: "name", text: c.name }), el("div", { class: "sub", title: cwdOf(c), text: placeOf(c) })),
      chips(c)),
    goalOf(c) && el("div", { class: "goal", text: goalOf(c) }),
    workBox(c),
    wait && el("div", { class: "wait", text: wait }),
    el("div", { class: "btns" },
      el("button", { class: "b", disabled: !running, text: S.interrupt, onclick: () => act(`/agents/${id}/interrupt`) }),
      el("button", { class: "b", disabled: !running, text: S.ctrlc, onclick: () => act(`/agents/${id}/ctrl-c`) }),
      renderLaunch(c, running)),
    el("div", { class: "btns" },
      confirmBtn(S.restart, () => act(`/agents/${id}/restart`), "b"),
      confirmBtn(S.resume, () => act(`/agents/${id}/resume`), "b"),
      running && confirmBtn(S.terminate, () => act(`/agents/${id}/terminate`))));
}

// ── 지금 상태(생각 중·멈춤) ──
function runningTool(agentId) {
  const log = logs.get(agentId);
  if (!log || !log.items) return null;
  for (let i = log.items.length - 1; i >= 0; i--) {
    const it = log.items[i];
    if (it.sidechain || it.kind === "image") continue;
    return it.kind === "tool_use" && it.toolName ? it.toolName : null;
  }
  return null;
}
function liveOf(c) {
  if (!c.session || c.session.state !== "running") return { kind: "off" };
  if (phaseOf(c)[0] === "human") return { kind: "human" };
  const p = c.turn && c.turn.phase;
  if (p === "idle") return { kind: "idle" };
  if (p === "ended") return { kind: "off" };
  if (p !== "working") return null;
  const now = nowMs();
  const elapsed = Math.max(0, now - (c.turn.since || c.turn.at || now));
  const out = outputAt[c.agentId];
  const silent = out ? Math.max(0, now - out) : elapsed;
  if (silent >= STALL_MS) return { kind: "stalled", elapsed, silent };
  return { kind: "thinking", elapsed };
}
function fmtDur(ms) {
  const s = Math.floor(ms / 1000);
  return s >= 60 ? S.live.minSec(Math.floor(s / 60), s % 60) : S.live.sec(s);
}
function renderLive(c) {
  const box = el("div", { id: "live", class: "live", role: "status" });
  fillLive(box, c);
  return box;
}
function fillLive(box, c) {
  const live = c && liveOf(c);
  const log = c && logs.get(c.agentId);
  if (!live || !log || !log.attached) { box.className = "live"; box.dataset.key = ""; box.replaceChildren(); return; }
  const tool = live.kind === "thinking" ? runningTool(c.agentId) : null;
  const text = live.kind === "thinking" ? (tool ? S.live.tool(tool, fmtDur(live.elapsed)) : S.live.thinking(fmtDur(live.elapsed)))
    : live.kind === "stalled" ? S.live.stalled(fmtDur(live.silent)) : S.live[live.kind];
  const key = live.kind + "|" + text;
  if (box.dataset.key === key) return;
  const moving = live.kind === "thinking";
  if (box.dataset.kind !== live.kind || !box.firstChild) {
    box.replaceChildren(moving ? el("span", { class: "live-dots", "aria-hidden": "true" }, el("i"), el("i"), el("i")) : el("span", { class: "live-mark", "aria-hidden": "true" }), el("span"));
  }
  box.className = "live " + live.kind;
  box.dataset.kind = live.kind;
  box.dataset.key = key;
  box.lastChild.textContent = text;
}
function costText(c) {
  const u = c && c.label && c.label.usage;
  if (!u || !c.session || c.session.sessionId !== u.sessionId) return "";
  return S.cost(u.tokens, u.cost, u.turns);
}
function renderCost(c) {
  const box = el("div", { id: "cost", class: "cost", title: S.costHint });
  box.textContent = costText(c);
  return box;
}
function refreshCost() {
  const box = document.getElementById("cost");
  if (!box) return;
  const text = costText(currentChar());
  if (box.textContent !== text) box.textContent = text;
}
function refreshLive() {
  const box = document.getElementById("live");
  if (box) fillLive(box, currentChar());
  for (const live of document.querySelectorAll("#panes .live[data-agent]")) fillLive(live, charOf(live.dataset.agent));
}
setInterval(refreshLive, 1000);

// ── 직접 입력과 보낼 대기열 ──
const msgDrafts = new Map();
function msgDraftOf(agentId) {
  let d = msgDrafts.get(agentId);
  if (!d) { d = { text: "", sending: false }; msgDrafts.set(agentId, d); }
  return d;
}
function queueOf(c) { return (c.queue && c.queue.items) || []; }
function willQueue(c) {
  const p = c.turn && c.turn.phase;
  const steer = !!(c.queue && c.queue.steer);
  return (p === "working" && !steer) || p === "needs-human" || !!(c.asks && c.asks.length) || queueOf(c).length > 0;
}
function renderMsgForm(c) {
  const d = msgDraftOf(c.agentId);
  return el("div", { class: "card msgform", id: "msgform" },
    el("h3", { text: S.message }),
    el("textarea", { id: "msgtext", placeholder: S.messagePh, oninput: (e) => { d.text = e.target.value; } }, d.text),
    renderMsgActions(c),
    renderQueue(c),
    el("div", { class: "note", text: S.messageHelp }));
}
function launchCliFor(c) {
  if (c.office || !c.session || c.session.state !== "running" || c.cli || c.atPrompt !== true) return null;
  const def = LAUNCH_CLIS.includes(c.defaultCli) && c.defaultCli !== "opencode" ? c.defaultCli : "claude";
  const pick = launchPick.get(c.agentId);
  return pick && pick !== "opencode" ? pick : def;
}
function renderMsgActions(c) {
  const d = msgDraftOf(c.agentId);
  const running = !!(c.session && c.session.state === "running");
  const launch = launchCliFor(c);
  const queue = !launch && willQueue(c);
  const text = launch ? (d.sending ? S.launchSending : S.launchSend(CLI_NAMES[launch] || launch)) : queue ? S.queueBtn : S.send;
  return el("div", { id: "msgact", class: "btns", "data-key": JSON.stringify([running, queue, launch, d.sending]) },
    el("button", { class: "b primary", disabled: !running || d.sending, text, onclick: () => sendMessage(c.agentId) }));
}
async function sendMessage(agentId) {
  const d = msgDraftOf(agentId);
  if (d.sending) return;
  if (!d.text.trim()) { toast(errText("empty")); return; }
  const c = currentChar();
  const cli = c && c.agentId === agentId ? launchCliFor(c) : null;
  d.sending = true; refreshMsg();
  try {
    const r = await api(`/agents/${encodeURIComponent(agentId)}/message`, { method: "POST", body: { text: d.text, ...(cli ? { cli } : {}) } });
    toast(r && r.outcome === "queued" ? S.queued : r && r.outcome === "launched" ? S.launchedSend : S.done);
    d.text = "";
    const box = document.getElementById("msgtext");
    if (box) box.value = "";
    charsRev = null; setTimeout(refreshNow, 300);
  } catch (e) { toast(e.message); }
  finally { d.sending = false; refreshMsg(); }
}
function renderQueue(c) {
  const items = queueOf(c);
  const error = c.queue && c.queue.error;
  const box = el("div", { class: "queue", id: "queue", "data-key": JSON.stringify([items.map((m) => m.id), error || ""]) });
  if (items.length) box.append(el("div", { class: "q-title", text: S.queueTitle(items.length) }));
  items.forEach((m, i) => box.append(el("div", { class: "q-item" },
    el("span", { class: "q-n", text: String(i + 1) }),
    el("span", { class: "q-text", title: m.text, text: firstLine(m.text, 200) }),
    el("button", { class: "b", type: "button", text: S.queueCancel, onclick: () => cancelQueued(c, m.id) }))));
  if (error) box.append(el("div", { class: "q-err", text: errText(error) }));
  return box;
}
async function cancelQueued(c, id) {
  const path = c.office ? "/office-agent/queue-cancel" : `/agents/${encodeURIComponent(c.agentId)}/queue-cancel`;
  try { await api(path, { method: "POST", body: { id } }); toast(S.queueRemoved); }
  catch (e) { toast(e.message); }
  charsRev = null; setTimeout(refreshNow, 200);
}
function refreshMsg() {
  const c = currentChar();
  if (!c) return;
  for (const [id, make] of [["queue", renderQueue], ["msgact", renderMsgActions]]) {
    const box = document.getElementById(id);
    if (!box || (id === "msgact" && c.office)) continue;
    const next = make(c);
    if (next.dataset.key !== box.dataset.key) box.replaceWith(next);
  }
}

// ── 캐릭터에게 보낸 업무 (클릭하면 업무 탭 상세로 이동) ──
function sentOf(agentId) {
  const open = (tasksData && tasksData.open) || [];
  return open.filter((t) => t.assigneeAgentId === agentId && !t.plan && (t.status === "in_progress" || t.status === "review"))
    .sort((a, b) => (a.status === "review" ? 0 : 1) - (b.status === "review" ? 0 : 1) || (b.updatedAt || 0) - (a.updatedAt || 0));
}
const doneBusy = new Set();
function renderSent(c) {
  const rows = sentOf(c.agentId);
  const box = el("div", { id: "sent", "data-key": JSON.stringify([rows.map((t) => [t.id, t.status, t.title]), [...doneBusy]]) });
  if (!rows.length) return box;
  box.append(el("div", { class: "card" }, el("h3", { text: S.sentTasks, style: "font-size:14px;margin:0 0 4px" }),
    ...rows.map((t) => el("div", { class: "row sent-row clickable", onclick: () => openTask(t.id) },
      el("div", { class: "grow" }, el("div", { text: t.title || firstLine(t.body, 60) }), el("div", { class: "sub", text: fmtTime(t.updatedAt) })),
      el("span", { class: "chip " + (t.status === "review" ? "idle" : "working"), text: S.taskState[t.status] || t.status }),
      el("button", { class: "b" + (t.status === "review" ? " primary" : ""), disabled: doneBusy.has(t.id), text: S.markDone, onclick: (e) => { e.stopPropagation(); markDone(t.id); } })))));
  return box;
}
async function markDone(id) {
  if (doneBusy.has(id)) return;
  doneBusy.add(id); refreshSent();
  try {
    await api(`/tasks/${encodeURIComponent(id)}/done`, { method: "POST", body: {} });
    toast(S.markedDone);
    if (tasksData) tasksData.open = tasksData.open.filter((t) => t.id !== id);
    tasksRev = null;
  } catch (e) { toast(e.message); }
  finally { doneBusy.delete(id); refreshSent(); setTimeout(refreshNow, 400); }
}
function refreshSent() {
  const c = currentChar();
  const box = document.getElementById("sent");
  if (!c || c.office || !box) return;
  const next = renderSent(c);
  if (next.dataset.key !== box.dataset.key) box.replaceWith(next);
}

// ── 캐릭터별 업무 보내기 ──
const drafts = new Map();
function draftOf(c) {
  let d = drafts.get(c.agentId);
  if (!d) { d = { title: "", body: "", cli: c.cli || c.defaultCli || "claude", needRestart: false, sending: false, open: false }; drafts.set(c.agentId, d); }
  return d;
}
function renderTaskForm(c) {
  const d = draftOf(c);
  const cli = el("select", { onchange: (e) => { d.cli = e.target.value; } },
    ...["claude", "codex", "agy", "kilo", "pi"].map((v) => el("option", { value: v, text: CLI_NAMES[v] || v, selected: d.cli === v })));
  return el("details", { class: "card taskform", id: "taskform", open: d.open, ontoggle: (e) => { d.open = e.currentTarget.open; } },
    el("summary", null, S.sendTask, el("span", { class: "sub", text: S.taskHelp })),
    el("input", { value: d.title, style: "margin-top:8px", placeholder: S.titlePh, oninput: (e) => { d.title = e.target.value; } }),
    el("textarea", { placeholder: S.bodyPh, style: "margin-top:6px", oninput: (e) => { d.body = e.target.value; } }, d.body),
    el("div", { class: "row", style: "margin-top:6px" }, el("label", { class: "sub", text: S.cli }), el("div", { style: "width:130px" }, cli)),
    renderTaskActions(c));
}
function renderTaskActions(c) {
  const d = draftOf(c);
  const restart = d.needRestart || busyShell(c);
  const what = c.cli ? S.cliRunning(CLI_NAMES[c.cli] || c.cli) : busyShell(c) ? S.cliBusy : S.restartNeeded;
  const key = JSON.stringify([restart, what, d.sending]);
  const submit = async () => {
    if (d.sending) return;
    if (!d.title.trim() && !d.body.trim()) { toast(errText("empty")); return; }
    d.sending = true; refreshTaskActions();
    try {
      await api("/tasks", { method: "POST", body: { title: d.title, body: d.body, assigneeAgentId: c.agentId, cli: d.cli, send: true, restart } });
      toast(S.done);
      drafts.delete(c.agentId);
      tasksRev = null;
      const form = document.getElementById("taskform");
      if (form) form.replaceWith(renderTaskForm(c));
      setTimeout(refreshNow, 400);
    } catch (e) {
      if (e.code === "restart-required") d.needRestart = true;
      toast(e.message);
    } finally { d.sending = false; refreshTaskActions(); }
  };
  return el("div", { id: "taskact", "data-key": key },
    restart && el("div", { class: "warn", text: "⚠ " + S.restartWarn(what) }),
    el("div", { class: "btns" }, restart
      ? confirmBtn(S.restartSend, submit, "b danger")
      : el("button", { class: "b primary", disabled: d.sending, text: S.send, onclick: submit })));
}
function refreshTaskActions() {
  const c = currentChar();
  const box = document.getElementById("taskact");
  if (!c || !box) return;
  const next = c.office ? renderOfficeActions(c) : renderTaskActions(c);
  if (next.dataset.key !== box.dataset.key) box.replaceWith(next);
}

// ── 일반 에이전트 지시 ──
const officeDraft = { text: "", cwd: null, cli: "claude", sending: false };
function tildePath(p) { return homeDir && p.startsWith(homeDir) ? "~" + p.slice(homeDir.length) : p; }
function officeMode(c) {
  const running = c.session && c.session.state === "running";
  if (c.opening) return "opening";
  if (!running) return "new";
  if (!c.cli) return c.atPrompt === true ? "new" : c.atPrompt === false ? "shell-busy" : "new";
  const p = c.turn && c.turn.phase;
  if (p === "idle") return "follow";
  if (p === "needs-human") return "human";
  return "working";
}
function renderOfficeForm(c) {
  const d = officeDraft;
  if (d.cwd == null || !folders.includes(d.cwd)) d.cwd = folders.includes(c.cwd) ? c.cwd : (folders[0] || "");
  const folder = el("select", { onchange: (e) => { d.cwd = e.target.value; } },
    ...folders.map((f) => el("option", { value: f, text: f === homeDir ? `~ (${S.home})` : tildePath(f), selected: d.cwd === f })));
  const cli = el("select", { onchange: (e) => { d.cli = e.target.value; } },
    ...["claude", "codex"].map((v) => el("option", { value: v, text: CLI_NAMES[v], selected: d.cli === v })));
  return el("div", { class: "card taskform", id: "taskform" },
    el("h3", { text: S.instruct }),
    el("textarea", { placeholder: S.instructPh, oninput: (e) => { d.text = e.target.value; } }, d.text),
    el("label", { class: "f", text: S.folder }), folder,
    el("div", { class: "row", style: "margin-top:6px" }, el("label", { class: "sub", text: S.cli }), el("div", { style: "width:130px" }, cli)),
    renderOfficeActions(c),
    renderQueue(c));
}
function renderOfficeActions(c) {
  const d = officeDraft;
  const mode = officeMode(c);
  const key = JSON.stringify([mode, d.sending, queueOf(c).length > 0]);
  const submit = (restart) => async () => {
    if (d.sending) return;
    if (!d.text.trim()) { toast(errText("empty")); return; }
    d.sending = true; refreshTaskActions();
    try {
      const r = await api("/office-agent", { method: "POST", body: { text: d.text, cwd: d.cwd || null, cli: d.cli, restart } });
      toast(r && r.outcome === "queued" ? S.queued : S.done);
      charsRev = null;
      d.text = "";
      const form = document.getElementById("taskform");
      if (form) form.replaceWith(renderOfficeForm(c));
      setTimeout(refreshNow, 400);
    } catch (e) { toast(e.message); }
    finally { d.sending = false; refreshTaskActions(); }
  };
  const restartBtn = confirmBtn(S.newSession, submit(true), "b danger");
  const note = { working: S.officeWorking, human: S.officeHuman, "shell-busy": S.officeShellBusy }[mode];
  let row;
  if (mode === "opening") row = [el("button", { class: "b primary", disabled: true, text: S.opening })];
  else if (mode === "new") row = [el("button", { class: "b primary", disabled: d.sending, text: S.send, onclick: submit(false) })];
  else if (mode === "follow") row = [el("button", { class: "b primary", disabled: d.sending, text: queueOf(c).length ? S.queueBtn : S.followUp, onclick: submit(false) }), restartBtn];
  else if (mode === "working" || mode === "human") row = [el("button", { class: "b primary", disabled: d.sending, text: S.queueBtn, onclick: submit(false) }), restartBtn];
  else row = [restartBtn];
  return el("div", { id: "taskact", "data-key": key },
    note && el("div", { class: "note", text: note }),
    mode !== "new" && mode !== "opening" && el("div", { class: "warn", text: "⚠ " + S.newWarn }),
    el("div", { class: "btns" }, ...row));
}

function firstLine(text, max = 90) {
  const line = (text || "").split("\n").find((l) => l.trim()) || "";
  return line.length > max ? line.slice(0, max) + "…" : line;
}
function renderLog(agentId, id = "log") {
  const box = el("div", { class: "log", id });
  box._agent = agentId;
  const log = logs.get(agentId);
  if (!log || !log.full) { box.append(el("div", { class: "empty", text: "…" })); return box; }
  box.dataset.last = String(log.last);
  if (!log.attached) { box.append(el("div", { class: "empty", text: S.noLog })); return box; }
  if (!log.items.length) { box.append(el("div", { class: "empty", text: S.noItems })); return box; }
  for (const it of log.items) placeItem(box, it);
  return box;
}
function placeItem(box, it) {
  const st = box._st || (box._st = { pending: [], last: null });
  const side = !!it.sidechain;
  if (it.kind === "image") {
    const top = box.firstElementChild;
    if (top && top._side === side && top._joins) { top.append(renderItem(it, box._agent)); return; }
  }
  if (it.kind === "tool_result") {
    const i = st.pending.findIndex((p) => p.side === side);
    const host = i >= 0 ? st.pending.splice(i, 1)[0].el : st.last && st.last.side === side ? st.last.el : null;
    if (host && host.parentNode === box) { host.append(renderItem(it, box._agent)); return; }
  }
  const blk = el("div", { class: "blk" }, renderItem(it, box._agent));
  blk._side = side;
  blk._joins = it.kind !== "text" || it.role === "user";
  box.prepend(blk);
  if (it.kind === "text") {
    st.pending = st.pending.filter((p) => p.side !== side);
    if (st.last && st.last.side === side) st.last = null;
  } else if (it.kind === "tool_use" && it.toolName) {
    st.pending.push({ el: blk, side });
    st.last = { el: blk, side };
  }
}
function messageBody(text) {
  const parts = [];
  let start = 0;
  let fence = "";
  function prose(end) {
    const paragraph = text.slice(start, end);
    if (paragraph.trim()) parts.push(el("div", { class: "msg-text", text: paragraph }));
  }
  for (const line of text.matchAll(/^ {0,3}(`{3,}|~{3,})([^\r\n]*)(?:\r?\n|$)/gm)) {
    const [, marker, info] = line;
    if (fence) {
      if (marker[0] !== fence[0] || marker.length < fence.length || info.trim()) continue;
      parts.push(textBlock(text.slice(start, line.index)));
      fence = "";
    } else {
      if (marker[0] === "`" && info.includes("`")) continue;
      prose(line.index);
      fence = marker;
    }
    start = line.index + line[0].length;
  }
  if (fence) parts.push(textBlock(text.slice(start)));
  else prose(text.length);
  return parts;
}
async function copyImage(img) {
  if (navigator.clipboard?.write && typeof ClipboardItem !== "undefined") {
    try {
      const png = img.decode().then(() => new Promise((resolve, reject) => {
        const canvas = el("canvas", { width: img.naturalWidth, height: img.naturalHeight });
        canvas.getContext("2d").drawImage(img, 0, 0);
        canvas.toBlob((blob) => blob ? resolve(blob) : reject(new Error()), "image/png");
      }));
      await Promise.all([png, navigator.clipboard.write([new ClipboardItem({ "image/png": png })])]);
      return toast(S.copied);
    } catch (_) {}
  }
  toast(S.imageCopyHelp);
}
function imageActions(img, url, name) {
  return el("div", { class: "content-actions", onclick: (e) => e.stopPropagation() },
    el("button", { class: "b", type: "button", text: S.copyImage, onclick: () => copyImage(img) }),
    downloadButton(S.downloadImage, url, name, S.imageGone));
}
const IMAGE_EXTENSIONS = { "image/png": "png", "image/jpeg": "jpg", "image/gif": "gif", "image/webp": "webp" };
function markdownBody(html) {
  const box = el("div", { class: "md" });
  box.innerHTML = html;
  for (const pre of box.querySelectorAll("pre")) {
    const code = pre.textContent;
    const block = el("div", { class: "text-block" });
    pre.replaceWith(block);
    block.append(el("pre", { text: code }), code && el("div", { class: "content-actions" }, copyButton(code)));
  }
  for (const table of box.querySelectorAll("table")) {
    const wrap = el("div", { class: "tablewrap" });
    table.replaceWith(wrap);
    wrap.append(table);
  }
  return box;
}
function speakerAvatar(role, agentId) {
  if (role === "user") return el("div", { class: "msgav", "aria-hidden": "true" }, "👤");
  const c = charOf(agentId);
  if (!c || c.office) return el("div", { class: "msgav", "aria-hidden": "true" }, "🤖");
  const initial = () => el("div", { class: "msgav", "aria-hidden": "true" }, (c.name || "?").slice(0, 1));
  if (!c.portrait) return initial();
  const img = el("img", { class: "msgav", alt: "", src: `/api/portrait/${encodeURIComponent(c.agentId)}?t=${token}&v=${c.portrait}` });
  img.onerror = () => img.replaceWith(initial());
  return img;
}
function renderItem(it, agentId) {
  const side = it.sidechain ? " side" : "";
  const agent = encodeURIComponent(agentId);
  if (it.kind === "image") {
    if (!it.image) return el("div", { class: "tool", text: S.imageGone });
    const source = () => it.image.source && el("div", { class: "imgsrc", text: it.image.source });
    const url = `/api/image/${agent}/${encodeURIComponent(it.image.id)}`;
    const src = `${url}?t=${token}`;
    const name = `image-${it.image.id}.${IMAGE_EXTENSIONS[it.image.mediaType] || "bin"}`;
    const img = el("img", { class: "shot" + side, loading: "lazy", alt: "", src });
    const opener = el("button", { type: "button", class: "shot-open", "aria-label": S.viewImage,
      onclick: () => openLightbox(src, url, name, opener) }, img);
    const wrap = el("div", { class: "imgwrap " + it.role }, opener, source(), imageActions(img, url, name));
    img.onerror = () => wrap.replaceWith(el("div", { class: "tool" }, S.imageGone, source()));
    return wrap;
  }
  if (it.kind === "text") {
    const epoch = logs.get(agentId).epoch;
    const actions = (it.files || []).map((file, index) =>
      downloadButton(`${S.download} · ${file.name}`, `/api/file/${agent}/${epoch}/${it.seq}/${index}`, file.name));
    if (it.text) actions.unshift(copyButton(it.text, S.copyMessage));
    const msg = el("div", { class: "msg " + it.role + side }, it.html ? markdownBody(it.html) : messageBody(it.text),
      actions.length > 0 && el("div", { class: "content-actions" }, actions));
    return el("div", { class: "msgrow " + it.role + side }, speakerAvatar(it.role, agentId), msg);
  }
  if (it.kind === "tool_use") {
    const head = (it.toolName ? it.toolName + ": " : "⤷ ") + firstLine(it.text);
    if (!it.toolName) return el("div", { class: "tool" + side, text: head });
    return el("details", { class: "tool" + side }, el("summary", { text: "🔧 " + head }), textBlock(it.text));
  }
  const n = (it.text || "").split("\n").length;
  return el("details", { class: "tool" + (it.isError ? " err" : "") + side },
    el("summary", { text: `↳ ${S.result} · ${n}${L === "ko" ? "" : " "}${S.lines} · ${firstLine(it.text, 60)}` }), textBlock(it.text));
}
function openLightbox(src, url, name, opener) {
  const img = el("img", { src, alt: "" });
  const actions = imageActions(img, url, name);
  const box = el("div", { class: "lb", onclick: () => close() },
    el("div", { class: "lb-content" }, img, actions));
  const onKey = (e) => { if (e.key === "Escape") close(); };
  const close = () => {
    box.remove();
    document.removeEventListener("keydown", onKey);
    opener.focus({ preventScroll: true });
  };
  document.addEventListener("keydown", onKey);
  document.body.append(box);
  actions.firstElementChild.focus({ preventScroll: true });
}

function renderTalks() {
  if (view.talk) return renderTalk();
  if (!talksData) return [el("div", { class: "empty", text: "…" })];
  if (!talksData.length) return [el("div", { class: "empty", text: S.noTalks })];
  const rows = [...talksData].sort((a, b) => (b.updatedAt || 0) - (a.updatedAt || 0));
  return rows.map((d) => el("div", { class: "card clickable", onclick: () => { view.talk = d.id; talkDetail = null; render(); refreshNow(); } },
    el("div", { class: "row" }, el("div", { class: "grow" }, el("div", { class: "name", text: d.topic || d.id }), el("div", { class: "sub", text: fmtTime(d.updatedAt) })),
      el("span", { class: "chip " + talkCls(d.state), text: S.talkState[d.state] || d.state }))));
}
function talkCls(state) {
  return { running: "working", wrapping_up: "working", waiting_user: "human", ready: "idle", paused: "bad" }[state] || "off";
}
let answerDraft = "";
function renderTalk() {
  const back = el("button", { class: "back", text: S.back, onclick: () => { view.talk = null; render(); } });
  if (!talkDetail || talkDetail.status.id !== view.talk) return [back, el("div", { class: "empty", text: "…" })];
  const st = talkDetail.status;
  const id = encodeURIComponent(st.id);
  const head = el("div", { class: "card" },
    el("div", { class: "row" }, el("div", { class: "grow" }, el("div", { class: "name", text: st.topic })), el("span", { class: "chip " + talkCls(st.state), text: S.talkState[st.state] || st.state })),
    el("div", { class: "sub", text: (st.participants || []).map((p) => (p.moderator ? "🎤 " : "") + (p.name || p.agentId) + (p.speaking ? " 💬" : "")).join(", ") }),
    st.attention && st.attention.detail && el("div", { class: "wait", text: (st.state === "waiting_user" ? S.question + ": " : "") + st.attention.detail }),
    st.state === "ready" && el("div", { class: "btns" }, el("button", { class: "b primary", text: S.start, onclick: () => act(`/discussions/${id}/start`) })),
    st.state === "waiting_user" && el("div", null,
      el("textarea", { placeholder: S.answerPh, oninput: (e) => { answerDraft = e.target.value; } }, answerDraft),
      el("div", { class: "btns" }, el("button", { class: "b primary", text: S.answer, onclick: async () => { if (await act(`/discussions/${id}/answer`, { text: answerDraft })) { answerDraft = ""; } } }))));
  const utts = talkDetail.transcript.map((u) => el("div", { class: "utt card" },
    el("div", { class: "who", text: `R${u.round} · ${u.name || (u.agentId ? u.agentId : u.kind)} · ${fmtTime(u.at)}` }), el("div", { class: "t", text: u.text })));
  return [back, head, ...utts];
}

function renderUsage() {
  if (!usageData) return [el("div", { class: "empty", text: "…" })];
  const out = [];
  for (const key of USAGE_KEYS) {
    const p = usageData[key];
    if (!p || !p.windows || !p.windows.length) continue;
    out.push(el("h2", { text: USAGE_NAMES[key] + (p.planLabel ? ` · ${p.planLabel}` : "") }));
    out.push(el("div", { class: "card" }, ...p.windows.map((w) => {
      const pct = Math.max(0, Math.min(100, Math.round(w.usedPercent)));
      return el("div", { style: "margin: 6px 0" },
        el("div", { class: "row" }, el("div", { class: "grow sub", text: [S.win[w.kind] || w.kind, w.label].filter(Boolean).join(" · ") }), el("div", { text: pct + "%" })),
        el("div", { class: "meter" + (pct >= 95 ? " full" : pct >= 75 ? " hot" : "") }, el("div", { style: `width:${pct}%` })),
        w.resetsAtMs && el("div", { class: "sub", text: `${S.resets} ${fmtTime(w.resetsAtMs)}` }));
    })));
  }
  return out.length ? out : [el("div", { class: "empty", text: S.usageNone })];
}

// ── 하단 사용량 바 ──
const USAGE_TTL = 60000;
async function refreshUsage() {
  if (usageData && Date.now() - (usageData._at || 0) < USAGE_TTL) return false;
  const d = await api("/usage");
  d._at = Date.now(); usageData = d;
  renderUsageBar();
  return true;
}
function barWindows(p) {
  if (p.provider === "antigravity") {
    const top = p.windows.reduce((a, w) => (w.usedPercent > a.usedPercent ? w : a));
    return p.windows.filter((w) => w.label === top.label).slice(0, 2);
  }
  const main = p.windows.filter((w) => w.kind === "session" || w.kind === "weekly" || w.kind === "monthly");
  return (main.length ? main : p.windows).slice(0, 3);
}
function renderUsageBar() {
  const bar = document.getElementById("usagebar");
  if (!token || unauthorized || !usageData) { bar.replaceChildren(); return; }
  const groups = [];
  for (const key of USAGE_KEYS) {
    const p = usageData[key];
    if (!p || !p.windows || !p.windows.length) continue;
    groups.push(el("span", { class: "ub-p" }, el("b", { text: USAGE_NAMES[key] }),
      ...barWindows(p).map((w) => {
        const pct = Math.max(0, Math.min(100, Math.round(w.usedPercent)));
        return el("span", { class: "ub-w" + (pct >= 95 ? " full" : pct >= 75 ? " hot" : ""), title: [S.win[w.kind] || w.kind, w.resetsAtMs && `${S.resets} ${fmtTime(w.resetsAtMs)}`].filter(Boolean).join(" · ") },
          S.winShort[w.kind.replace("_model", "")] || S.win[w.kind] || w.kind, el("span", { class: "ub-m" }, el("i", { style: `width:${pct}%` })), pct + "%");
      })));
  }
  bar.replaceChildren(...groups);
}
{
  const bar = document.getElementById("usagebar");
  const open = () => { view.tab = "usage"; view.agent = null; view.talk = null; store("agent-office-web-dash-tab", "usage"); render(); window.scrollTo(0, 0); };
  bar.addEventListener("click", open);
  bar.addEventListener("keydown", (e) => { if (e.key === "Enter" || e.key === " ") { e.preventDefault(); open(); } });
}

// ── 갱신 ──
const LOG_KEEP = 2000;
async function refreshChars() {
  const d = await api("/state");
  setHost(d.host, d.app);
  outputAt = d.outputAt || {}; serverNow = d.now || 0; serverNowAt = Date.now();
  let changed = false;
  if (d.rev !== charsRev) {
    charsRev = d.rev; chars = d.characters;
    officeData = d.officeAgent || null; folders = d.folders || []; homeDir = d.home || "";
    changed = true;
  }
  const key = [...(chars || []), ...(officeData ? [officeChar()] : [])].filter((c) => (liveOf(c) || {}).kind === "stalled").map((c) => c.agentId).join(",");
  if (key !== stalledKey) { stalledKey = key; changed = true; }
  return changed;
}
async function refreshTasks(allDone) {
  const qs = allDone ? "?allDone=1" : "";
  const d = await api("/tasks" + qs);
  const rev = d.rev || JSON.stringify(d);
  if (rev !== tasksRev || !tasksData) {
    tasksRev = rev;
    tasksData = d;
    return true;
  }
  return false;
}
let hostShown = null;
function setHost(host, app) {
  const key = (host || "") + "|" + app;
  if (key === hostShown) return;
  hostShown = key;
  document.getElementById("title").textContent = host || "Agent Office";
  document.getElementById("ver").textContent = (host ? "Agent Office " : "") + "v" + app;
  document.title = host ? `${host} · Agent Office` : "Agent Office";
}
async function refreshLog(agentId, full = true) {
  let prev = logs.get(agentId);
  if (prev && full && !prev.full) { logs.delete(agentId); prev = null; }
  const q = new URLSearchParams();
  if (prev) { q.set("after", prev.last); q.set("epoch", prev.epoch); }
  if (!full && !(prev && prev.full)) q.set("tail", PV_TAIL);
  const qs = q.toString();
  const d = await api(`/log/${encodeURIComponent(agentId)}${qs ? "?" + qs : ""}`);
  if (prev && prev.epoch === d.epoch) {
    if (!d.items.length && prev.attached === d.attached) return "same";
    const attachedChanged = prev.attached !== d.attached;
    const keep = prev.full ? LOG_KEEP : PV_TAIL;
    prev.items.push(...d.items);
    if (prev.items.length > keep) prev.items.splice(0, prev.items.length - keep);
    prev.last = d.last; prev.attached = d.attached;
    return attachedChanged ? "reset" : "append";
  }
  d.full = !q.has("tail");
  logs.set(agentId, d);
  return "reset";
}
function keepLogs(ids) { for (const k of [...logs.keys()]) if (!ids.includes(k)) logs.delete(k); }
let busy = false;
async function tick() {
  if (busy || !token || unauthorized || document.hidden) return;
  busy = true;
  try {
    let changed = false;
    const usageChanged = await refreshUsage().catch(() => false);
    if (view.tab === "chars" || view.tab === "tasks" || hostShown == null) changed = await refreshChars();
    if (view.tab === "chars" && syncPanes()) changed = true;
    if (view.tab === "chars" && view.agent) {
      const agentId = view.agent;
      keepLogs([agentId, ...(splitOn ? panes.keys() : [])]);
      const logMode = await refreshLog(agentId);
      if (agentId !== OFFICE) await refreshTasks(false).catch(() => false);
      if (view.agent === agentId) { updateAgentView(changed, logMode); refreshSent(); }
    } else if (view.tab === "chars" && splitOn) {
      const ids = [...panes.keys()];
      keepLogs(ids);
      const modes = new Map(await Promise.all(ids.map(async (id) => [id, await refreshLog(id).catch(() => "same")])));
      if (view.tab === "chars" && !view.agent && splitOn) updateSplit(changed, modes);
    } else if (view.tab === "chars") {
      const ids = allChars().filter((c) => isActive(c) && !isFolded(c)).map((c) => c.agentId);
      keepLogs(ids);
      await Promise.all(ids.map((id) => refreshLog(id, false).catch(() => "same")));
      if (view.tab === "chars" && !view.agent && !splitOn) { if (changed) render(); else refreshPreviews(); }
    } else if (view.tab === "tasks") {
      const allDone = !!(window.Dash && window.Dash.tasks && window.Dash.tasks.allDone);
      const tasksChanged = await refreshTasks(allDone);
      const ids = taskPreviewIds();
      keepLogs(ids);
      await Promise.all(ids.map((id) => refreshLog(id, false).catch(() => "same")));
      if (view.tab === "tasks" && window.Dash && window.Dash.tasks) { window.Dash.tasks.onTick(changed, tasksChanged); refreshPreviews(); }
    } else if (view.tab === "talks") {
      if (view.talk) {
        const d = await api(`/discussions/${encodeURIComponent(view.talk)}`);
        if (!talkDetail || JSON.stringify(d) !== JSON.stringify(talkDetail)) {
          const typing = document.activeElement && document.activeElement.tagName === "TEXTAREA";
          talkDetail = d; if (!typing) render();
        }
      } else {
        const d = await api("/discussions");
        if (JSON.stringify(d) !== JSON.stringify(talksData)) { talksData = d; render(); }
      }
    } else if (view.tab === "usage") {
      if (usageChanged) render();
    } else if (changed) render();
  } catch (e) {
    if (!unauthorized) console.warn(e);
  } finally { busy = false; }
}
function updateAgentView(changed, logMode) {
  const c = currentChar();
  const head = document.getElementById("head");
  if (!c || !head || !document.getElementById("taskform")) { if (changed || logMode !== "same") render(); return; }
  if (changed) { head.replaceWith(renderHead(c)); refreshTaskActions(); }
  refreshAsks();
  refreshMsg();
  refreshLive();
  refreshCost();
  if (logMode === "reset") document.getElementById("log").replaceWith(renderLog(c.agentId));
  else if (logMode === "append") appendLog(document.getElementById("log"));
}
function appendLog(box) {
  const agentId = box && box._agent;
  const log = agentId && logs.get(agentId);
  if (!box || !log) return;
  if (!box.dataset.last || box.querySelector(":scope > .empty")) { box.replaceWith(renderLog(agentId, box.id || null)); return; }
  const shown = Number(box.dataset.last);
  for (const it of log.items) if (it.seq > shown) placeItem(box, it);
  while (box.childElementCount > LOG_KEEP) box.lastElementChild.remove();
  box.dataset.last = String(log.last);
}
function updateUp() {
  document.getElementById("up").style.display = view.tab === "chars" && view.agent && window.scrollY > 600 ? "block" : "none";
}
document.getElementById("up").onclick = () => window.scrollTo(0, 0);
window.addEventListener("scroll", updateUp, { passive: true });
function refreshNow() { setTimeout(tick, 0); }
document.addEventListener("visibilitychange", () => { if (!document.hidden) refreshNow(); });

let booted = false;
function boot() {
  if (booted) return;
  booted = true;
  setInterval(tick, 2500);
  render();
  refreshNow();
}

window.Dash = {
  L, S, el, errText, toast, copyText, api, act, confirmBtn, fmtTime, firstLine,
  store, load, sstore, sload, avatar, phaseOf, waitingMessage, busyShell,
  leaf, cwdOf, placeOf, CLI_NAMES, WIDE, view, openAgent, openTask, render,
  refreshNow, setHeaderVar, boot, previewSlot,
  getChars: () => chars,
  charOf,
  getTasks: () => tasksData,
  refreshTasksNow: async (allDone) => { tasksRev = null; await refreshTasks(allDone); },
  invalidateTasks: () => { tasksRev = null; },
  invalidateChars: () => { charsRev = null; },
  tasks: null,
};
})();
