// 태스크 대화(docs/task-chat-design.md). 캐릭터 없는 앱 안 터미널로 태스크 하나를 주제로 묻고 답한다.
// 세션 id는 `task-chat-<taskId>`이고, 탭·상태는 여기만 쥔다(appStore.sessions·recentAgentIds에 넣지 않는다).
import { create } from "zustand";
import { TASK_CHAT_PREFIX, isTaskChatId } from "@shared/types";
import type { SessionStateEvent, SessionStatus, TaskChatContext, TaskChatRequest, WorkItem } from "@shared/types";
import { t } from "../i18n";
import { tauriApi } from "../ipc/tauriApi";
import { useAppStore } from "../store/appStore";
import { normalizeWorkAgent } from "../profile/generate";
import { terminalRegistry } from "../terminal/TerminalRegistry";
import { useWorkItemStore } from "./workItemStore";
import { useSessionSummaryStore } from "./sessionSummaryStore";
import { workItemTitle } from "./workItemDraft";
import { planItems } from "./autoRun";
import { sendErrorText } from "./sendErrorText";

/** 동시에 살아 있는 대화 세션 상한(PTY·스레드 보호). 네이티브 `MAX_TASK_CHATS`와 같다. */
export const MAX_TASK_CHATS = 4;
const MAX_NOTES = 20;

export interface TaskChat {
  agentId: string;
  taskId: string;
  /** 연 시점의 표시 제목. 태스크가 지워지면 탭 이름으로 쓴다. */
  title: string;
  cwd?: string;
  sessionId: string;
  status: SessionStatus;
  openedAt: number;
  /** 마지막으로 탭을 본 시각. 상한 안내에서 가장 오래 안 본 대화를 고른다. */
  viewedAt: number;
}
interface TaskChatState {
  chats: Record<string, TaskChat>;
  /** 탭 순서(연 순서). */
  order: string[];
  /** 여는 중인 태스크 id. */
  opening: Record<string, true>;
}
export const useTaskChatStore = create<TaskChatState>(() => ({ chats: {}, order: [], opening: {} }));

export { isTaskChatId };
export function taskChatId(taskId: string): string { return `${TASK_CHAT_PREFIX}${taskId}`; }
function isLive(status: SessionStatus | undefined): boolean { return status === "starting" || status === "running"; }
export function liveTaskChats(): TaskChat[] {
  const { chats, order } = useTaskChatStore.getState();
  return order.map((id) => chats[id]).filter((chat) => chat && isLive(chat.status));
}
export function liveTaskChatCount(): number { return liveTaskChats().length; }
/** 상한에 걸렸을 때 닫으라고 안내할 대화. */
export function stalestTaskChat(): TaskChat | undefined {
  return liveTaskChats().sort((a, b) => a.viewedAt - b.viewedAt)[0];
}

/** 열기를 막는 이유. 이미 살아 있는 대화가 있으면 그 탭으로 가므로 막지 않는다. */
export function taskChatBlockedReason(taskId: string): string | undefined {
  const { chats, opening } = useTaskChatStore.getState();
  if (opening[taskId]) return "task-chat-busy";
  if (isLive(chats[taskChatId(taskId)]?.status)) return undefined;
  if (liveTaskChatCount() >= MAX_TASK_CHATS) return "task-chat-limit";
  return undefined;
}

const ERROR_KEYS: Record<string, string> = {
  "task-chat-busy": "busy", "task-chat-running": "running", "task-chat-limit": "limit",
  "task-chat-invalid-task": "invalidTask", "task-chat-invalid-cwd": "invalidCwd",
};
/** 대화 열기 오류 문구. 대화 고유 코드가 아니면 보내기 오류 문구를 쓴다. */
export function taskChatErrorText(error: string): string {
  const key = ERROR_KEYS[error.split(":")[0]];
  if (key === "limit") {
    const stale = stalestTaskChat();
    return stale ? t("workpanels:taskChat.errors.limitStale", { max: MAX_TASK_CHATS, title: taskChatTitle(stale) }) : t("workpanels:taskChat.errors.limit", { max: MAX_TASK_CHATS });
  }
  return key ? t(`workpanels:taskChat.errors.${key}`) : sendErrorText(error);
}

