// src/renderer/chatView/chatViewModel.ts
//
// 터미널 탭 "대화 보기"의 순수 로직: 로그 이어 붙이기, 블록 묶기, 질문 답 엮기, 도구 입력 요약.
// 스토어·Tauri 의존이 없어 단위 테스트가 그대로 부른다.
import type { ChatLiveStatus, ChatLogItem, ChatLogPage, PendingAsk } from "@shared/types";

/** 화면이 들고 있는 최대 항목 수(오래된 것부터 버린다). */
export const CHAT_VIEW_KEEP = 2000;

/**
 * 새로 받은 쪽을 들고 있던 로그에 붙인다. epoch가 같으면 이어 붙이고, 다르면(처음·새 세션·
 * 이미지 설정 변경) 통째로 바꾼다.
 */
export function mergeChatPage(prev: ChatLogPage | null, next: ChatLogPage): ChatLogPage {
  if (!prev || prev.epoch !== next.epoch) return { ...next, items: next.items.slice(-CHAT_VIEW_KEEP) };
  if (!next.items.length && prev.attached === next.attached) return prev;
  const seen = prev.items.length ? prev.items[prev.items.length - 1].seq : 0;
  const items = [...prev.items, ...next.items.filter((i) => i.seq > seen)].slice(-CHAT_VIEW_KEEP);
  return { ...next, items };
}

/** 화면의 한 덩어리: 말 하나, 또는 도구 호출과 거기 딸린 결과·이미지. */
export interface ChatBlock {
  key: string;
  side: boolean;
  head: ChatLogItem;
  children: ChatLogItem[];
}

/**
 * 오래된 것부터 블록으로 묶는다. 도구 결과는 아직 결과를 못 받은 같은 쪽(본/서브에이전트)
 * 도구 호출에 호출 순서대로 붙고, 이미지는 바로 앞 블록(도구 결과나 함께 보낸 사람 말)에 붙는다.
 * 말이 끼면 그쪽의 결과 대기 짝은 끊는다(웹 상태판 `placeItem`과 같은 규칙, 방향만 반대).
 */
export function groupChatItems(items: ChatLogItem[]): ChatBlock[] {
  const blocks: ChatBlock[] = [];
  const pending: ChatBlock[] = [];
  for (const it of items) {
    const side = !!it.sidechain;
    const last = blocks[blocks.length - 1];
    if (it.kind === "tool_result") {
      const i = pending.findIndex((b) => b.side === side);
      const host = i >= 0 ? pending.splice(i, 1)[0] : last && last.side === side && last.head.kind === "tool_use" ? last : null;
      if (host) { host.children.push(it); continue; }
    }
    if (it.kind === "image" && last && last.side === side && (last.head.kind !== "text" || last.head.role === "user")) {
      last.children.push(it);
      continue;
    }
    const block: ChatBlock = { key: `${it.seq}`, side, head: it, children: [] };
    blocks.push(block);
    if (it.kind === "text") {
      for (let i = pending.length - 1; i >= 0; i--) if (pending[i].side === side) pending.splice(i, 1);
    } else if (it.kind === "tool_use" && it.toolName) {
      pending.push(block);
    }
  }
  return blocks;
}

export interface AskQuestion {
  question: string;
  header?: string;
  multiSelect: boolean;
  options: { label: string; description?: string }[];
}

/** AskUserQuestion 입력의 질문들(모양이 어긋난 항목은 건너뛴다). */
export function askQuestions(ask: PendingAsk): AskQuestion[] {
  const raw = (ask.toolInput as { questions?: unknown }).questions;
  if (!Array.isArray(raw)) return [];
  return raw.flatMap((q) => {
    if (!q || typeof q !== "object" || typeof (q as { question?: unknown }).question !== "string") return [];
    const r = q as { question: string; header?: unknown; multiSelect?: unknown; options?: unknown };
    const options = Array.isArray(r.options)
      ? r.options.flatMap((o) => (o && typeof o === "object" && typeof (o as { label?: unknown }).label === "string"
        ? [{ label: (o as { label: string }).label, description: typeof (o as { description?: unknown }).description === "string" ? (o as { description: string }).description : undefined }]
        : []))
      : [];
    return [{ question: r.question, header: typeof r.header === "string" ? r.header : undefined, multiSelect: r.multiSelect === true, options }];
  });
}

