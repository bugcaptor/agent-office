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

// ── 호스트 입력 즉시 에코 ─────────────────────────────────────────────
//
// 데스크톱에서 사람이 친 프롬프트는 `UserPromptSubmit` 훅을 타고 activity
// 프레임(kind=prompt, text=원문)으로 **즉시** 도착하지만, 같은 문장이 채팅
// 버블이 되려면 CLI가 JSONL에 그것을 쓰고 서버 tail이 다음 틱에 집어 올릴
// 때까지 기다려야 한다. 그 공백을 "대기 중 유저 버블"로 메운다.
//
// 정합성은 **소거**로 맞춘다: 전사 항목이 도착하면 같은 문장의 에코를 지운다.
// 낙관 표시의 고전적 함정(원본이 영영 안 와서 버블이 두 벌 남는 것)은 상한
// (`MAX_ECHOES`)과 정규화 매칭이 막는다.

/** 동시에 들고 있을 에코 상한. 넘치면 오래된 것부터 버린다. */
export const MAX_ECHOES = 10;

/** 문장 비교용 정규화 — 앞뒤 공백만 턴다(주입은 원문을 그대로 보낸다). */
function echoKey(text: string): string {
  return text.trim();
}

/**
 * activity 페이로드에서 에코할 프롬프트 원문을 뽑는다. 없으면 null.
 * kind=prompt여도 훅 body에 프롬프트가 없으면 `text`가 비어 온다 — 그때는
 * 진행 라인만 뜨고 에코는 만들지 않는다.
 */
export function promptEcho(payload: { kind?: string; text?: string | null }): string | null {
  if (payload.kind !== "prompt") return null;
  const text = payload.text?.trim();
  return text ? text : null;
}

/**
 * 에코 하나를 목록에 넣는다. **같은 문장이 이미 대기 중이면 넣지 않는다** —
 * 웹에서 보낸 낙관 에코와 그 직후 호스트가 미러하는 activity prompt가 같은
 * 문장이라 그냥 쌓으면 버블이 두 개가 된다.
 */
export function pushEcho(echoes: string[], text: string, max = MAX_ECHOES): string[] {
  const key = echoKey(text);
  if (!key) return echoes;
  if (echoes.some((e) => echoKey(e) === key)) return echoes;
  return [...echoes, text].slice(-max);
}

/** 에코 하나를 되돌린다(주입 실패 → 드래프트 복원과 한 쌍). */
export function removeEcho(echoes: string[], text: string): string[] {
  const at = echoes.findIndex((e) => echoKey(e) === echoKey(text));
  if (at < 0) return echoes;
  return [...echoes.slice(0, at), ...echoes.slice(at + 1)];
}

/**
 * 전사에 실제로 나타난 유저 발화만큼 에코를 지운다. 매칭되지 않은 에코는
 * 남는다(교체=backfill 프레임에서도 마찬가지 — 아직 전사에 없는 방금 입력이
 * 사라지면 안 된다). 지울 것이 없으면 **같은 참조**를 돌려준다.
 */
export function dedupEchoes(echoes: string[], items: TranscriptItem[]): string[] {
  if (echoes.length === 0) return echoes;
  const rest = [...echoes];
  for (const item of items) {
    if (item.role !== "user" || item.kind !== "text") continue;
    const key = echoKey(item.text);
    if (!key) continue;
    const at = rest.findIndex((e) => echoKey(e) === key);
    if (at >= 0) rest.splice(at, 1);
  }
  return rest.length === echoes.length ? echoes : rest;
}

// ── 긴 본문 접기 ──────────────────────────────────────────────────────
//
// 서버가 웹용 한도(16k자/240줄)로 자르므로 전문이 오지만, 그대로 펼쳐 두면
// 도구 결과 한 건이 화면을 통째로 먹는다. 접힌 상태로 시작하고 "더 보기"로
// 편다.

/** 접힌 채로 시작할 문자 수 기준. */
export const COLLAPSE_CHARS = 500;
/** 접힌 채로 시작할 줄 수 기준. */
export const COLLAPSE_LINES = 8;
/** 접었을 때 보여줄 줄 수(기준보다 하나 적게 — 잘렸다는 것이 보이게). */
export const PREVIEW_LINES = 6;

/** 이 본문이 접기 대상인가. */
export function isLongText(
  text: string,
  maxChars = COLLAPSE_CHARS,
  maxLines = COLLAPSE_LINES
): boolean {
  return text.length > maxChars || text.split("\n").length > maxLines;
}

/**
 * 접힌 미리보기 본문. 길지 않으면 원문 그대로다(호출부가 분기하지 않아도
 * 되게 — `isLongText`가 false면 이 함수는 항등이다).
 */
export function previewText(
  text: string,
  maxChars = COLLAPSE_CHARS,
  previewLines = PREVIEW_LINES
): string {
  if (!isLongText(text, maxChars)) return text;
  const head = text.split("\n").slice(0, previewLines).join("\n");
  return head.length > maxChars ? head.slice(0, maxChars) : head;
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