/** 탭에 보일 제목. 태스크가 남아 있으면 지금 표시 제목을 쓴다. */
export function taskChatTitle(chat: Pick<TaskChat, "taskId" | "title">): string {
  const item = useWorkItemStore.getState().items.find((i) => i.id === chat.taskId);
  return item ? workItemTitle(item) : chat.title;
}

function isAbsolutePath(path: string | undefined): path is string {
  return !!path && (path.startsWith("/") || /^[A-Za-z]:[\\/]/.test(path) || path.startsWith("\\\\"));
}
/** 작업 폴더: 최신 연결 세션 요약의 cwd → 담당 프로필 cwd → 없음(네이티브가 홈으로 연다). */
export function resolveTaskChatCwd(item: WorkItem): string | undefined {
  const summaries = useSessionSummaryStore.getState().summaries;
  for (let n = item.sessionIds.length - 1; n >= 0; n--) {
    const cwd = summaries[item.sessionIds[n]]?.cwd;
    if (isAbsolutePath(cwd)) return cwd;
  }
  const cwd = item.assigneeAgentId ? useAppStore.getState().agents[item.assigneeAgentId]?.cwd : undefined;
  return isAbsolutePath(cwd) ? cwd : undefined;
}

/** 이 태스크가 속한 자동 진행의 루트. 지금 진행 중인 실행을 먼저, 그다음 최근에 시작한 것을 고른다. */
function planRoot(item: WorkItem, items: WorkItem[]): WorkItem | undefined {
  const roots = items.filter((root) => root.autoRun && (root.id === item.id || planItems(root, items).some((p) => p.id === item.id)));
  const rank = (root: WorkItem) => (root.autoRun!.currentId === item.id ? 2 : 0) + (root.autoRun!.state !== "finished" ? 1 : 0);
  return roots.sort((a, b) => rank(b) - rank(a) || b.autoRun!.startedAt - a.autoRun!.startedAt)[0];
}

/** 네이티브 맥락 묶음의 재료(태스크·플랜 스냅샷). 보고·세션 로그·Git 기록은 네이티브가 붙인다. */
export function buildTaskChatContext(item: WorkItem): TaskChatContext {
  const items = useWorkItemStore.getState().items;
  const { agents } = useAppStore.getState();
  const summaries = useSessionSummaryStore.getState().summaries;
  const byId = new Map(items.map((i) => [i.id, i]));
  const link = (i: WorkItem) => ({ id: i.id, title: workItemTitle(i), status: i.status });
  const dependsOn = (item.dependsOn ?? []).map((id) => byId.get(id)).filter((i): i is WorkItem => !!i).map(link);
  const dependents = items.filter((i) => i.dependsOn?.includes(item.id)).map(link);
  const sessions = item.sessionIds.flatMap((sessionId) => {
    const summary = summaries[sessionId];
    const agentId = summary?.agentId ?? item.assigneeAgentId;
    return agentId ? [{ sessionId, agentId, ...(summary ? { sentAt: summary.startedAt } : {}) }] : [];
  });
  const assignee = item.assigneeAgentId ? agents[item.assigneeAgentId]?.name : undefined;
  const context: TaskChatContext = {
    task: {
      title: workItemTitle(item), status: item.status, body: item.body,
      ...(assignee ? { assignee } : {}),
      ...(dependsOn.length ? { dependsOn } : {}),
      ...(dependents.length ? { dependents } : {}),
      ...(sessions.length ? { sessions } : {}),
    },
  };
  const root = planRoot(item, items);
  const run = root?.autoRun;
  if (root && run) {
    const log = run.log.filter((entry) => entry.taskId === item.id).map((entry) => ({ at: entry.at, kind: entry.kind, ...(entry.text ? { text: entry.text } : {}) }));
    // 앞 태스크의 인계 메모는 다음 태스크마다 넘어간다(workerContext). 이 태스크를 처음 보낸 뒤 것은 넘어오지 않았다.
    const firstSend = run.log.find((entry) => entry.kind === "send" && entry.taskId === item.id)?.at;
    const handoffNotes = run.log
      .filter((entry) => entry.kind === "next" && entry.taskId && entry.taskId !== item.id && entry.text?.trim() && (firstSend === undefined || entry.at <= firstSend))
      .slice(-MAX_NOTES)
      .map((entry) => ({ taskId: entry.taskId!, title: byId.has(entry.taskId!) ? workItemTitle(byId.get(entry.taskId!)!) : "?", text: entry.text!.trim() }));
    context.plan = {
      id: root.id, title: workItemTitle(root), body: root.body, state: run.state,
      ...(run.pauseReason ? { pauseReason: run.pauseReason } : {}),
      ...(run.pauseDetail ? { pauseDetail: run.pauseDetail } : {}),
      ...(log.length ? { log } : {}),
      ...(handoffNotes.length ? { handoffNotes } : {}),
    };
  }
  return context;
}

