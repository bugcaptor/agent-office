"use strict";
(() => {
const D = window.Dash;
const { L, S, el, errText, toast, copyText, api, confirmBtn, fmtTime, firstLine, store, load, avatar, phaseOf, waitingMessage, busyShell, leaf, cwdOf, CLI_NAMES, WIDE, view } = D;

const T = {
  ko: {
    newWork: "+ 새 업무", resumeNew: "+ 새 업무 이어 쓰기", newWorkTitle: "새 업무",
    emptyBoard: "등록된 업무가 없습니다. '+ 새 업무'로 할 일을 만들어 보세요.",
    emptyGroup: "남은 업무가 없습니다.", pvOpen: "눌러서 담당 캐릭터의 대화 로그 보기",
    pickHint: "왼쪽 목록에서 업무를 고르거나 '+ 새 업무'를 눌러 만드세요.",
    unassigned: "미배정", missingAgent: "삭제된 캐릭터", missingWork: "(지워진 업무)", homeFolder: "홈",
    status: { review: "검토 대기", in_progress: "진행 중", todo: "할 일", done: "완료" },
    planChip: "플랜", rootRuns: "루트 작업 포함", rootRunsHint: "선행 작업을 마친 뒤 루트 업무도 마지막 단계로 실행합니다",
    planToggle: "플랜 단계 접기/펼치기",
    unsaved: "수정 중", blockedBy: (titles) => `대기: ${titles}`, dependents: (n) => `후속 ${n}`,
    waitingHuman: "⚠ 답을 기다림",
    openChat: "💬 대화 로그", openChatAnswer: "대화 로그에서 답하기 →",
    attentionBanner: (name, msg) => `${name}이(가) 답을 기다립니다${msg ? `: ${msg}` : ""}`,
    doneMore: (n) => `완료된 업무 전체 보기 (${n}건 더 있음)`,
    copyId: "ID 복사",
    // 편집기
    title: "제목", titlePh: "비우면 본문 첫 줄을 제목으로 씁니다", body: "내용", bodyPh: "업무 설명·목표·제약(선택)",
    assignee: "담당 캐릭터", statusLabel: "상태",
    agentOverride: "실행 CLI", useProfileDefault: "프로필 기본 사용", effectiveAgent: (a) => `실제 적용: ${a}`,
    model: "모델", cliDefault: "CLI 기본", modelsLoading: "모델 목록 불러오는 중…", modelsFailed: "목록을 못 불러왔습니다", retry: "다시 시도",
    effort: "추론 강도",
    dependsOn: "선행 작업", noDeps: "선행 작업이 없습니다.", addDep: "+ 선행 작업 추가…", noDepCand: "추가할 선행 작업이 없습니다",
    planRoot: "플랜 루트 (플랜 메모 전용)", planRootOn: "자동 진행 시 선행 작업만 순서대로 실행하고 루트는 메모로만 씁니다.", planRootOff: "선행 작업을 마친 뒤 이 루트 업무도 마지막 단계로 실행합니다.",
    limitsTitle: "자동 진행 상세 설정", limitsHint: "비워 두면 루트 설정 또는 기본값(후속 지시 3회 · 대기 10분)을 따릅니다.",
    maxFollowUps: "최대 후속 지시 (0~99)", permWait: "승인·질문 대기 시간(분, 1~1440)", inheritPh: "상속/기본",
    add: "추가", save: "저장", revert: "되돌리기", discard: "초안 버리기", deleteBtn: "삭제",
    markDone: "완료", send: "보내기", saveAndSend: "저장하고 보내기", sending: "보내는 중…", restartSend: "재시작하고 보내기",
    moveUp: "↑ 위로", moveDown: "↓ 아래로",
    // 자동 진행
    arTitle: "자동 진행", arChip: { running: "자동 진행 중", paused: "일시정지", finished: "자동 진행 끝남", usageWait: "사용량 대기" },
    arIdle: (n, name) => `${name}이(가) 남은 ${n}개 태스크를 순서대로 진행합니다.`,
    arRunning: (title) => `진행 중: ${title}`, arPreparing: "다음 태스크 준비 중…",
    arFollowUps: (c, m) => `후속 지시 ${c}/${m}`,
    arFinished: "모든 단계를 마쳤습니다.", arFinishedWithRoot: "루트 작업까지 모두 마쳤습니다.", arFinishedByRoot: "루트가 완료되어 자동 진행이 끝났습니다.",
    arProgress: (d, t) => `진행도 ${d}/${t}`, arCost: (c) => `판단자 비용 ${c}`,
    arUsageGuard: "사용량 대기 (한도 95% 초과 예상 시 리셋까지 자동 대기)",
    arUsageWaitDetail: (w) => `${w.provider === "codex" ? "Codex" : "Claude"} ${S.win[w.kind] || w.kind}${w.label ? ` · ${w.label}` : ""} 예상 ${Math.round(w.nowPercent)}%${w.addPercent != null ? ` (+${Math.round(w.addPercent)}%p)` : ""} / 한도 ${w.limit}%${w.resetsAtMs ? ` · 초기화 ${fmtTime(w.resetsAtMs)}` : ""}`,
    arStart: "자동 진행 시작", arPause: "일시정지", arResume: "계속", arSkipWait: "기다리지 않고 계속", arEnd: "끝내기", arClear: "기록 지우기",
    arEndHint: "끝내기는 자동 진행 기록만 지우며 업무 상태는 그대로 둡니다.",
    arOpenBusy: (title) => `진행 중인 플랜 열기 (${title})`,
    arLog: (n) => `진행 기록 (${n})`, arViaCtl: "CLI로 조작",
    arReasons: {
      "human-input": "터미널에 쓰다 만 사람 입력이 있어 멈췄습니다",
      "session-ended": "세션이 종료되어 멈췄습니다",
      "send-failed": "태스크를 보내지 못해 멈췄습니다",
      "judge-failed": "판단자 호출에 실패해 멈췄습니다",
      "followup-limit": (max) => `후속 지시 상한(${max}회)에 닿아 멈췄습니다`,
      "judge-stop": "판단자가 사람 확인을 요청했습니다",
      "task-busy": "태스크가 이미 진행 중이라 멈췄습니다",
      "task-changed": "진행 중 태스크가 바뀌어 멈췄습니다",
      "app-restarted": "앱을 다시 시작해 일시정지 상태로 복원되었습니다",
      user: "사용자가 일시정지했습니다",
      "ctl-pause": "CLI가 일시정지했습니다",
      escalated: "외부 CLI가 사람에게 넘겼습니다",
      "permission-timeout": "권한 승인·질문 대기 시간을 넘어 멈췄습니다",
      "usage-wait": "구독 사용량 한도에 가까워 리셋을 기다리는 중입니다",
    },
    arLogKinds: {
      start: () => "자동 진행 시작",
      send: (t) => `보냄: ${t}`,
      next: (t) => `통과: ${t}`,
      followup: (t) => `후속 지시: ${t}`,
      pause: () => "일시정지",
      resume: () => "계속",
      finish: () => "완료",
      "human-done": (t) => `사람이 완료 처리: ${t}`,
      "root-done": () => "루트 완료로 종료",
    },
    // 예약
    scTitle: "예약", scNone: "예약이 없습니다.", scOn: "켜짐", scOff: "꺼짐",
    scNext: (w) => `다음 실행: ${w}`, scLastRun: (w) => `마지막 실행: ${w}`,
    scLastSkip: (w, r) => `마지막 건너뜀(${w}): ${r}`,
    scOffDone: "완료된 업무는 예약이 꺼집니다.", scOffHint: "켜기를 누르면 다음 예정 시각에 실행됩니다.", scOffInvalid: "지난 시각의 예약입니다. 규칙을 바꾼 뒤 켜세요.",
    scKind: "반복", scKinds: { once: "한 번", interval: "일정 간격", daily: "매일", weekly: "매주" },
    scWhen: "실행 시각", scEvery: "간격", scUnits: { m: "분", h: "시간", d: "일" }, scDays: "요일", scTime: "시각",
    scAdd: "예약 추가", scChange: "규칙 바꾸기", scTurnOn: "켜기", scTurnOff: "끄기", scRemove: "예약 지우기", scSaveOn: "저장하고 켜기", scCancel: "취소",
    scChipOff: "예약 꺼짐", scChipRun: "예약 회차",
    scRuleOnce: (w) => `한 번 · ${w}`, scRuleMin: (n) => `${n}분마다`, scRuleHour: (n) => `${n}시간마다`, scRuleDay: (n) => `${n}일마다`,
    scRuleDaily: (t) => `매일 ${t}`, scRuleWeekly: (d, t) => `매주 ${d} ${t}`,
    scHintOnce: "정한 시각에 이 업무를 담당 캐릭터에게 보내고 예약을 지웁니다.",
    scHintOncePlan: "정한 시각에 이 플랜의 자동 진행을 시작하고 예약을 지웁니다.",
    scHintRec: "반복 예약은 회차마다 새 카드로 복사해 실행합니다. 원본은 직접 보낼 수 없습니다.",
    scHintRecPlan: "반복 플랜 예약은 회차마다 루트와 단계를 모두 복사해 자동 진행합니다.",
    scInvalidPast: "지금보다 뒤의 시각을 고르세요.", scInvalidInterval: "간격은 5분에서 30일 사이여야 합니다.", scInvalidDays: "요일을 하나 이상 고르세요.", scInvalidTime: "시각을 입력하세요.",
    scSkipReasons: { missed: "예정 시각을 놓침", "previous-open": "지난 회차가 아직 끝나지 않음", "agent-working": "캐릭터가 작업 중", "agent-session": "터미널에 CLI가 열려 있음", blocked: "보내기 조건이 막힘", failed: "시작 실패" },
  },
  en: {
    newWork: "+ New task", resumeNew: "+ Resume draft", newWorkTitle: "New task",
    emptyBoard: "No tasks yet. Click '+ New task' to create one.",
    emptyGroup: "No open tasks.", pvOpen: "Click to open the assignee's conversation log",
    pickHint: "Pick a task on the left or click '+ New task'.",
    unassigned: "Unassigned", missingAgent: "Deleted character", missingWork: "(deleted task)", homeFolder: "Home",
    status: { review: "Review", in_progress: "In progress", todo: "To do", done: "Done" },
    planChip: "Plan", rootRuns: "Includes root", rootRunsHint: "Runs the root task as the final step after prerequisites",
    planToggle: "Toggle plan steps",
    unsaved: "Unsaved", blockedBy: (titles) => `Waiting: ${titles}`, dependents: (n) => `${n} dependent(s)`,
    waitingHuman: "⚠ Needs answer",
    openChat: "💬 Conversation", openChatAnswer: "Answer in conversation →",
    attentionBanner: (name, msg) => `${name} is waiting for an answer${msg ? `: ${msg}` : ""}`,
    doneMore: (n) => `Show all completed tasks (${n} more)`,
    copyId: "Copy ID",
    title: "Title", titlePh: "Leave blank to use the first line of the body", body: "Body", bodyPh: "Details, goals, constraints (optional)",
    assignee: "Assignee", statusLabel: "Status",
    agentOverride: "CLI override", useProfileDefault: "Use profile default", effectiveAgent: (a) => `Effective: ${a}`,
    model: "Model", cliDefault: "CLI default", modelsLoading: "Loading models…", modelsFailed: "Couldn't load models", retry: "Retry",
    effort: "Effort",
    dependsOn: "Prerequisites", noDeps: "No prerequisites.", addDep: "+ Add prerequisite…", noDepCand: "No eligible prerequisites",
    planRoot: "Plan root (notes only)", planRootOn: "Auto-run runs prerequisites in order and uses this root as plan notes only.", planRootOff: "Runs this root task as the final step after prerequisites finish.",
    limitsTitle: "Auto-run limits", limitsHint: "Leave blank to inherit from plan root or defaults (3 follow-ups · 10 min wait).",
    maxFollowUps: "Max follow-ups (0–99)", permWait: "Approval/question wait (min, 1–1440)", inheritPh: "Inherit/default",
    add: "Add", save: "Save", revert: "Revert", discard: "Discard draft", deleteBtn: "Delete",
    markDone: "Done", send: "Send", saveAndSend: "Save & send", sending: "Sending…", restartSend: "Restart & send",
    moveUp: "↑ Up", moveDown: "↓ Down",
    arTitle: "Auto-run", arChip: { running: "Auto-running", paused: "Paused", finished: "Auto-run done", usageWait: "Usage wait" },
    arIdle: (n, name) => `${name} will run ${n} remaining task(s) in order.`,
    arRunning: (title) => `Running: ${title}`, arPreparing: "Preparing next task…",
    arFollowUps: (c, m) => `Follow-ups ${c}/${m}`,
    arFinished: "All steps finished.", arFinishedWithRoot: "All steps including root finished.", arFinishedByRoot: "Ended because root was marked done.",
    arProgress: (d, t) => `Progress ${d}/${t}`, arCost: (c) => `Judge cost ${c}`,
    arUsageGuard: "Usage guard (wait for reset if projected over 95%)",
    arUsageWaitDetail: (w) => `${w.provider === "codex" ? "Codex" : "Claude"} ${S.win[w.kind] || w.kind}${w.label ? ` · ${w.label}` : ""} est. ${Math.round(w.nowPercent)}%${w.addPercent != null ? ` (+${Math.round(w.addPercent)}%p)` : ""} / limit ${w.limit}%${w.resetsAtMs ? ` · resets ${fmtTime(w.resetsAtMs)}` : ""}`,
    arStart: "Start auto-run", arPause: "Pause", arResume: "Resume", arSkipWait: "Continue without waiting", arEnd: "End", arClear: "Clear log",
    arEndHint: "Ending only clears the auto-run record; task statuses stay as-is.",
    arOpenBusy: (title) => `Open active plan (${title})`,
    arLog: (n) => `Run log (${n})`, arViaCtl: "via CLI",
    arReasons: {
      "human-input": "Paused because of unsent human input in the terminal",
      "session-ended": "Paused because the session ended",
      "send-failed": "Paused because sending the task failed",
      "judge-failed": "Paused because the judge call failed",
      "followup-limit": (max) => `Paused at follow-up limit (${max})`,
      "judge-stop": "Judge asked for human input",
      "task-busy": "Paused because the task was already busy",
      "task-changed": "Paused because the task changed mid-run",
      "app-restarted": "Paused after app restart",
      user: "Paused by user",
      "ctl-pause": "Paused via CLI",
      escalated: "Escalated to human by CLI",
      "permission-timeout": "Paused after waiting too long for approval/answer",
      "usage-wait": "Waiting for usage window reset",
    },
    arLogKinds: {
      start: () => "Started auto-run",
      send: (t) => `Sent: ${t}`,
      next: (t) => `Accepted: ${t}`,
      followup: (t) => `Follow-up: ${t}`,
      pause: () => "Paused",
      resume: () => "Resumed",
      finish: () => "Finished",
      "human-done": (t) => `Marked done by human: ${t}`,
      "root-done": () => "Ended on root done",
    },
    scTitle: "Schedule", scNone: "No schedule.", scOn: "On", scOff: "Off",
    scNext: (w) => `Next run: ${w}`, scLastRun: (w) => `Last run: ${w}`,
    scLastSkip: (w, r) => `Last skipped (${w}): ${r}`,
    scOffDone: "Schedules are turned off when done.", scOffHint: "Turn on to run at the next scheduled time.", scOffInvalid: "This time has passed. Change the rule first.",
    scKind: "Repeat", scKinds: { once: "Once", interval: "Interval", daily: "Daily", weekly: "Weekly" },
    scWhen: "When", scEvery: "Every", scUnits: { m: "min", h: "hr", d: "day" }, scDays: "Days", scTime: "Time",
    scAdd: "Add schedule", scChange: "Change rule", scTurnOn: "Turn on", scTurnOff: "Turn off", scRemove: "Remove", scSaveOn: "Save & turn on", scCancel: "Cancel",
    scChipOff: "Schedule off", scChipRun: "Scheduled run",
    scRuleOnce: (w) => `Once · ${w}`, scRuleMin: (n) => `Every ${n}m`, scRuleHour: (n) => `Every ${n}h`, scRuleDay: (n) => `Every ${n}d`,
    scRuleDaily: (t) => `Daily ${t}`, scRuleWeekly: (d, t) => `Weekly ${d} ${t}`,
    scHintOnce: "Sends this task at the scheduled time and clears the schedule.",
    scHintOncePlan: "Starts auto-run for this plan at the scheduled time and clears the schedule.",
    scHintRec: "Recurring tasks clone a new card for each run; templates can't be sent directly.",
    scHintRecPlan: "Recurring plans clone the root and all steps for each run.",
    scInvalidPast: "Pick a time in the future.", scInvalidInterval: "Interval must be between 5 minutes and 30 days.", scInvalidDays: "Pick at least one day.", scInvalidTime: "Enter a time.",
    scSkipReasons: { missed: "Missed scheduled time", "previous-open": "Previous run still open", "agent-working": "Character is working", "agent-session": "CLI is open in terminal", blocked: "Send blocked", failed: "Failed to start" },
  },
}[L];

const WORK_STATUS_ORDER = ["review", "in_progress", "todo", "done"];
const WORK_CLIS = ["claude", "codex", "agy", "kilo", "pi"];
const EFFORTS = {
  claude: ["low", "medium", "high", "xhigh", "max"],
  codex: ["low", "medium", "high", "xhigh", "max"],
  agy: ["low", "medium", "high"],
  kilo: [],
  pi: ["off", "minimal", "low", "medium", "high", "xhigh", "max"],
};
const AUTO_RUN_LIMITS = { maxFollowUps: { min: 0, max: 99, def: 3 }, permissionWaitMinutes: { min: 1, max: 1440, def: 10 } };
const SCHEDULE_MIN = 5, SCHEDULE_MAX = 30 * 24 * 60;
const DAY_ORDER = [1, 2, 3, 4, 5, 6, 0];
const UNIT_MINUTES = { m: 1, h: 60, d: 1440 };
const MISSING_ASSIGNEE = "__missing__";
const COLLAPSE_KEY = "agent-office-web-dash-task-groups";

// ── 로컬 상태 (폴링 시 유지) ──
let allDone = false;
let creating = false;
let createDraft = null;
const edits = new Map(); // itemId -> partial patch
const schedEditors = new Map(); // itemId -> { open, draft }
const usageGuardPicks = new Map(); // itemId -> boolean
const needRestart = new Set(); // itemId
const busyOps = new Set(); // itemId or "create"
const modelCache = new Map(); // cli -> { status: "loading"|"ready"|"failed", list: [] }
const logOpen = new Set(); // itemId
const limitsOpen = new Set(); // itemId or "__new__"
const doneGroupOpen = new Set(); // groupKey
let collapsed = new Set((() => { try { return JSON.parse(load(COLLAPSE_KEY) || "[]"); } catch (_) { return []; } })());
let lastListKey = "";

function saveCollapsed() { store(COLLAPSE_KEY, JSON.stringify([...collapsed])); }
function setGroupCollapsed(key, isCollapsed) {
  if (isCollapsed) collapsed.add(key); else collapsed.delete(key);
  saveCollapsed();
}

// ── 순수 계산 (데스크톱 workpanels 로직 미러) ──
function allItems() {
  const d = D.getTasks();
  if (!d) return [];
  return [...(d.open || []), ...(d.done || [])];
}
function itemById(id) { return allItems().find((i) => i.id === id); }
function charsMap() {
  // 다른 머신 캐릭터에게 맡긴 업무도 그 캐릭터 묶음에 보이게 넣는다(앱에서 맡긴 것).
  const m = new Map();
  for (const c of D.getAllChars() || []) m.set(c.agentId, c);
  return m;
}
function workItemTitle(item) {
  const t = (item.title || "").trim();
  if (t) return t;
  const line = (item.body || "").split("\n").map((l) => l.trim()).find(Boolean) || "";
  return line.length > 80 ? line.slice(0, 80) + "…" : line;
}
function hasWorkContent(item) { return !!((item.title || "").trim() || (item.body || "").trim()); }
function workAgentLabel(agent) {
  if (!agent || !agent.cli) return "claude";
  return [agent.cli, agent.model, agent.effort].filter(Boolean).join(" / ");
}
function sameVal(a, b) { return JSON.stringify(a ?? null) === JSON.stringify(b ?? null); }
function hasDraftChanges(item, patch) {
  if (!patch) return false;
  for (const [k, v] of Object.entries(patch)) {
    if (k === "dependsOn") {
      if (!sameVal(v && v.length ? v : [], item.dependsOn && item.dependsOn.length ? item.dependsOn : [])) return true;
    } else if (k === "plan") {
      if (!!v !== !!item.plan) return true;
    } else if (!sameVal(v, item[k])) return true;
  }
  return false;
}
function validLimit(key, val) {
  if (val == null || val === "") return true;
  const { min, max } = AUTO_RUN_LIMITS[key];
  return typeof val === "number" && Number.isInteger(val) && val >= min && val <= max;
}
function validWorkLimits(v) { return validLimit("maxFollowUps", v.maxFollowUps) && validLimit("permissionWaitMinutes", v.permissionWaitMinutes); }
function effectiveLimit(key, root, task) {
  const o = task && task[key], p = root && root[key];
  if (o != null && validLimit(key, o)) return o;
  return p != null && validLimit(key, p) ? p : AUTO_RUN_LIMITS[key].def;
}
function reaches(from, target, byId) {
  const seen = new Set();
  const stack = [from];
  while (stack.length) {
    const id = stack.pop();
    if (id === target) return true;
    if (seen.has(id)) continue;
    seen.add(id);
    const it = byId.get(id);
    if (it && it.dependsOn) stack.push(...it.dependsOn);
  }
  return false;
}
function pendingPrerequisites(item, items) {
  const byId = new Map(items.map((i) => [i.id, i]));
  return (item.dependsOn || []).map((id) => byId.get(id)).filter((o) => o && o.status !== "done");
}
function dependencyCandidates(id, dependsOn, items) {
  const byId = new Map(items.map((i) => [i.id, i]));
  const deps = dependsOn || [];
  return items.filter((it) => it.id !== id && it.status !== "done" && !deps.includes(it.id) && !(id && reaches(it.id, id, byId)));
}
function dependentCount(id, items) { return items.filter((i) => (i.dependsOn || []).includes(id)).length; }
function planItems(root, items) {
  const byId = new Map(items.map((i) => [i.id, i]));
  const members = new Map();
  const stack = [...(root.dependsOn || [])];
  while (stack.length) {
    const id = stack.pop();
    const it = byId.get(id);
    if (!it || id === root.id || members.has(id)) continue;
    members.set(id, it);
    if (it.dependsOn) stack.push(...it.dependsOn);
  }
  const rank = (it) => (it.status === "done" ? 0 : 1) * 1e9 + (it.order || 0);
  const placed = new Set();
  const ordered = [];
  let rest = [...members.values()];
  while (rest.length) {
    const ready = rest.filter((it) => (it.dependsOn || []).every((id) => !members.has(id) || placed.has(id)));
    const batch = (ready.length ? ready : rest).sort((a, b) => rank(a) - rank(b)).slice(0, 1);
    for (const it of batch) { placed.add(it.id); ordered.push(it); }
    rest = rest.filter((it) => !placed.has(it.id));
  }
  return ordered;
}
function runSteps(root, items) {
  const plan = planItems(root, items);
  return root.plan || !plan.length ? plan : [...plan, root];
}
function assigneeKey(item, cmap) {
  if (!item.assigneeAgentId) return "";
  return cmap.has(item.assigneeAgentId) ? item.assigneeAgentId : MISSING_ASSIGNEE;
}
function planLayout(items, cmap) {
  const waitedOn = new Set(items.flatMap((i) => i.dependsOn || []));
  const roots = items.filter((i) => i.status !== "done" && i.dependsOn && i.dependsOn.length && !waitedOn.has(i.id))
    .sort((a, b) => (a.order || 0) - (b.order || 0));
  const members = new Map();
  const claimed = new Set();
  for (const root of roots) {
    const plan = planItems(root, items);
    if (!plan.length) continue;
    const key = assigneeKey(root, cmap);
    const moved = plan.filter((it) => !claimed.has(it.id) && (assigneeKey(it, cmap) === key || assigneeKey(it, cmap) === ""));
    for (const it of moved) claimed.add(it.id);
    members.set(root.id, moved);
  }
  return { members, claimed };
}
function isRecurring(item) { return !!(item.schedule && item.schedule.rule && item.schedule.rule.kind !== "once"); }
function planMemberIds(root, byId) {
  const seen = new Set();
  const stack = [...(root.dependsOn || [])];
  while (stack.length) {
    const id = stack.pop();
    if (id === root.id || seen.has(id)) continue;
    seen.add(id);
    const it = byId.get(id);
    if (it && it.dependsOn) stack.push(...it.dependsOn);
  }
  return [...seen];
}
function scheduleTemplateOf(item, items) {
  const byId = new Map(items.map((o) => [o.id, o]));
  return items.find((o) => isRecurring(o) && (o.id === item.id || planMemberIds(o, byId).includes(item.id)));
}
function busyRunRoot(root, items) {
  return items.find((it) => it.id !== root.id && it.autoRun && it.autoRun.state !== "finished" && it.assigneeAgentId === root.assigneeAgentId);
}
function workSendDisabledReason(c, items) {
  if (!c) return "work-send-agent-missing";
  // 다른 머신 캐릭터: 그 머신이 붙어 있어야 한다(저장소 확인은 앱이 한다).
  if (c.remote) return c.machine && c.machine.online ? null : "link-device-offline";
  if (items.some((i) => i.autoRun && i.autoRun.state === "running" && i.assigneeAgentId === c.agentId)) return "work-send-auto-run";
  if (c.session && c.session.state === "starting") return "work-send-busy";
  if (c.clockedOut) return "work-send-clocked-out";
  if (c.hasStartupCommand) return "work-send-profile-unsupported";
  return null;
}
function startBlockedReason(root, items, cmap, observerEnabled) {
  if (root.status === "done") return "auto-run-root-done";
  if (!root.assigneeAgentId || !cmap.has(root.assigneeAgentId)) return "auto-run-no-assignee";
  const open = runSteps(root, items).filter((i) => i.status !== "done");
  if (!open.length) return "auto-run-no-plan";
  if (open.length === 1 && open[0].id === root.id && root.status !== "todo") return "auto-run-root-busy";
  if (open.some((i) => scheduleTemplateOf(i, items)) || scheduleTemplateOf(root, items)) return "auto-run-schedule-template";
  if (busyRunRoot(root, items)) return "auto-run-runner-busy";
  // 단계마다 맡는 캐릭터(없으면 루트 담당)를 본다. 다른 머신 단계는 그 차례에 머신이 붙기를 기다린다.
  let local = false;
  for (const step of open) {
    const c = cmap.get(step.assigneeAgentId || root.assigneeAgentId);
    if (c && c.remote) continue;
    local = true;
    const prof = workSendDisabledReason(c, items);
    if (prof) return prof;
  }
  if (local && !observerEnabled) return "auto-run-observer-off";
  return null;
}
function sendBlockedReason(value, origItem, items, cmap) {
  if (!hasWorkContent(value)) return null;
  if (scheduleTemplateOf(origItem, items)) return "work-send-schedule-template";
  if (value.plan) return "work-send-plan-root";
  if (value.status !== "done" && pendingPrerequisites(value, items).length) return "work-send-blocked";
  return workSendDisabledReason(value.assigneeAgentId ? cmap.get(value.assigneeAgentId) : null, items);
}
function charNeedsHuman(agentId) {
  const c = agentId && D.charOf(agentId);
  return !!(c && phaseOf(c)[0] === "human");
}
function itemNeedsHuman(item) {
  if (!item.assigneeAgentId) return false;
  if (item.status === "in_progress") return charNeedsHuman(item.assigneeAgentId);
  if (item.autoRun && item.autoRun.state === "running") return charNeedsHuman(item.assigneeAgentId);
  return false;
}

// ── 예약 규칙 도우미 ──
function pad2(n) { return String(n).padStart(2, "0"); }
function formatLocalMinute(ms) {
  const d = new Date(ms);
  return `${d.getFullYear()}-${pad2(d.getMonth() + 1)}-${pad2(d.getDate())} ${pad2(d.getHours())}:${pad2(d.getMinutes())}`;
}
function localDateTime(date, time) {
  const dm = /^(\d{4})-(\d{2})-(\d{2})$/.exec(date || "");
  const tm = /^(\d{1,2}):(\d{2})$/.exec(time || "");
  if (!dm || !tm) return undefined;
  const y = +dm[1], mo = +dm[2], day = +dm[3], h = +tm[1], m = +tm[2];
  if (h >= 24 || m >= 60) return undefined;
  const at = new Date(y, mo - 1, day, h, m);
  return at.getFullYear() === y && at.getMonth() === mo - 1 && at.getDate() === day ? at.getTime() : undefined;
}
function weekdayName(day) {
  return new Date(2023, 0, 1 + day).toLocaleDateString(L === "ko" ? "ko-KR" : undefined, { weekday: "short" });
}
function ruleText(rule) {
  if (!rule) return "";
  switch (rule.kind) {
    case "once": return T.scRuleOnce(formatLocalMinute(rule.at));
    case "interval": {
      const u = rule.minutes % 1440 === 0 ? "d" : rule.minutes % 60 === 0 ? "h" : "m";
      const n = rule.minutes / UNIT_MINUTES[u];
      return u === "d" ? T.scRuleDay(n) : u === "h" ? T.scRuleHour(n) : T.scRuleMin(n);
    }
    case "daily": return T.scRuleDaily(rule.time);
    case "weekly": return T.scRuleWeekly(DAY_ORDER.filter((d) => (rule.days || []).includes(d)).map(weekdayName).join(", "), rule.time);
    default: return "";
  }
}
function validScheduleRule(rule, now = Date.now()) {
  if (!rule) return false;
  switch (rule.kind) {
    case "once": return Number.isFinite(rule.at) && rule.at > now;
    case "interval": return Number.isInteger(rule.minutes) && rule.minutes >= SCHEDULE_MIN && rule.minutes <= SCHEDULE_MAX;
    case "daily": return /^\d{1,2}:\d{2}$/.test(rule.time || "");
    case "weekly": return /^\d{1,2}:\d{2}$/.test(rule.time || "") && Array.isArray(rule.days) && rule.days.length > 0;
    default: return false;
  }
}
function makeSchedDraft(rule) {
  const soon = formatLocalMinute(Date.now() + 3600_000).split(" ");
  const base = { kind: "daily", date: soon[0], time: "09:00", amount: 1, unit: "h", days: [1, 2, 3, 4, 5] };
  if (!rule) return base;
  if (rule.kind === "once") { const [date, time] = formatLocalMinute(rule.at).split(" "); return { ...base, kind: "once", date, time }; }
  if (rule.kind === "interval") {
    const unit = rule.minutes % 1440 === 0 ? "d" : rule.minutes % 60 === 0 ? "h" : "m";
    return { ...base, kind: "interval", amount: rule.minutes / UNIT_MINUTES[unit], unit };
  }
  if (rule.kind === "daily") return { ...base, kind: "daily", time: rule.time };
  if (rule.kind === "weekly") return { ...base, kind: "weekly", days: [...(rule.days || [])], time: rule.time };
  return base;
}
function ruleOfDraft(d) {
  if (d.kind === "once") { const at = localDateTime(d.date, d.time); return at == null ? null : { kind: "once", at }; }
  if (d.kind === "interval") { const n = Number(d.amount); return Number.isFinite(n) ? { kind: "interval", minutes: Math.round(n * UNIT_MINUTES[d.unit]) } : null; }
  if (d.kind === "daily") return { kind: "daily", time: d.time };
  if (d.kind === "weekly") return { kind: "weekly", days: [...d.days].sort((a, b) => a - b), time: d.time };
  return null;
}

// ── 모델 목록 조회 ──
function ensureModels(cli, datalistId, statusEl) {
  if (!cli || cli === "kilo") return;
  let entry = modelCache.get(cli);
  if (entry && (entry.status === "ready" || entry.status === "loading")) {
    paintModels(entry, datalistId, statusEl);
    return;
  }
  entry = { status: "loading", list: [] };
  modelCache.set(cli, entry);
  paintModels(entry, datalistId, statusEl);
  api(`/models/${encodeURIComponent(cli)}`).then((list) => {
    entry.list = Array.isArray(list) ? list : [];
    entry.status = entry.list.length ? "ready" : "failed";
    paintModels(entry, datalistId, statusEl);
  }).catch(() => {
    entry.status = "failed";
    paintModels(entry, datalistId, statusEl);
  });
}
function paintModels(entry, datalistId, statusEl) {
  const dl = document.getElementById(datalistId);
  if (dl) dl.replaceChildren(...entry.list.map((m) => el("option", { value: m })));
  if (!statusEl) return;
  if (entry.status === "loading") statusEl.replaceChildren(el("span", { text: T.modelsLoading }));
  else if (entry.status === "failed") statusEl.replaceChildren(el("span", { text: T.modelsFailed + " " }));
  else statusEl.replaceChildren();
}

// ── API 액션 ──
async function runTaskApi(busyKey, fn, okMsg) {
  if (busyOps.has(busyKey)) return null;
  busyOps.add(busyKey);
  try {
    const r = await fn();
    if (okMsg) toast(okMsg);
    D.invalidateChars();
    await D.refreshTasksNow(allDone).catch(() => {});
    setTimeout(D.refreshNow, 250);
    return r;
  } catch (e) {
    toast(e.message);
    throw e;
  } finally {
    busyOps.delete(busyKey);
  }
}

function emptyCreateDraft(assigneeAgentId) {
  return { title: "", body: "", status: "todo", assigneeAgentId: assigneeAgentId || "", agent: null, dependsOn: [], plan: false, maxFollowUps: undefined, permissionWaitMinutes: undefined };
}
function startCreate(assigneeAgentId) {
  createDraft = createDraft ? { ...createDraft, ...(assigneeAgentId != null ? { assigneeAgentId } : {}) } : emptyCreateDraft(assigneeAgentId);
  creating = true;
  view.task = null;
  D.render();
  if (!WIDE.matches) window.scrollTo(0, 0);
}
function selectTask(id) {
  creating = false;
  view.task = id;
  D.render();
  if (!WIDE.matches && id) window.scrollTo(0, 0);
}

// ── 드래그 앤 드롭 순서 변경 ──
let dragId = null;
async function reorderCard(id, targetId, after) {
  if (!id || !targetId || id === targetId) return;
  try {
    await runTaskApi("reorder", () => api(`/tasks/${encodeURIComponent(id)}/reorder`, { method: "POST", body: { targetId, after } }));
    refreshListOnly();
    const curItem = view.task && itemById(view.task);
    if (curItem) { const sa = document.getElementById("tb-sendact"); if (sa) sa.replaceWith(renderSendActions(curItem)); }
  } catch (_) {}
}

// ── 보드 목록 렌더링 ──
function listFingerprint() {
  const td = D.getTasks();
  const cs = D.getChars() || [];
  const att = cs.map((c) => [c.agentId, phaseOf(c)[0], c.clockedOut, c.cli, c.atPrompt]);
  const dirtyIds = [...edits.keys()].filter((id) => { const it = itemById(id); return it && hasDraftChanges(it, edits.get(id)); });
  return JSON.stringify([td && td.rev, td && td.allDone, att, view.task, creating, !!createDraft, [...collapsed], [...doneGroupOpen], dirtyIds]);
}

function renderBoardList() {
  const td = D.getTasks();
  const chars = D.getChars();
  const wrap = el("div", { class: "tb-list", id: "tb-list" });
  if (!td || !chars) { wrap.append(el("div", { class: "empty", text: "…" })); return wrap; }

  const items = allItems();
  const cmap = charsMap();
  const plans = planLayout(items, cmap);
  const observerEnabled = td.observerEnabled !== false;

  const topbar = el("div", { class: "tb-topbar" },
    el("button", { class: "b primary", type: "button", text: createDraft && !creating ? T.resumeNew : T.newWork, onclick: () => startCreate() }),
    el("span", { class: "grow", text: `${(td.open || []).length} / ${items.length}` }));
  wrap.append(topbar);

  if (!items.length && !chars.length) {
    wrap.append(el("div", { class: "empty", text: T.emptyBoard }));
    return wrap;
  }

  // 캐릭터(오피스 순서) → 미배정 → 삭제된 캐릭터
  const remoteKeys = [...cmap.values()].filter((c) => c.remote && items.some((it) => it.assigneeAgentId === c.agentId)).map((c) => c.agentId);
  const keys = [...chars.map((c) => c.agentId), ...remoteKeys, "", MISSING_ASSIGNEE];
  const groups = keys.map((key) => ({
    key,
    char: cmap.get(key) || null,
    name: key === "" ? T.unassigned : key === MISSING_ASSIGNEE ? T.missingAgent
      : cmap.get(key).remote ? `${cmap.get(key).name} · ${(cmap.get(key).machine || {}).name || ""}` : cmap.get(key).name,
    items: items.filter((it) => assigneeKey(it, cmap) === key && !plans.claimed.has(it.id)),
  })).filter((g) => g.items.length > 0 || g.char);

  const renderCard = (item, role) => {
    const patch = edits.get(item.id);
    const dirty = hasDraftChanges(item, patch);
    const waitingOn = item.status === "done" ? [] : pendingPrerequisites(item, items);
    const deps = dependentCount(item.id, items);
    const plan = role && role.plan;
    const run = item.autoRun;
    const startable = plan && (!run || run.state === "finished");
    const blocked = startable ? startBlockedReason(item, items, cmap, observerEnabled) : null;
    const waitingHuman = itemNeedsHuman(item);
    const title = workItemTitle(item);
    const derived = !(item.title || "").trim();

    const badges = el("div", { class: "tb-badges" });
    if (plan) badges.append(el("span", { class: "tb-pill plan", text: T.planChip }));
    if (!plan || item.status !== "todo") badges.append(el("span", { class: "tb-pill st-" + item.status, text: T.status[item.status] || item.status }));
    if (dirty) badges.append(el("span", { class: "tb-pill dirty", text: T.unsaved }));
    if (waitingHuman) badges.append(el("span", { class: "tb-pill warn", text: T.waitingHuman }));
    if (run) {
      const arKey = run.pauseReason === "usage-wait" && run.state === "paused" ? "usageWait" : run.state;
      badges.append(el("span", { class: "tb-pill ar-" + run.state, text: T.arChip[arKey] || run.state }));
    }
    if (item.schedule) {
      const scText = item.schedule.enabled && item.schedule.nextAt != null
        ? `${isRecurring(item) ? "↻ " : "⏰ "}${fmtTime(item.schedule.nextAt)}`
        : T.scChipOff;
      badges.append(el("span", { class: "tb-pill sched" + (item.schedule.enabled ? "" : " off"), text: scText }));
    }
    if (item.scheduledFrom) badges.append(el("span", { class: "tb-pill", text: T.scChipRun }));
    if (plan && !item.plan) badges.append(el("span", { class: "tb-pill", title: T.rootRunsHint, text: T.rootRuns }));
    if (plan) badges.append(el("span", { class: "tb-pill", text: `${plan.done}/${plan.total}` }));
    if (role && role.member && role.member.unassigned) badges.append(el("span", { class: "tb-pill", text: T.unassigned }));
    if (!plan && waitingOn.length) badges.append(el("span", { class: "tb-pill warn", text: T.blockedBy(waitingOn.map(workItemTitle).join(", ")) }));
    if (!plan && !(role && role.member) && deps > 0 && item.status !== "done") badges.append(el("span", { class: "tb-pill", text: T.dependents(deps) }));
    if (item.lastError) badges.append(el("span", { class: "tb-pill bad", text: "!" }));

    const btn = el("button", { class: "tb-card-btn", type: "button", onclick: () => selectTask(item.id) },
      el("div", { class: "tb-card-title" + (derived ? " derived" : ""), text: title }),
      badges);

    const head = el("div", { class: "tb-card-head" });
    if (plan) {
      head.append(el("button", { class: "tb-plan-toggle", type: "button", disabled: !plan.toggle, "aria-expanded": String(plan.expanded), "aria-label": T.planToggle, text: plan.expanded ? "▾" : "▸", onclick: (e) => { e.stopPropagation(); plan.toggle && plan.toggle(); } }));
    }
    head.append(btn);
    if (startable) {
      const busy = blocked === "auto-run-runner-busy" ? busyRunRoot(item, items) : null;
      const hint = blocked ? (errText(blocked) + (busy ? ` (${workItemTitle(busy)})` : "")) : T.arStart;
      head.append(el("button", { class: "tb-plan-start", type: "button", disabled: !!blocked || busyOps.has(item.id), title: hint, "aria-label": T.arStart, text: "▶", onclick: async (e) => {
        e.stopPropagation();
        try { await runTaskApi(item.id, () => api(`/tasks/${encodeURIComponent(item.id)}/autorun`, { method: "POST", body: { action: "start", usageGuard: true } }), S.done); } catch (_) {}
      } }));
    }

    const card = el("article", {
      class: `tb-card status-${item.status}${plan ? " is-plan" : ""}${waitingHuman ? " waiting" : ""}${view.task === item.id ? " selected" : ""}`,
      "data-id": item.id,
      draggable: "true",
      ondragstart: (e) => { dragId = item.id; e.dataTransfer.effectAllowed = "move"; try { e.dataTransfer.setData("text/plain", item.id); } catch (_) {} },
      ondragend: () => { dragId = null; card.classList.remove("drag-before", "drag-after"); },
      ondragover: (e) => {
        if (!dragId || dragId === item.id) return;
        const src = itemById(dragId);
        if (!src || src.status !== item.status) return;
        e.preventDefault();
        const r = card.getBoundingClientRect();
        const after = e.clientY > r.top + r.height / 2;
        card.classList.toggle("drag-after", after);
        card.classList.toggle("drag-before", !after);
      },
      ondragleave: () => { card.classList.remove("drag-before", "drag-after"); },
      ondrop: (e) => {
        card.classList.remove("drag-before", "drag-after");
        if (!dragId || dragId === item.id) return;
        e.preventDefault();
        const r = card.getBoundingClientRect();
        reorderCard(dragId, item.id, e.clientY > r.top + r.height / 2);
        dragId = null;
      },
    }, head);
    const pvChar = item.status === "in_progress" && item.assigneeAgentId && D.charOf(item.assigneeAgentId);
    if (pvChar) {
      const slot = D.previewSlot(pvChar);
      slot.classList.add("tb-pv");
      slot.title = T.pvOpen;
      slot.onclick = () => D.openAgent(pvChar.agentId);
      card.append(slot);
    }
    return card;
  };

  const renderPlanBlock = (root, groupKey) => {
    const members = plans.members.get(root.id) || [];
    const steps = runSteps(root, items);
    const pkey = `plan:${root.id}`;
    const expanded = !collapsed.has(pkey) || members.some((m) => m.id === view.task);
    const toggle = members.length ? () => { setGroupCollapsed(pkey, expanded); refreshListOnly(); } : null;
    return el("div", { class: "tb-plan" },
      renderCard(root, { plan: { done: steps.filter((s) => s.status === "done").length, total: steps.length, expanded, toggle } }),
      expanded && members.length > 0 && el("div", { class: "tb-plan-members" },
        ...members.map((m) => renderCard(m, { member: { unassigned: assigneeKey(m, cmap) !== groupKey } }))));
  };

  for (const g of groups) {
    const open = WORK_STATUS_ORDER.filter((st) => st !== "done").flatMap((st) =>
      g.items.filter((i) => i.status === st).sort((a, b) => (a.order || 0) - (b.order || 0)));
    const done = g.items.filter((i) => i.status === "done").sort((a, b) => (b.updatedAt || 0) - (a.updatedAt || 0));
    const remaining = open.length + open.reduce((sum, it) => sum + ((plans.members.get(it.id) || []).filter((m) => m.status !== "done").length), 0);
    const proj = g.char ? leaf(cwdOf(g.char)) : "";
    const isOpen = !collapsed.has(g.key);

    const summary = el("summary", null,
      g.char && avatar(g.char),
      el("span", { class: "tb-gname" }, el("strong", { text: g.name }), proj && el("small", { text: proj, title: cwdOf(g.char) })),
      el("span", { class: "tb-count" + (remaining ? "" : " zero"), text: String(remaining) }),
      g.char && !g.char.remote && el("button", { class: "tb-add", type: "button", title: `${g.name} · ${T.newWork}`, text: "+", onclick: (e) => { e.preventDefault(); e.stopPropagation(); startCreate(g.key); } }));

    const body = el("div", { class: "tb-group-body" });
    if (!open.length && !done.length) body.append(el("div", { class: "tb-group-empty", text: T.emptyGroup }));
    for (const it of open) body.append(plans.members.has(it.id) ? renderPlanBlock(it, g.key) : renderCard(it));
    if (done.length) {
      const doneOpen = doneGroupOpen.has(g.key) || done.some((i) => i.id === view.task || (i.autoRun && i.autoRun.state !== "finished"));
      body.append(el("details", { class: "tb-done", open: doneOpen, ontoggle: (e) => { if (e.currentTarget.open) doneGroupOpen.add(g.key); else doneGroupOpen.delete(g.key); } },
        el("summary", { text: `${T.status.done} (${done.length})` }),
        ...done.map((it) => renderCard(it))));
    }

    wrap.append(el("details", { class: "tb-group", open: isOpen, ontoggle: (e) => setGroupCollapsed(g.key, !e.currentTarget.open) }, summary, body));
  }

  const hiddenDone = (td.doneTotal || 0) - (td.done || []).length;
  if (hiddenDone > 0 && !allDone) {
    wrap.append(el("button", { class: "b tb-more-done", type: "button", text: T.doneMore(hiddenDone), onclick: () => {
      allDone = true;
      D.invalidateTasks();
      D.refreshNow();
    } }));
  }

  return wrap;
}

function refreshListOnly() {
  const cur = document.getElementById("tb-list");
  if (!cur) return;
  const scrollY = window.scrollY;
  cur.replaceWith(renderBoardList());
  lastListKey = listFingerprint();
  window.scrollTo(0, scrollY);
}

// ── 편집기 공통 ──
function renderEditorForm(opts) {
  const { itemId, value, creating: isNew, dirty, onChange, onSave, onDiscard } = opts;
  const items = allItems();
  const chars = D.getChars() || [];
  const cmap = charsMap();
  const dependsOn = value.dependsOn || [];
  const candidates = dependencyCandidates(itemId, dependsOn, items);
  const canSave = !busyOps.has(itemId || "create") && dirty && hasWorkContent(value) && validWorkLimits(value);
  const agent = value.agent || null;
  const cli = agent ? agent.cli : "";
  const dlId = `tb-models-${itemId || "new"}`;
  const modelStatus = el("div", { class: "tb-hint" });

  const inherited = value.assigneeAgentId && cmap.get(value.assigneeAgentId) && cmap.get(value.assigneeAgentId).defaultAgent
    ? cmap.get(value.assigneeAgentId).defaultAgent
    : { cli: "claude" };

  const form = el("form", { class: "tb-form", onsubmit: (e) => { e.preventDefault(); const sb = document.getElementById("tb-save-btn"); if (sb && !sb.disabled) onSave(); } });

  // 제목
  form.append(el("label", { class: "f", text: T.title }),
    el("input", { type: "text", maxlength: "500", value: value.title || "", placeholder: (!value.title && value.body ? firstLine(value.body, 60) : T.titlePh), oninput: (e) => onChange({ title: e.target.value }) }));

  // 본문
  form.append(el("label", { class: "f", text: T.body }),
    el("textarea", { rows: "5", placeholder: T.bodyPh, oninput: (e) => onChange({ body: e.target.value }) }, value.body || ""));

  // 담당 + 상태 (2열)
  const assigneeSel = el("select", { onchange: (e) => onChange({ assigneeAgentId: e.target.value || undefined }, true) },
    el("option", { value: "", text: T.unassigned, selected: !value.assigneeAgentId }),
    value.assigneeAgentId && !cmap.has(value.assigneeAgentId) && el("option", { value: value.assigneeAgentId, text: T.missingAgent, selected: true }),
    ...chars.map((c) => el("option", { value: c.agentId, text: c.name, selected: value.assigneeAgentId === c.agentId })));

  const statusSel = el("select", { onchange: (e) => onChange({ status: e.target.value }, true) },
    ...WORK_STATUS_ORDER.map((st) => el("option", { value: st, text: T.status[st], selected: value.status === st })));

  form.append(el("div", { class: "tb-grid2" },
    el("div", null, el("label", { class: "f", text: T.assignee }), assigneeSel),
    el("div", null, el("label", { class: "f", text: T.statusLabel }), statusSel)));

  // CLI / 모델 / 추론 강도
  const cliSel = el("select", { onchange: (e) => {
    const nextCli = e.target.value;
    onChange({ agent: nextCli ? { cli: nextCli } : null }, true);
  } },
    el("option", { value: "", text: T.useProfileDefault, selected: !cli }),
    ...WORK_CLIS.map((v) => el("option", { value: v, text: CLI_NAMES[v] || v, selected: cli === v })));

  const agentRow = el("div", { class: "tb-grid2" },
    el("div", null, el("label", { class: "f", text: T.agentOverride }), cliSel));

  if (cli) {
    const modelInput = el("input", { type: "text", list: dlId, value: (agent && agent.model) || "", placeholder: T.cliDefault, oninput: (e) => onChange({ agent: { ...agent, model: e.target.value || undefined } }) });
    const dl = el("datalist", { id: dlId });
    agentRow.append(el("div", null, el("label", { class: "f", text: T.model }), modelInput, dl, modelStatus));
    if (cli !== "kilo") {
      const effs = EFFORTS[cli] || [];
      const curEff = (agent && agent.effort) || "";
      const effSel = el("select", { onchange: (e) => onChange({ agent: { ...agent, effort: e.target.value || undefined } }, true) },
        el("option", { value: "", text: T.cliDefault, selected: !curEff }),
        curEff && !effs.includes(curEff) && el("option", { value: curEff, text: curEff, selected: true }),
        ...effs.map((ef) => el("option", { value: ef, text: ef, selected: curEff === ef })));
      agentRow.append(el("div", null, el("label", { class: "f", text: T.effort }), effSel));
    }
    setTimeout(() => ensureModels(cli, dlId, modelStatus), 0);
  }
  form.append(agentRow);
  if (!cli) form.append(el("div", { class: "tb-hint", text: T.effectiveAgent(workAgentLabel(inherited)) }));

  // 선행 작업
  const depWrap = el("div", { class: "tb-deps" }, el("label", { class: "f", text: T.dependsOn }));
  if (dependsOn.length) {
    const ul = el("ul", { class: "tb-dep-list" });
    for (const depId of dependsOn) {
      const other = items.find((i) => i.id === depId);
      const title = other ? workItemTitle(other) : T.missingWork;
      ul.append(el("li", { class: "tb-dep-item" },
        el("span", { class: "grow" + (other && other.status === "done" ? " sub" : ""), text: title }),
        other && el("span", { class: "tb-pill st-" + other.status, text: T.status[other.status] }),
        el("button", { class: "b", type: "button", text: "×", onclick: () => {
          const next = dependsOn.filter((d) => d !== depId);
          onChange({ dependsOn: next }, true);
        } })));
    }
    depWrap.append(ul);
  } else {
    depWrap.append(el("div", { class: "tb-hint", text: T.noDeps }));
  }
  const depSel = el("select", { disabled: !candidates.length, onchange: (e) => {
    if (!e.target.value) return;
    onChange({ dependsOn: [...dependsOn, e.target.value] }, true);
  } },
    el("option", { value: "", text: candidates.length ? T.addDep : T.noDepCand }),
    ...candidates.map((it) => el("option", { value: it.id, text: workItemTitle(it) })));
  depWrap.append(depSel);
  form.append(depWrap);

  // 플랜 루트 체크박스 (선행 작업이 있거나 이미 켜져 있을 때)
  if (dependsOn.length > 0 || value.plan) {
    form.append(el("label", { class: "inline" },
      el("input", { type: "checkbox", checked: !!value.plan, onchange: (e) => onChange({ plan: e.target.checked }, true) }),
      T.planRoot),
      el("div", { class: "tb-hint", text: value.plan ? T.planRootOn : T.planRootOff }));
  }

  // 자동 진행 상세 설정 (접힘)
  const limKey = itemId || "__new__";
  form.append(el("details", { class: "tb-fold", open: limitsOpen.has(limKey), ontoggle: (e) => { if (e.currentTarget.open) limitsOpen.add(limKey); else limitsOpen.delete(limKey); } },
    el("summary", { text: T.limitsTitle }),
    el("div", { class: "tb-hint", text: T.limitsHint }),
    el("div", { class: "tb-grid2" },
      el("div", null,
        el("label", { class: "f", text: T.maxFollowUps }),
        el("input", { type: "number", min: "0", max: "99", step: "1", value: value.maxFollowUps != null ? String(value.maxFollowUps) : "", placeholder: T.inheritPh,
          oninput: (e) => onChange({ maxFollowUps: e.target.value === "" ? null : Number(e.target.value) }) })),
      el("div", null,
        el("label", { class: "f", text: T.permWait }),
        el("input", { type: "number", min: "1", max: "1440", step: "1", value: value.permissionWaitMinutes != null ? String(value.permissionWaitMinutes) : "", placeholder: T.inheritPh,
          oninput: (e) => onChange({ permissionWaitMinutes: e.target.value === "" ? null : Number(e.target.value) }) })))));

  // 저장 / 되돌리기 줄
  const saveBtn = el("button", { class: "b primary", type: "submit", id: "tb-save-btn", disabled: !canSave, text: isNew ? T.add : T.save });
  const discardBtn = el("button", { class: "b", type: "button", id: "tb-discard-btn", style: dirty ? "" : "visibility:hidden", text: isNew ? T.discard : T.revert, onclick: onDiscard });
  form.append(el("div", { class: "btns" }, discardBtn, el("span", { class: "tb-spacer" }), saveBtn));
  return form;
}

function updateSaveBar(itemId, value, dirty) {
  const canSave = !busyOps.has(itemId || "create") && dirty && hasWorkContent(value) && validWorkLimits(value);
  const sb = document.getElementById("tb-save-btn");
  if (sb) sb.disabled = !canSave;
  const db = document.getElementById("tb-discard-btn");
  if (db) db.style.visibility = dirty ? "" : "hidden";
}

// ── 자동 진행 섹션 ──
function pauseText(run) {
  const lastLog = run.log && run.log[run.log.length - 1];
  const max = (lastLog && lastLog.followUpLimit != null) ? lastLog.followUpLimit : AUTO_RUN_LIMITS.maxFollowUps.def;
  const r = T.arReasons[run.pauseReason || "user"];
  const reason = typeof r === "function" ? r(max) : (r || run.pauseReason || "");
  if (!run.pauseDetail) return { reason };
  const literal = ["judge-stop", "followup-limit", "task-busy", "task-changed", "session-ended", "escalated", "permission-timeout"].includes(run.pauseReason);
  return { reason, detail: literal ? run.pauseDetail : errText(run.pauseDetail) };
}

function renderAutoRunSection(root) {
  const td = D.getTasks() || {};
  const items = allItems();
  const cmap = charsMap();
  const run = root.autoRun || null;
  const plan = planItems(root, items);
  const box = el("section", { class: "tb-section", id: "tb-autorun" });
  if (!run && !plan.length) {
    box.style.display = "none";
    box.dataset.key = "none";
    return box;
  }
  const steps = runSteps(root, items);
  const done = steps.filter((s) => s.status === "done").length;
  const titles = new Map(items.map((i) => [i.id, workItemTitle(i)]));
  const runner = root.assigneeAgentId && cmap.get(root.assigneeAgentId) ? cmap.get(root.assigneeAgentId).name : T.unassigned;
  const observerEnabled = td.observerEnabled !== false;
  const blocked = run && run.state !== "finished" ? null : startBlockedReason(root, items, cmap, observerEnabled);
  const busyRoot = blocked === "auto-run-runner-busy" ? busyRunRoot(root, items) : null;
  const current = run && run.currentId ? items.find((i) => i.id === run.currentId) : null;
  const usageWaiting = !!(run && run.state === "paused" && run.pauseReason === "usage-wait");
  const guardVal = run ? run.usageGuard !== false : (usageGuardPicks.has(root.id) ? usageGuardPicks.get(root.id) : true);
  const isBusy = busyOps.has("ar:" + root.id);

  box.dataset.key = JSON.stringify([run, done, steps.length, blocked, busyRoot && busyRoot.id, guardVal, isBusy, td.sessionCostEnabled]);
  box.append(el("h3", { text: T.arTitle }));

  if (!run) {
    box.append(el("div", { class: "tb-hint", text: T.arIdle(steps.length - done, runner) }));
  } else if (run.state === "running") {
    const curText = current
      ? `${T.arRunning(workItemTitle(current))} · ${T.arFollowUps(run.followUps || 0, effectiveLimit("maxFollowUps", root, current))}`
      : T.arPreparing;
    box.append(el("div", { style: "font-size:13.5px;font-weight:500", text: curText }));
  } else if (run.state === "paused") {
    const p = pauseText(run);
    const pbox = el("div", { class: "tb-ar-paused" },
      el("p", { style: "font-weight:600", text: p.reason }),
      p.detail && el("p", { class: "tb-ar-detail", text: p.detail }),
      usageWaiting && run.usageWait && el("p", { class: "tb-hint", text: T.arUsageWaitDetail(run.usageWait) }));
    box.append(pbox);
  } else if (run.state === "finished") {
    const lastKind = run.log && run.log.length ? run.log[run.log.length - 1].kind : null;
    const msg = lastKind === "root-done" ? T.arFinishedByRoot : root.plan ? T.arFinished : T.arFinishedWithRoot;
    box.append(el("div", { style: "font-size:13.5px", text: msg }));
  }

  const progParts = [T.arProgress(done, steps.length)];
  if (run && td.sessionCostEnabled && run.costUsd > 0) progParts.push(T.arCost(`$${run.costUsd.toFixed(2)}`));
  box.append(el("div", { class: "tb-hint", text: progParts.join(" · ") }));

  // 사용량 대기 체크박스
  box.append(el("label", { class: "inline", style: "font-size:13px" },
    el("input", { type: "checkbox", checked: guardVal, disabled: isBusy, onchange: async (e) => {
      const on = e.target.checked;
      usageGuardPicks.set(root.id, on);
      if (run && run.state !== "finished") {
        try { await runTaskApi("ar:" + root.id, () => api(`/tasks/${encodeURIComponent(root.id)}/autorun`, { method: "POST", body: { action: "usage-guard", usageGuard: on } })); } catch (_) {}
      }
    } }),
    T.arUsageGuard));

  // 버튼 줄
  const btns = el("div", { class: "btns" });
  const arPost = (action, extra) => async () => {
    try {
      await runTaskApi("ar:" + root.id, () => api(`/tasks/${encodeURIComponent(root.id)}/autorun`, { method: "POST", body: { action, ...(extra || {}) } }), S.done);
      refreshListOnly();
      const fresh = itemById(root.id);
      if (fresh) { const cur = document.getElementById("tb-autorun"); if (cur) cur.replaceWith(renderAutoRunSection(fresh)); }
    } catch (_) {}
  };
  if (run && run.state !== "running") {
    btns.append(el("button", { class: "b", type: "button", disabled: isBusy, text: run.state === "finished" ? T.arClear : T.arEnd, onclick: arPost("end") }));
  }
  btns.append(el("span", { class: "tb-spacer" }));
  if (run && (run.state === "running" || usageWaiting)) {
    btns.append(el("button", { class: "b", type: "button", disabled: isBusy, text: T.arPause, onclick: arPost("pause") }));
  }
  if (run && run.state === "paused") {
    btns.append(el("button", { class: "b primary", type: "button", disabled: isBusy, text: usageWaiting ? T.arSkipWait : T.arResume, onclick: arPost("resume") }));
  }
  if (!run || run.state === "finished") {
    btns.append(el("button", { class: "b primary", type: "button", disabled: !!blocked || isBusy, text: T.arStart, onclick: arPost("start", { usageGuard: guardVal }) }));
  }
  box.append(btns);

  if (blocked) {
    const h = el("div", { class: "tb-hint err" }, errText(blocked));
    if (busyRoot) {
      h.append(" ", el("button", { class: "b", type: "button", style: "padding:2px 8px;font-size:12px", text: T.arOpenBusy(workItemTitle(busyRoot)), onclick: () => selectTask(busyRoot.id) }));
    }
    box.append(h);
  }
  if (run && run.state !== "running") box.append(el("div", { class: "tb-hint", text: T.arEndHint }));

  if (run && run.log && run.log.length) {
    const ol = el("ol", { class: "tb-log" });
    for (const entry of run.log) {
      const time = new Date(entry.at).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
      const title = titles.get(entry.taskId || "") || T.missingWork;
      const headFn = T.arLogKinds[entry.kind];
      const head = headFn ? headFn(title) : entry.kind;
      let txt = "";
      if (entry.kind === "pause") {
        const rr = T.arReasons[entry.text || "user"];
        txt = typeof rr === "function" ? rr(entry.followUpLimit != null ? entry.followUpLimit : AUTO_RUN_LIMITS.maxFollowUps.def) : (rr || entry.text || "");
      } else if (entry.text === "ctl") txt = T.arViaCtl;
      else if (entry.text && entry.text.trim()) txt = entry.text.trim();
      ol.append(el("li", null, el("time", { text: time }), el("strong", { text: head }), txt && el("span", { class: "tb-log-text", text: txt })));
    }
    box.append(el("details", { class: "tb-fold", open: logOpen.has(root.id), ontoggle: (e) => { if (e.currentTarget.open) logOpen.add(root.id); else logOpen.delete(root.id); } },
      el("summary", { text: T.arLog(run.log.length) }), ol));
  }
  return box;
}

// ── 예약 섹션 ──
function renderScheduleSection(item) {
  const items = allItems();
  const sc = item.schedule || null;
  const ed = schedEditors.get(item.id) || { open: false, draft: makeSchedDraft(sc && sc.rule) };
  const box = el("section", { class: "tb-section", id: "tb-sched" });

  if (!sc && (item.status === "done" || item.scheduledFrom)) {
    box.style.display = "none";
    box.dataset.key = "none";
    return box;
  }

  const isPlan = planItems(item, items).length > 0;
  const now = Date.now();
  const canEnable = !!(sc && !sc.enabled && validScheduleRule(sc.rule, now) && item.status !== "done");
  const isBusy = busyOps.has("sc:" + item.id);

  box.dataset.key = JSON.stringify([sc, item.status, ed.open, isBusy]);
  box.append(el("h3", { text: T.scTitle }));

  if (!sc && !ed.open) box.append(el("div", { class: "tb-hint", text: T.scNone }));
  if (sc) {
    box.append(el("div", { style: "font-size:13.5px" },
      el("strong", { text: ruleText(sc.rule) }),
      " · ",
      sc.enabled
        ? el("span", { text: sc.nextAt != null ? T.scNext(formatLocalMinute(sc.nextAt)) : T.scOn })
        : el("span", { style: "color:var(--warn)", text: T.scOff })));
    if (!sc.enabled) box.append(el("div", { class: "tb-hint", text: item.status === "done" ? T.scOffDone : canEnable ? T.scOffHint : T.scOffInvalid }));
    if (sc.lastRunAt != null) box.append(el("div", { class: "tb-hint", text: T.scLastRun(formatLocalMinute(sc.lastRunAt)) }));
    if (sc.lastSkip) {
      const rText = T.scSkipReasons[sc.lastSkip.reason] || sc.lastSkip.reason;
      const detail = sc.lastSkip.detail ? ` (${errText(sc.lastSkip.detail)})` : "";
      box.append(el("div", { class: "tb-hint", text: T.scLastSkip(formatLocalMinute(sc.lastSkip.at), rText + detail) }));
    }
  }

  const recurring = (ed.open ? ed.draft.kind : sc && sc.rule && sc.rule.kind) !== "once" && (ed.open || !!sc);
  if (ed.open || sc) {
    box.append(el("div", { class: "tb-hint", text: recurring ? (isPlan ? T.scHintRecPlan : T.scHintRec) : (isPlan ? T.scHintOncePlan : T.scHintOnce) }));
  }

  const scPost = (body, msg) => async () => {
    try {
      await runTaskApi("sc:" + item.id, () => api(`/tasks/${encodeURIComponent(item.id)}/schedule`, { method: "POST", body }), msg || S.saved);
      schedEditors.delete(item.id);
      refreshListOnly();
      const fresh = itemById(item.id);
      if (fresh) {
        const cur = document.getElementById("tb-sched"); if (cur) cur.replaceWith(renderScheduleSection(fresh));
        const sa = document.getElementById("tb-sendact"); if (sa) sa.replaceWith(renderSendActions(fresh));
      }
    } catch (_) {}
  };

  if (ed.open) {
    const redraw = () => { const cur = document.getElementById("tb-sched"); if (cur) cur.replaceWith(renderScheduleSection(item)); };
    const upd = (p) => { ed.draft = { ...ed.draft, ...p }; schedEditors.set(item.id, ed); redraw(); };
    const rule = ruleOfDraft(ed.draft);
    const valid = validScheduleRule(rule, Date.now());
    let invalidHint = "";
    if (!valid) {
      if (ed.draft.kind === "once") invalidHint = T.scInvalidPast;
      else if (ed.draft.kind === "interval") invalidHint = T.scInvalidInterval;
      else if (ed.draft.kind === "weekly" && !ed.draft.days.length) invalidHint = T.scInvalidDays;
      else invalidHint = T.scInvalidTime;
    }

    box.append(el("label", { class: "f", text: T.scKind }),
      el("select", { onchange: (e) => upd({ kind: e.target.value }) },
        ...["once", "interval", "daily", "weekly"].map((k) => el("option", { value: k, text: T.scKinds[k], selected: ed.draft.kind === k }))));

    if (ed.draft.kind === "once") {
      box.append(el("label", { class: "f", text: T.scWhen }),
        el("input", { type: "datetime-local", value: `${ed.draft.date}T${ed.draft.time}`, onchange: (e) => {
          const [date, time = ""] = e.target.value.split("T");
          upd({ date, time: time.slice(0, 5) });
        } }));
    }
    if (ed.draft.kind === "interval") {
      box.append(el("div", { class: "tb-sched-row", style: "margin-top:8px" },
        el("label", null, el("span", { class: "sub", text: T.scEvery }),
          el("input", { type: "number", min: "1", step: "1", value: Number.isFinite(ed.draft.amount) ? String(ed.draft.amount) : "", onchange: (e) => upd({ amount: Number(e.target.value) }) })),
        el("select", { onchange: (e) => upd({ unit: e.target.value }) },
          ...["m", "h", "d"].map((u) => el("option", { value: u, text: T.scUnits[u], selected: ed.draft.unit === u })))));
    }
    if (ed.draft.kind === "weekly") {
      box.append(el("label", { class: "f", text: T.scDays }),
        el("div", { class: "tb-days" },
          ...DAY_ORDER.map((day) => {
            const on = ed.draft.days.includes(day);
            return el("label", { class: "tb-day" + (on ? " on" : "") },
              el("input", { type: "checkbox", checked: on, onchange: (e) => upd({ days: e.target.checked ? [...ed.draft.days, day] : ed.draft.days.filter((x) => x !== day) }) }),
              weekdayName(day));
          })));
    }
    if (ed.draft.kind === "daily" || ed.draft.kind === "weekly") {
      box.append(el("label", { class: "f", text: T.scTime }),
        el("input", { type: "time", value: ed.draft.time, onchange: (e) => upd({ time: e.target.value }) }));
    }
    if (invalidHint) box.append(el("div", { class: "tb-hint err", text: invalidHint }));
    box.append(el("div", { class: "btns" },
      el("button", { class: "b", type: "button", text: T.scCancel, onclick: () => { schedEditors.delete(item.id); redraw(); } }),
      el("span", { class: "tb-spacer" }),
      el("button", { class: "b primary", type: "button", disabled: !valid || isBusy, text: T.scSaveOn, onclick: scPost({ action: "set", rule }) })));
  } else {
    const btns = el("div", { class: "btns" });
    if (sc) btns.append(el("button", { class: "b", type: "button", disabled: isBusy, text: T.scRemove, onclick: scPost({ action: "remove" }) }));
    btns.append(el("span", { class: "tb-spacer" }));
    if (item.status !== "done") {
      btns.append(el("button", { class: "b", type: "button", disabled: isBusy, text: sc ? T.scChange : T.scAdd, onclick: () => {
        schedEditors.set(item.id, { open: true, draft: makeSchedDraft(sc && sc.rule) });
        const cur = document.getElementById("tb-sched");
        if (cur) cur.replaceWith(renderScheduleSection(item));
      } }));
    }
    if (sc && sc.enabled) btns.append(el("button", { class: "b", type: "button", disabled: isBusy, text: T.scTurnOff, onclick: scPost({ action: "disable" }) }));
    if (canEnable) btns.append(el("button", { class: "b primary", type: "button", disabled: isBusy, text: T.scTurnOn, onclick: scPost({ action: "enable" }) }));
    box.append(btns);
  }
  return box;
}

// ── 보내기·완료·삭제·순서변경 하단 액션 줄 ──
function siblingsForReorder(item) {
  const items = allItems();
  const cmap = charsMap();
  const gk = assigneeKey(item, cmap);
  return items.filter((i) => i.status === item.status && assigneeKey(i, cmap) === gk)
    .sort((a, b) => (a.order || 0) - (b.order || 0));
}

function renderSendActions(item) {
  const items = allItems();
  const cmap = charsMap();
  const patch = edits.get(item.id) || {};
  const value = { ...item, ...patch };
  const dirty = hasDraftChanges(item, patch);
  const blocked = sendBlockedReason(value, item, items, cmap);
  const targetChar = value.assigneeAgentId ? cmap.get(value.assigneeAgentId) : null;
  const restart = !blocked && (needRestart.has(item.id) || busyShell(targetChar));
  const what = targetChar && targetChar.cli ? S.cliRunning(CLI_NAMES[targetChar.cli] || targetChar.cli) : busyShell(targetChar) ? S.cliBusy : S.restartNeeded;
  const sending = busyOps.has("send:" + item.id);
  const canSend = !sending && !blocked && hasWorkContent(value) && validWorkLimits(value);
  const sibs = siblingsForReorder(item);
  const idx = sibs.findIndex((s) => s.id === item.id);
  const prevSib = idx > 0 ? sibs[idx - 1] : null;
  const nextSib = idx >= 0 && idx < sibs.length - 1 ? sibs[idx + 1] : null;

  const box = el("div", { id: "tb-sendact", "data-key": JSON.stringify([item.status, dirty, blocked, restart, what, sending, canSend, prevSib && prevSib.id, nextSib && nextSib.id]) });

  const doSend = async () => {
    if (!canSend) return;
    try {
      if (dirty) await saveEdit(item.id);
      await runTaskApi("send:" + item.id, () => api(`/tasks/${encodeURIComponent(item.id)}/send`, { method: "POST", body: { restart } }), S.done);
      needRestart.delete(item.id);
    } catch (e) {
      if (e && e.code === "restart-required") {
        needRestart.add(item.id);
        const cur = document.getElementById("tb-sendact");
        if (cur) cur.replaceWith(renderSendActions(item));
      }
    }
  };

  if (restart) box.append(el("div", { class: "warn", text: "⚠ " + S.restartWarn(what) }));

  const row = el("div", { class: "tb-actions" },
    confirmBtn(T.deleteBtn, async () => {
      try {
        await runTaskApi("del:" + item.id, () => api(`/tasks/${encodeURIComponent(item.id)}/delete`, { method: "POST", body: {} }), S.deleted);
        edits.delete(item.id);
        schedEditors.delete(item.id);
        if (view.task === item.id) view.task = null;
        D.render();
      } catch (_) {}
    }, "b danger"),
    prevSib && el("button", { class: "b", type: "button", text: T.moveUp, onclick: () => reorderCard(item.id, prevSib.id, false) }),
    nextSib && el("button", { class: "b", type: "button", text: T.moveDown, onclick: () => reorderCard(item.id, nextSib.id, true) }),
    el("span", { class: "tb-spacer" }),
    item.status !== "done" && el("button", { class: "b", type: "button", disabled: sending, text: T.markDone, onclick: async () => {
      try {
        await runTaskApi("done:" + item.id, () => api(`/tasks/${encodeURIComponent(item.id)}/done`, { method: "POST", body: {} }), S.markedDone);
        const p = edits.get(item.id);
        if (p) { delete p.status; if (!Object.keys(p).length) edits.delete(item.id); }
        refreshListOnly();
        const fresh = itemById(item.id);
        if (fresh) { const cur = document.getElementById("tb-detail"); if (cur) cur.replaceWith(renderDetailPanel(fresh)); }
      } catch (_) {}
    } }),
    restart
      ? confirmBtn(T.restartSend, doSend, "b danger")
      : el("button", { class: "b primary", type: "button", disabled: !canSend, text: sending ? T.sending : dirty ? T.saveAndSend : T.send, onclick: doSend }));

  box.append(row);
  if (blocked) box.append(el("div", { class: "tb-hint err", text: errText(blocked) }));
  return box;
}

// ── 편집 저장 ──
async function saveEdit(itemId) {
  const item = itemById(itemId);
  const raw = edits.get(itemId);
  if (!item || !raw || !hasDraftChanges(item, raw)) return;
  const patch = {};
  for (const [k, v] of Object.entries(raw)) {
    if (k === "assigneeAgentId") patch.assigneeAgentId = v || null;
    else if (k === "agent") patch.agent = v && v.cli ? v : null;
    else if (k === "dependsOn") patch.dependsOn = v && v.length ? v : null;
    else if (k === "maxFollowUps" || k === "permissionWaitMinutes") patch[k] = v == null || v === "" ? null : v;
    else patch[k] = v;
  }
  await runTaskApi("edit:" + itemId, () => api(`/tasks/${encodeURIComponent(itemId)}/edit`, { method: "POST", body: patch }), S.saved);
  edits.delete(itemId);
  needRestart.delete(itemId);
}

// ── 상세/새 업무 패널 ──
function renderAttention(item) {
  const box = el("div", { id: "tb-att" });
  const waiting = itemNeedsHuman(item);
  const c = item.assigneeAgentId ? D.charOf(item.assigneeAgentId) : null;
  const msg = waiting && c ? waitingMessage(c) : null;
  box.dataset.key = JSON.stringify([waiting, c && c.agentId, msg]);
  if (!waiting || !c) return box;
  box.append(el("div", { class: "tb-att" },
    el("div", { class: "grow", text: "⚠ " + T.attentionBanner(c.name, msg) }),
    el("button", { class: "b warn-btn", type: "button", text: T.openChatAnswer, onclick: () => D.openAgent(c.agentId) })));
  return box;
}

function renderDetailHeadChips(item) {
  const c = item.assigneeAgentId ? D.charOf(item.assigneeAgentId) : null;
  const waiting = itemNeedsHuman(item);
  const box = el("div", { id: "tb-head-chips", style: "display:inline-flex;gap:6px;align-items:center;flex-wrap:wrap", "data-key": JSON.stringify([item.status, c && c.agentId, waiting]) },
    el("span", { class: "tb-pill st-" + item.status, text: T.status[item.status] || item.status }),
    c && el("button", { class: "b" + (waiting ? " warn-btn" : ""), type: "button", style: "padding:3px 9px;font-size:12px", text: T.openChat, onclick: () => D.openAgent(c.agentId) }));
  return box;
}

function renderCreatePanel() {
  const panel = el("div", { class: "tb-detail", id: "tb-detail", "data-mode": "create" });
  if (!WIDE.matches) {
    panel.append(el("button", { class: "back", type: "button", text: S.back, onclick: () => { creating = false; D.render(); } }));
  }
  const d = createDraft || (createDraft = emptyCreateDraft());
  const dirty = hasWorkContent(d) || !!d.assigneeAgentId || !!d.agent || (d.dependsOn && d.dependsOn.length > 0) || !!d.plan || d.status !== "todo";
  const card = el("div", { class: "card" },
    el("div", { class: "tb-panel-head" },
      el("h2", { text: T.newWorkTitle }),
      WIDE.matches && el("button", { class: "b", type: "button", text: "×", onclick: () => { creating = false; D.render(); } })),
    renderEditorForm({
      creating: true,
      value: d,
      dirty,
      onChange: (p, structural) => {
        createDraft = { ...createDraft, ...p };
        if (structural) { const cur = document.getElementById("tb-detail"); if (cur) cur.replaceWith(renderCreatePanel()); }
        else updateSaveBar(null, createDraft, true);
      },
      onSave: async () => {
        const body = {
          title: createDraft.title || "",
          body: createDraft.body || "",
          status: createDraft.status || "todo",
          assigneeAgentId: createDraft.assigneeAgentId || null,
          agent: createDraft.agent && createDraft.agent.cli ? createDraft.agent : null,
          dependsOn: createDraft.dependsOn && createDraft.dependsOn.length ? createDraft.dependsOn : null,
          plan: !!createDraft.plan,
          maxFollowUps: createDraft.maxFollowUps != null ? createDraft.maxFollowUps : null,
          permissionWaitMinutes: createDraft.permissionWaitMinutes != null ? createDraft.permissionWaitMinutes : null,
          send: false,
        };
        try {
          const res = await runTaskApi("create", () => api("/tasks", { method: "POST", body }), S.saved);
          createDraft = null;
          creating = false;
          view.task = (res && res.itemId) || null;
          D.render();
        } catch (_) {}
      },
      onDiscard: () => { createDraft = null; creating = false; D.render(); },
    }));
  panel.append(card);
  return panel;
}

function renderDetailPanel(item) {
  const panel = el("div", { class: "tb-detail", id: "tb-detail", "data-mode": "item", "data-id": item.id, "data-rev": String(item.updatedAt || 0) });
  if (!WIDE.matches) {
    panel.append(el("button", { class: "back", type: "button", text: S.back, onclick: () => { view.task = null; D.render(); } }));
  }
  const patch = edits.get(item.id) || {};
  const value = { ...item, ...patch };
  const dirty = hasDraftChanges(item, patch);

  const idBar = el("span", { class: "tb-idbar" },
    el("code", { text: item.id }),
    el("button", { class: "b", type: "button", title: T.copyId, text: "⧉", onclick: () => copyText(item.id) }));

  const lastErr = el("div", { id: "tb-lasterr", "data-key": item.lastError || "" },
    item.lastError && el("div", { class: "warn", role: "alert", text: "⚠ " + errText(item.lastError) }));

  const card = el("div", { class: "card" },
    renderAttention(item),
    el("div", { class: "tb-panel-head" },
      el("h2", { text: workItemTitle(item) }),
      idBar,
      renderDetailHeadChips(item),
      WIDE.matches && el("button", { class: "b", type: "button", text: "×", onclick: () => { view.task = null; D.render(); } })),
    renderEditorForm({
      itemId: item.id,
      value,
      creating: false,
      dirty,
      onChange: (p, structural) => {
        const next = { ...(edits.get(item.id) || {}), ...p };
        edits.set(item.id, next);
        const merged = { ...item, ...next };
        const isDirty = hasDraftChanges(item, next);
        if (structural) {
          const cur = document.getElementById("tb-detail");
          if (cur) cur.replaceWith(renderDetailPanel(item));
        } else {
          updateSaveBar(item.id, merged, isDirty);
          const sa = document.getElementById("tb-sendact");
          if (sa) sa.replaceWith(renderSendActions(item));
        }
      },
      onSave: async () => {
        try {
          await saveEdit(item.id);
          refreshListOnly();
          const fresh = itemById(item.id);
          if (fresh) { const cur = document.getElementById("tb-detail"); if (cur) cur.replaceWith(renderDetailPanel(fresh)); }
        } catch (_) {}
      },
      onDiscard: () => {
        edits.delete(item.id);
        const cur = document.getElementById("tb-detail");
        if (cur) cur.replaceWith(renderDetailPanel(item));
      },
    }),
    lastErr,
    renderAutoRunSection(item),
    renderScheduleSection(item),
    renderSendActions(item));

  panel.append(card);
  return panel;
}

function renderTasks(main) {
  const td = D.getTasks();
  const chars = D.getChars();
  if (!td || !chars) {
    main.replaceChildren(el("div", { class: "empty", text: "…" }));
    return;
  }
  const selected = view.task ? itemById(view.task) : null;
  if (view.task && !selected) view.task = null;

  if (!WIDE.matches) {
    if (creating) { main.replaceChildren(renderCreatePanel()); return; }
    if (selected) { main.replaceChildren(renderDetailPanel(selected)); return; }
    main.replaceChildren(renderBoardList());
    lastListKey = listFingerprint();
    return;
  }

  const list = renderBoardList();
  lastListKey = listFingerprint();
  const detail = creating
    ? renderCreatePanel()
    : selected
      ? renderDetailPanel(selected)
      : el("div", { class: "tb-detail", id: "tb-detail", "data-mode": "empty" },
          el("div", { class: "card tb-empty-detail", text: T.pickHint }));
  main.replaceChildren(el("div", { class: "tb-layout" }, list, detail));
}

function onTick(stateChanged, tasksChanged) {
  if (!stateChanged && !tasksChanged) return;
  const listEl = document.getElementById("tb-list");
  const detailEl = document.getElementById("tb-detail");
  if (!listEl && !detailEl) { D.render(); return; }

  // 좌측 목록: 내용이 바뀐 경우에만 갱신
  if (listEl) {
    const nextKey = listFingerprint();
    if (nextKey !== lastListKey) refreshListOnly();
  }

  // 우측(또는 모바일) 상세: 입력 폼은 건드리지 않고 동적 섹션만 data-key로 부분 갱신
  if (detailEl && detailEl.dataset.mode === "item") {
    const id = detailEl.dataset.id;
    const item = itemById(id);
    if (!item) {
      view.task = null;
      D.render();
      return;
    }
    const focusedInside = detailEl.contains(document.activeElement) && /^(INPUT|TEXTAREA|SELECT)$/.test(document.activeElement.tagName);
    const hasEdits = hasDraftChanges(item, edits.get(id));
    if (!hasEdits && !focusedInside && !(schedEditors.get(id) && schedEditors.get(id).open) && detailEl.dataset.rev !== String(item.updatedAt || 0)) {
      detailEl.replaceWith(renderDetailPanel(item));
      return;
    }
    const swap = (elId, make) => {
      const cur = document.getElementById(elId);
      if (!cur) return;
      const next = make();
      if (next.dataset.key !== cur.dataset.key) cur.replaceWith(next);
    };
    swap("tb-att", () => renderAttention(item));
    swap("tb-head-chips", () => renderDetailHeadChips(item));
    swap("tb-lasterr", () => el("div", { id: "tb-lasterr", "data-key": item.lastError || "" },
      item.lastError && el("div", { class: "warn", role: "alert", text: "⚠ " + errText(item.lastError) })));
    swap("tb-autorun", () => renderAutoRunSection(item));
    if (!(schedEditors.get(id) && schedEditors.get(id).open)) {
      swap("tb-sched", () => renderScheduleSection(item));
    }
    swap("tb-sendact", () => renderSendActions(item));
  }
}

D.tasks = {
  get allDone() { return allDone; },
  onSelect(id) { creating = false; view.task = id; },
  render: renderTasks,
  onTick,
};
})();