/** 고른 선택지와 직접 쓴 답을 질문마다 한 줄로 엮는다(여러 개는 ", "). 빈 답은 빈 문자열. */
export function joinAnswers(
  questions: AskQuestion[],
  picks: Record<string, string[]>,
  typed: Record<string, string>,
): Record<string, string> {
  const out: Record<string, string> = {};
  for (const q of questions) {
    const parts = [...(picks[q.question] ?? [])];
    const t = (typed[q.question] ?? "").trim();
    if (t) parts.push(t);
    out[q.question] = parts.join(", ");
  }
  return out;
}

/** 권한 요청에서 보일 본문: 무엇을(파일·주소) 어떻게(명령·변경) 하려는지. */
export function permissionSummary(ask: PendingAsk): { target?: string; body: string; note?: string } {
  const i = ask.toolInput as Record<string, unknown>;
  const str = (v: unknown) => (typeof v === "string" ? v : "");
  const target = str(i.file_path) || str(i.notebook_path) || str(i.path) || undefined;
  const diff = (o: unknown, n: unknown) =>
    [...str(o).split("\n").map((l) => "- " + l), ...str(n).split("\n").map((l) => "+ " + l)].join("\n");
  switch (ask.toolName) {
    case "Bash": return { body: str(i.command), note: str(i.description) || undefined };
    case "Write": return { target, body: str(i.content) };
    case "Edit": return { target, body: diff(i.old_string, i.new_string) };
    case "MultiEdit": return { target, body: (Array.isArray(i.edits) ? i.edits : []).map((e) => diff((e as Record<string, unknown>).old_string, (e as Record<string, unknown>).new_string)).join("\n⋯\n") };
    case "WebFetch": return { body: str(i.url), note: str(i.prompt) || undefined };
    case "ExitPlanMode": return { body: str(i.plan) };
    default: return { target, body: JSON.stringify(i, null, 2) };
  }
}

/** 작업 중인데 PTY 출력이 이만큼 끊기면 "멈췄을 수 있음"으로 본다. CLI는 작업 중 스피너·경과 시간을 늘 다시 그린다. */
export const STALL_MS = 20_000;

/** 대화 끝에 보일 지금 상태. 생각 중(움직임)과 멈춤·입력 대기(정지)를 가른다. */
export type LiveState =
  | { kind: "thinking"; elapsedMs: number; tool?: string }
  | { kind: "stalled"; elapsedMs: number; silentMs: number }
  | { kind: "human" }
  | { kind: "idle" }
  | { kind: "off" };

/** 본 대화에서 결과를 아직 못 받은 마지막 도구 호출(지금 도는 도구). */
export function runningTool(blocks: ChatBlock[]): string | undefined {
  for (let i = blocks.length - 1; i >= 0; i--) {
    const b = blocks[i];
    if (b.side) continue;
    if (b.head.kind !== "tool_use" || !b.head.toolName) return undefined;
    return b.children.some((c) => c.kind === "tool_result") ? undefined : b.head.toolName;
  }
  return undefined;
}

export function liveState(status: ChatLiveStatus | null, tool?: string): LiveState | null {
  if (!status) return null;
  if (!status.running || status.phase === "ended") return { kind: "off" };
  if (status.phase === "needs-human") return { kind: "human" };
  if (status.phase === "idle") return { kind: "idle" };
  if (status.phase !== "working") return null;
  const elapsedMs = Math.max(0, status.now - (status.since ?? status.now));
  const silentMs = status.outputAt ? Math.max(0, status.now - status.outputAt) : elapsedMs;
  if (silentMs >= STALL_MS) return { kind: "stalled", elapsedMs, silentMs };
  return { kind: "thinking", elapsedMs, tool };
}

/** 경과 시간 → [분, 초]. */
export function minSec(ms: number): [number, number] {
  const s = Math.floor(ms / 1000);
  return [Math.floor(s / 60), s % 60];
}
