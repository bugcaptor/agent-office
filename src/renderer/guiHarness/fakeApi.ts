// src/renderer/guiHarness/fakeApi.ts
//
// 헤드리스 GUI 하네스의 가짜 백엔드(docs/gui-harness-design.md). 하네스 서버의
// Vite 플러그인이 `ipc/tauriApi.ts` import를 이 모듈로 바꿔 끼우므로 같은 이름
// `tauriApi`를 내보낸다. 구현한 메서드는 `AgentOfficeApi` 시그니처로 타입 검사를
// 받고, 구현하지 않은 메서드는 reject하면서 `log.unfaked`에 남긴다 — 화면이
// 그 호출에 기대면 하네스 출력에서 바로 보인다.
//
// 이벤트(`on*`)는 실제 `listen()`을 그대로 써서, 가짜 백엔드가 내는 이벤트와
// `@tauri-apps/api/event`를 직접 구독하는 모듈이 같은 경로로 받는다.
import { listen } from "@tauri-apps/api/event";
import { Events } from "@shared/ipc";
import type {
  AgentOfficeApi,
  AppSettings,
  MemoSheet,
  NotificationEvent,
  RepositoryAuditItem,
  SessionState,
  SessionStateEvent,
  TaskChatRequest,
} from "@shared/types";
import memoFixture from "@shared/contract-fixtures/memo-sheet.current.json";
import { emitTauriEvent } from "./tauriStub";
import { loadScenario } from "./scenarios";

const harness = window.__guiHarness!;
const params = new URLSearchParams(location.search);
const scenario = loadScenario(String(harness.scenario), params.get("lang") ?? "ko");
let settings: AppSettings = scenario.settings.settings;
let state = scenario.state;
let workItems = scenario.workItems;
const auditRepo = (name: string, extra: Partial<RepositoryAuditItem>): RepositoryAuditItem => ({
  path: `/work/${name}`, name, lastWorkedAt: 0, branch: "main", upstream: "origin/main", hasRemote: true,
  ahead: 0, behind: 0, changedCount: 0, conflictCount: 0, timedOut: false, unavailable: false, ...extra,
});
let auditRepos = [
  auditRepo("agent-office", { ahead: 3, changedCount: 2 }),
  auditRepo("diverged", { ahead: 1, behind: 2 }),
  auditRepo("tidy", {}),
];

const outputSubs = new Map<string, Set<(data: string, bytes: number) => void>>();
// 백엔드처럼 세션 출력 백로그를 쥐고 있다가 새 구독에 재생한다.
const backlog = new Map(Object.entries(scenario.output));
const sessions = new Map(scenario.adopted.map((s) => [s.agentId, s.sessionId]));
let sessionSeq = 100;
// Markdown 편집기(문서 뷰어 버튼 확인용). 어떤 경로든 같은 예시 문서를 돌려준다.
const markdownSample = "# Doc viewer\n\n- [x] GFM table\n- [ ] Mermaid\n\n```mermaid\ngraph LR; A-->B\n```\n";
/** `docViewOpen`이 받은 `root/relPath`(스크립트가 읽어 확인한다). */
const docViewOpened: string[] = [];
/** `openTaskChat`이 받은 요청(스크립트가 읽어 확인한다). */
const taskChatRequests: TaskChatRequest[] = [];

const byteLength = (data: string) => new TextEncoder().encode(data).length;

function pushOutput(agentId: string, data: string): void {
  backlog.set(agentId, (backlog.get(agentId) ?? "") + data);
  for (const cb of outputSubs.get(agentId) ?? []) cb(data, byteLength(data));
}

function emitSessionState(agentId: string, next: SessionState, exitCode = 0): void {
  const sessionId = sessions.get(agentId) ?? `s${sessionSeq++}`;
  sessions.set(agentId, sessionId);
  const event: SessionStateEvent = { sessionId, agentId, state: next, at: Date.now() };
  if (next === "exited") event.exit = { sessionId, exitCode, intentional: false };
  emitTauriEvent(Events.sessionState, event);
}