/** 새 세션의 첫 크기. 지금 보이는(또는 최근) 캐릭터 터미널 크기를 빌린다. 탭을 열면 다시 맞춘다. */
function initialSize(): { cols?: number; rows?: number } {
  const { activeTerminalAgentId, recentAgentIds, sessions } = useAppStore.getState();
  const chat = activeTerminalAgentId ? useTaskChatStore.getState().chats[activeTerminalAgentId] : undefined;
  const source = chat ? undefined : sessions[activeTerminalAgentId ?? recentAgentIds[0] ?? ""];
  const fitted = activeTerminalAgentId && terminalRegistry.get(activeTerminalAgentId)?.term;
  if (fitted && fitted.cols > 0 && fitted.rows > 0) return { cols: fitted.cols, rows: fitted.rows };
  return source ? { cols: source.cols, rows: source.rows } : {};
}

function forget(agentId: string): void {
  terminalRegistry.destroy(agentId);
  latest.delete(agentId);
  useTaskChatStore.setState((s) => {
    const chats = { ...s.chats };
    delete chats[agentId];
    return { chats, order: s.order.filter((id) => id !== agentId) };
  });
}

/**
 * 태스크 대화를 연다. 이미 살아 있으면 그 탭으로 간다. 실패하면 오류 코드를 돌려준다(탭은 생기지 않는다).
 * 업무의 sessionIds·상태와 자동 진행은 건드리지 않는다.
 */
