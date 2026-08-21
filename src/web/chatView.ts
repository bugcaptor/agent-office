// src/web/chatView.ts
//
// 채팅 뷰의 **순수 규칙**. 렌더 컴포넌트에서 떼어 둔 이유는 테스트다 —
// 버블 병합·자동 스크롤 판단·알림 분류는 DOM 없이 검증할 수 있어야 한다.

import type { NotificationItem, TranscriptItem } from "./protocol";

/** 화면에 들고 있는 최대 항목 수. 폰 메모리와 렌더 비용의 상한선. */
export const MAX_ITEMS = 400;

/** 맨 아래로 자동 추종할지 판단하는 여유(px). 한 줄 높이쯤. */
export const STICK_THRESHOLD_PX = 48;

export interface ChatFrame {
  items?: TranscriptItem[];
  backfill?: boolean;
  unavailable?: boolean;
}

/**
 * chat 프레임 하나를 목록에 반영한다.
 *
 * `backfill`은 **교체**다: 재접속·늦은 합류에서 서버가 최근 대화를 다시
 * 보내는데, 이어 붙이면 같은 대화가 두 벌 쌓인다. 교체 규칙이 그 문제를
 * 클라이언트 쪽 dedup 없이 없앤다(서버 규칙과 한 쌍 — protocol.rs 참고).
 */
export function applyChatFrame(
  prev: TranscriptItem[],
  frame: ChatFrame
): TranscriptItem[] {
  const incoming = frame.items ?? [];
  if (frame.backfill) return incoming.slice(-MAX_ITEMS);
  if (incoming.length === 0) return prev;
  return [...prev, ...incoming].slice(-MAX_ITEMS);
}

/** 스크롤 위치가 "맨 아래를 보고 있는" 상태인가. */
export function isAtBottom(
  view: { scrollTop: number; scrollHeight: number; clientHeight: number },
  threshold = STICK_THRESHOLD_PX
): boolean {
  return view.scrollHeight - view.scrollTop - view.clientHeight <= threshold;
}

/** 도구 항목의 접힌 한 줄. 펼치면 `item.text` 전문이 보인다. */
export function toolSummary(item: TranscriptItem): string {
  const name = item.toolName ?? (item.kind === "tool_result" ? "결과" : "활동");
  const head = item.text.split("\n", 1)[0]?.trim() ?? "";
  const brief = head.length > 60 ? `${head.slice(0, 60)}…` : head;
  return brief ? `${name} · ${brief}` : name;
}

export function itemGlyph(item: TranscriptItem): string {
  if (item.kind === "tool_result") return item.isError ? "⚠️" : "↩︎";
  if (item.kind === "tool_use") return "🔧";
  return item.role === "user" ? "🙋" : "🤖";
}

/**
 * 상단에 고정할 알림(=확인 요청)인가.
 *
 * hook 알림은 "에이전트가 사람에게 묻고 멈춰 있다"는 뜻이라 퀵 키 카드가
 * 붙는다. stop/bell은 완료·경보라 흘려보내는 라인이면 충분하다.
 */
export function isQuestion(n: NotificationItem): boolean {
  return n.source === "hook";
}

export interface ActivityLine {
  text: string;
}

/**
 * activity 프레임 → 진행 라인. 표시할 것이 없으면 null.
 * `prompt`는 턴 시작, `tool`은 도구 하트비트다(sub-* 는 미니 캐릭터 전용이라
 * 채팅에서는 무시한다 — 서브에이전트 수는 여기서 의미가 없다).
 */
export function activityLine(payload: {
  kind?: string;
  text?: string | null;
}): ActivityLine | null {
  switch (payload.kind) {
    case "prompt":
    case "resume":
      return { text: "⏳ 작업 중" };
    case "tool": {
      const tool = payload.text?.trim();
      return { text: tool ? `⏳ 작업 중 · 🔧 ${tool}` : "⏳ 작업 중" };
    }
    default:
      return null;
  }
}

/** 확인 요청 카드에 붙는 퀵 키. 서버 allowlist(`key_bytes`)와 같은 이름들. */
export const QUICK_KEYS: Array<{ label: string; key: string }> = [
  { label: "1", key: "1" },
  { label: "2", key: "2" },
  { label: "3", key: "3" },
  { label: "y", key: "y" },
  { label: "n", key: "n" },
  { label: "⏎", key: "enter" },
  { label: "Esc", key: "esc" },
  { label: "↑", key: "up" },
  { label: "↓", key: "down" },
  { label: "^C", key: "ctrl-c" },
];
