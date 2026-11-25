// src/renderer/store/mascotSpeech.ts
//
// 마스코트 머리 위 말풍선에 띄울 "에이전트가 한 말"을 고르는 순수 함수.
// 도구 호출 요약(latestToolText)은 싣지 않는다 — 말풍선은 에이전트의 말만.
//
// 출처는 둘이다.
//  - 턴 중 내레이션: Claude(와 pi)가 도구 사이에 하는 말. 러스트가 PostToolUse
//    마다 전사 꼬리에서 뽑아 5초 스로틀로 보내고 `taskLabels[id].
//    latestAssistantText`에 쌓인다. 새 프롬프트가 오면 비워진다.
//  - 턴 끝 최종 응답: 완료(stop) 알림 본문. Claude는 전사 마지막 응답,
//    Codex는 `last_assistant_message`(러스트에서 300자 절단). Codex는 턴 중
//    내레이션이 없어 이쪽만 뜬다.
//
// 보여 줄 시간(타이머)은 브리지가 맡는다 — 여기는 "지금 말할 거리"만 계산한다.

import type { AgentTaskLabel, Notification } from "./types";
import { truncateChars } from "../labels/labelText";
import { STOP_FALLBACK_MESSAGES } from "../i18n/backendPhrases";

/** 말풍선에 싣는 최대 글자 수(chars). 200px 폭 × 3줄에 맞춘 값 — 넘치는 건
 *  CSS line-clamp가 한 번 더 자른다. */
export const MASCOT_SPEECH_MAX_CHARS = 80;

/** 줄 머리 마크다운 표식(제목·인용·목록) — 말풍선엔 기호가 의미 없다. */
const LINE_MARKER = /^(?:#{1,6}\s+|>\s*|[-*+]\s+|\d+[.)]\s+)/;

/**
 * 원문 → 말풍선 텍스트. 코드 펜스 줄을 버리고, 줄 머리 표식과 강조/코드
 * 기호를 걷어낸 뒤 줄을 공백으로 이어 붙여 `MASCOT_SPEECH_MAX_CHARS`로 자른다.
 * 남는 게 없으면 null.
 */
export function toSpeechText(raw: string | undefined | null): string | null {
  if (!raw) return null;
  const parts: string[] = [];
  for (const line of raw.split("\n")) {
    const trimmed = line.trim();
    if (trimmed.length === 0 || trimmed.startsWith("```")) continue;
    const cleaned = trimmed.replace(LINE_MARKER, "").replace(/\*\*|__|`/g, "").trim();
    if (cleaned.length > 0) parts.push(cleaned);
  }
  const text = parts.join(" ").replace(/\s+/g, " ").trim();
  return text.length > 0 ? truncateChars(text, MASCOT_SPEECH_MAX_CHARS) : null;
}

/**
 * 마스코트가 보여 주는 에이전트(`agentId`)의 지금 말할 거리.
 *  1. 그 에이전트의 가장 최근 알림이 완료(done)면 최종 응답. 메시지가 없어
 *     hub가 채운 폴백 문구는 에이전트가 한 말이 아니라 제외한다.
 *     질문·정보 알림이면 null — 배지가 이미 알린다(권한 요청 문구는 CLI가
 *     만든 것이지 에이전트가 한 말이 아니다).
 *  2. 알림이 없고 작업 중이면 턴 중 내레이션.
 *  3. 그 밖엔 null.
 */
export function pickMascotSpeech(input: {
  agentId: string | null;
  working: boolean;
  /** 최신순(appStore가 앞에 끼운다). */
  notifications: ReadonlyArray<Pick<Notification, "agentId" | "type" | "message">>;
  taskLabels: Record<string, Pick<AgentTaskLabel, "latestAssistantText"> | undefined>;
}): string | null {
  const { agentId, working, notifications, taskLabels } = input;
  if (agentId === null) return null;
  const latest = notifications.find((n) => n.agentId === agentId);
  if (latest) {
    if (latest.type !== "done" || STOP_FALLBACK_MESSAGES.has(latest.message.trim())) return null;
    return toSpeechText(latest.message);
  }
  if (!working) return null;
  return toSpeechText(taskLabels[agentId]?.latestAssistantText);
}