export async function openTaskChat(taskId: string): Promise<string | undefined> {
  const agentId = taskChatId(taskId);
  const existing = useTaskChatStore.getState().chats[agentId];
  if (existing && isLive(existing.status)) {
    useAppStore.getState().openTerminal(agentId);
    return undefined;
  }
  const blocked = taskChatBlockedReason(taskId);
  if (blocked) return blocked;
  const item = useWorkItemStore.getState().items.find((i) => i.id === taskId);
  if (!item) return "task-chat-invalid-task";
  const profile = item.assigneeAgentId ? useAppStore.getState().agents[item.assigneeAgentId] : undefined;
  const cwd = resolveTaskChatCwd(item);
  const request: TaskChatRequest = {
    taskId, agent: normalizeWorkAgent(item.agent ?? profile?.defaultAgent ?? { cli: "claude" }),
    ...(cwd ? { cwd } : {}),
    ...(profile?.shell ? { shell: profile.shell } : {}),
    ...initialSize(),
    context: buildTaskChatContext(item),
  };
  useTaskChatStore.setState((s) => ({ opening: { ...s.opening, [taskId]: true } }));
  // 끝난 대화를 다시 열면 새 화면에서 시작한다. 탭을 열기 전에 출력을 구독해 첫 화면을 놓치지 않는다.
  if (existing) forget(agentId);
  terminalRegistry.ensure(agentId);
  try {
    const result = await tauriApi.openTaskChat(request);
    const now = Date.now();
    const status = latest.get(agentId)?.sessionId === result.sessionId ? latest.get(agentId)!.status : "running";
    const chat: TaskChat = { agentId, taskId, title: request.context.task.title, ...(cwd ? { cwd } : {}), sessionId: result.sessionId, status, openedAt: now, viewedAt: now };
    useTaskChatStore.setState((s) => ({ chats: { ...s.chats, [agentId]: chat }, order: [...s.order.filter((id) => id !== agentId), agentId] }));
    useAppStore.getState().openTerminal(agentId);
    return undefined;
  } catch (error) {
    terminalRegistry.destroy(agentId);
    return error instanceof Error ? error.message : String(error);
  } finally {
    useTaskChatStore.setState((s) => { const opening = { ...s.opening }; delete opening[taskId]; return { opening }; });
  }
}

/** 대화 탭 닫기. CLI가 살아 있으면 확인창을 먼저 띄운다. */
export function requestCloseTaskChat(agentId: string): void {
  const chat = useTaskChatStore.getState().chats[agentId];
  if (!chat) return;
  if (isLive(chat.status)) useAppStore.getState().openModal({ kind: "confirm-close-task-chat", agentId });
  else void closeTaskChat(agentId);
}

/** 대화를 끝낸다. 세션을 버리고 탭을 없앤다. 보던 탭이면 이웃 대화 탭, 없으면 최근 캐릭터 탭으로 옮긴다. */
export async function closeTaskChat(agentId: string): Promise<void> {
  const { chats, order } = useTaskChatStore.getState();
  const chat = chats[agentId];
  if (!chat) return;
  const app = useAppStore.getState();
  if (app.activeTerminalAgentId === agentId) {
    const idx = order.indexOf(agentId);
    const next = order[idx + 1] ?? order[idx - 1] ?? app.recentAgentIds[0];
    if (next) app.openTerminal(next);
    else app.closeTerminal();
  }
  forget(agentId);
  if (isLive(chat.status)) {
    try { await tauriApi.disposeSession(agentId); } catch (err) { console.warn(`task-chat: dispose failed for ${agentId}`, err); }
  }
}

/** 마지막 세션 상태(탭이 생기기 전에 온 것도). 열기 결과의 sessionId와 맞으면 그 상태로 시작한다. */
const latest = new Map<string, { sessionId: string; status: SessionStatus }>();
export function applyTaskChatState(e: SessionStateEvent): void {
  const status: SessionStatus = e.state === "disposed" ? "exited" : e.state;
  latest.set(e.agentId, { sessionId: e.sessionId, status });
  useTaskChatStore.setState((s) => {
    const chat = s.chats[e.agentId];
    if (!chat || chat.sessionId !== e.sessionId || chat.status === status) return s;
    return { chats: { ...s.chats, [e.agentId]: { ...chat, status } } };
  });
}

/** 대화 세션 상태를 받고, 본 시각을 기록한다. 캐릭터 중심 스토어는 이 접두사를 무시한다(sessionBridge 등). */
export function installTaskChatTracking(): () => void {
  const offState = tauriApi.onSessionState((e) => { if (isTaskChatId(e.agentId)) applyTaskChatState(e); });
  const offActive = useAppStore.subscribe((s) => s.activeTerminalAgentId, (id) => {
    if (!id) return;
    useTaskChatStore.setState((s) => (s.chats[id] ? { chats: { ...s.chats, [id]: { ...s.chats[id], viewedAt: Date.now() } } } : s));
  });
  return () => { offState(); offActive(); };
}