function notify(agentId: string, message = "needs input"): void {
  const sessionId = sessions.get(agentId) ?? "s0";
  const n: NotificationEvent = {
    id: `n${Date.now()}`,
    sessionId,
    agentId,
    source: "hook",
    message,
    dedupKey: `hook:${sessionId}:${Date.now()}`,
    at: Date.now(),
  };
  emitTauriEvent(Events.notificationNew, n);
}

function subscribe<T>(event: string, cb: (payload: T) => void): () => void {
  let un: (() => void) | null = null;
  let disposed = false;
  void listen<T>(event, (e) => cb(e.payload)).then((f) => {
    if (disposed) f();
    else un = f;
  });
  return () => {
    disposed = true;
    un?.();
  };
}

const memo = (): MemoSheet => ({ ...memoFixture, content: "" });

const impl: Partial<AgentOfficeApi> = {
  // ── 부팅 ──
  async loadState() { return state; },
  async saveState(next) { state = next; },
  async getAppSettings() { return { firstRun: scenario.settings.firstRun, settings }; },
  async setAppSettings(patch) {
    settings = { ...settings, ...patch };
    emitTauriEvent(Events.appSettingsChanged, settings);
  },
  async adoptDetachedSessions() {
    // 브리지가 붙은 뒤 불리므로 알림은 여기서 흘린다.
    setTimeout(() => scenario.notifications.forEach((n) => emitTauriEvent(Events.notificationNew, n)), 0);
    return scenario.adopted;
  },
  async handoffSupported() { return false; },
  async listAvailableShells() { return []; },
  async loadUsageSnapshot() { return scenario.usage; },
  async loadAwards() { return scenario.awards; },
  async finalizeAward() { return scenario.awards; },
  async loadAwardPortrait() { return null; },
  async loadSessionTurns() { return []; },
  async loadSessionEvents() { return []; },
  async loadWorkItems() { return workItems; },
  async saveWorkItems(next) { workItems = next; },
  async loadSessionSummaries() { return []; },
  async appendSessionSummaries() {},
  async loadWorkLogs() { return {}; },
  async saveWorkLog() {},
  async scanWorkInbox() { return []; },
  async repositoryAuditList() { return auditRepos; },
  async repositoryAuditPush(path) {
    // 푸시 버튼 흐름을 화면에서 확인할 수 있게 잠깐 기다렸다가 성공시킨다.
    await new Promise((r) => setTimeout(r, 400));
    auditRepos = auditRepos.map((r) => (r.path === path ? { ...r, ahead: 0 } : r));
    return auditRepos.find((r) => r.path === path)!;
  },
  async listClaudeResumeSessions() { return {}; },
  async setKeepAwake() {},
  async setMascotVisible() {},
  async workdirGitBranch() { return { isRepo: false, branch: null }; },
  async markdownListFiles() { return { files: [{ relPath: "docs/plan.md", name: "plan.md" }], truncated: false }; },
  async markdownReadFile() { return { content: markdownSample, version: "v1" }; },
  async markdownWriteFile() { return { version: "v2" }; },
  async docViewOpen(root, relPath) { docViewOpened.push(`${root}/${relPath}`); },

  // ── 캐릭터 그림 ── (없으면 시드로 절차 생성)
  async loadPortrait() { return null; },
  async loadSprite() { return null; },
  async loadMinimi() { return null; },
  async savePortrait() {},
  async saveSprite() {},
  async saveMinimi() {},
  async deletePortrait() {},
  async deleteSprite() {},
  async deleteMinimi() {},

  // ── 세션 ──
  async createSession(agentId) {
    const sessionId = `s${sessionSeq++}`;
    sessions.set(agentId, sessionId);
    setTimeout(() => {
      emitSessionState(agentId, "running");
      pushOutput(agentId, "\x1b[1;32m$\x1b[0m ");
    }, 0);
    return { sessionId, state: "starting" };
  },
  // 태스크 대화(docs/task-chat-design.md): 세션을 띄우고 첫 프롬프트를 넣은 것처럼 맥락 요약을 찍는다.
  async openTaskChat(request) {
    const agentId = `task-chat-${request.taskId}`;
    const sessionId = `s${sessionSeq++}`;
    sessions.set(agentId, sessionId);
    taskChatRequests.push(request);
    emitSessionState(agentId, "running");
    pushOutput(agentId, `\x1b[1;36m> ${request.context.task.title}\x1b[0m\r\n${request.cwd ?? "~"} · ${request.agent.cli}\r\n`);
    return { agentId, sessionId };
  },
  async disposeSession(agentId) {
    emitSessionState(agentId, "exited");
    sessions.delete(agentId);
  },
  writeInput(agentId, data, source = "human") {
    // 사람이 친 글자만 메아리친다. 엔터는 줄바꿈 + 새 프롬프트.
    if (source === "human") pushOutput(agentId, data.replace(/\r/g, "\r\n\x1b[1;32m$\x1b[0m "));
  },
  resize() {},
  onData(agentId, cb) {
    const subs = outputSubs.get(agentId) ?? new Set();
    outputSubs.set(agentId, subs);
    subs.add(cb);
    const replay = backlog.get(agentId);
    if (replay) setTimeout(() => subs.has(cb) && cb(replay, byteLength(replay)), 0);
    return () => { subs.delete(cb); };
  },
  onSessionState: (cb) => subscribe(Events.sessionState, cb),
  onNotification: (cb) => subscribe(Events.notificationNew, cb),
  onNotificationCleared: (cb) => subscribe(Events.notificationCleared, cb),
  onActivity: (cb) => subscribe(Events.activityEvent, cb),
  onTurnUsage: (cb) => subscribe(Events.turnUsage, cb),
  onAppSettingsChanged: (cb) => subscribe(Events.appSettingsChanged, cb),
  appendSessionTurn() {},
  async listNotifications(agentId) { return scenario.notifications.filter((n) => n.agentId === agentId); },
  clearNotifications() {},
  setBadgeCount() {},
  async listSessionLogs() { return { total: 0, items: [] }; },

  // ── 패널·다이얼로그 ──
  async loadDiary() { return []; },
  async loadMemo() { return memo(); },
  async saveMemo() {},
  async listMemoArchive() { return []; },
  async archiveMemoSheet() { return memo(); },
  async deleteMemos() {},
  async apiKeyStatus() {
    return { anthropicSet: false, anthropicFromEnv: false, openrouterSet: false, openrouterFromEnv: false };
  },
  async controlStatus() {
    return { enabled: false, running: false, approved: false, port: null, appDataDir: "/tmp/gui-harness" };
  },
  async codexImageStatus() { return { available: false }; },
  async listProviderModels() { return []; },
  async workAgentModels() { return []; },
  async runRecipeStatus() { return null; },
};

export const tauriApi = new Proxy(impl, {
  get(target, key) {
    if (key in target) return target[key as keyof AgentOfficeApi];
    return async () => {
      harness.log.unfaked.push(String(key));
      throw new Error(`gui-harness: unfaked api ${String(key)}`);
    };
  },
}) as AgentOfficeApi;

Object.assign(harness, {
  pushOutput,
  emitSessionState,
  notify,
  /** 스크립트에서 스토어를 직접 조작할 때(예: `openTerminal`). */
  store: async () => (await import("../store/appStore")).useAppStore,
  /** Markdown 편집기 스토어(예: `openFile`). */
  markdownStore: async () => (await import("../markdown/markdownStore")).useMarkdownStore,
  docViewOpened,
  /** 태스크 대화 스토어(예: `openTaskChat`). */
  taskChatStore: async () => await import("../workpanels/taskChatStore"),
  taskChatRequests,
});
