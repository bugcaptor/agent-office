// src/renderer/discussion/meetingBoardModel.ts
//
// 회의록 보드(오피스 회의실 옆)의 행 파생. 순수 함수 — 스토어·DOM 의존 없음.
//
// 규칙
// - 행 순서 = 회의실 의자 순서(사회자 먼저, 이어서 토론자 저장 순서).
// - 최근 발언은 기록(transcript)에서만 고른다. 동시 첫 라운드 발언은 전원이 낸 뒤에야 기록에 들어오므로
//   공개 전 발언이 보드에 새지 않는다.
// - 발췌는 앞부분을 자른 한 줄이다(요약 LLM 호출 없음). 전문은 보기 창의 속기록 탭에서 본다(행을 누르면 열린다).
import type { Discussion, DiscussionUtterance, DiscussionUtteranceKind } from "@shared/types";
import { participantsModeratorFirst } from "./discussionModel";
import { speakingSince } from "./officeStage";

/** 보드 한 줄에 싣는 발췌 길이(글자). 넘치면 CSS 말줄임이 한 번 더 자른다. */
export const EXCERPT_CHARS = 80;
/** 행에 마우스를 올렸을 때 보여 줄 조금 긴 발췌(글자). */
export const EXCERPT_TITLE_CHARS = 300;

export interface MeetingBoardLast {
  /** `transcript` 안 위치(행을 누르면 속기록에서 이 발언을 가리킨다). */
  index: number;
  at: number;
  kind: DiscussionUtteranceKind;
  /** 한 줄로 편 발췌(EXCERPT_CHARS까지). */
  excerpt: string;
  /** 마우스 올림용 조금 긴 발췌. */
  title: string;
}

export interface MeetingBoardRow {
  agentId: string;
  moderator: boolean;
  /** 말하는 중이면 차례를 받은 시각. */
  speakingSince?: number;
  last?: MeetingBoardLast;
}

export function meetingBoardRows(d: Discussion): MeetingBoardRow[] {
  const speaking = speakingSince(d);
  const seen = new Set<string>();
  const rows: MeetingBoardRow[] = [];
  for (const p of participantsModeratorFirst(d)) {
    if (seen.has(p.agentId)) continue;
    seen.add(p.agentId);
    const index = lastUtteranceIndex(d.transcript, p.agentId);
    const u = index >= 0 ? d.transcript[index] : undefined;
    rows.push({
      agentId: p.agentId,
      moderator: p.agentId === d.moderatorAgentId,
      ...(speaking.has(p.agentId) ? { speakingSince: speaking.get(p.agentId) } : {}),
      ...(u ? { last: { index, at: u.at, kind: u.kind, excerpt: utteranceExcerpt(u, EXCERPT_CHARS), title: utteranceExcerpt(u, EXCERPT_TITLE_CHARS) } } : {}),
    });
  }
  return rows;
}

function lastUtteranceIndex(transcript: readonly DiscussionUtterance[], agentId: string): number {
  for (let i = transcript.length - 1; i >= 0; i--) if (transcript[i].agentId === agentId) return i;
  return -1;
}

/**
 * 발언 하나를 한 줄로 편다. 사회자의 지목(`- 이름: 질문` 줄들)은 `→ 이름: 질문`을 ` / `로 잇고, 그 밖에는 마크다운
 * 머리표·강조 기호를 걷어 내고 공백을 하나로 줄인 뒤 앞부분만 남긴다.
 */
export function utteranceExcerpt(u: Pick<DiscussionUtterance, "kind" | "text">, max: number): string {
  const lines = u.text.split("\n").map((l) => l.trim()).filter(Boolean);
  const directed = u.kind === "moderator" && lines.length > 0 && lines.every((l) => /^- [^:]+: /.test(l));
  const flat = directed
    ? lines.map((l) => `→ ${l.slice(2)}`).join(" / ")
    : lines.map(stripMarkdown).filter(Boolean).join(" ");
  const text = flat.replace(/\s+/g, " ").trim();
  const chars = [...text];
  return chars.length > max ? `${chars.slice(0, max).join("")}…` : text;
}

function stripMarkdown(line: string): string {
  return line
    .replace(/^#{1,6}\s+/, "")
    .replace(/^>\s?/, "")
    .replace(/^([-*+]|\d+[.)])\s+/, "")
    .replace(/(\*\*|__|`)/g, "")
    .trim();
}

/** "N분 전" 표기용 경과. 1분 미만은 방금, 60분부터는 시간 단위. */
export type BoardAge = { unit: "now" } | { unit: "minutes"; count: number } | { unit: "hours"; count: number };

export function boardAge(at: number, now: number): BoardAge {
  const minutes = Math.floor(Math.max(0, now - at) / 60_000);
  if (minutes < 1) return { unit: "now" };
  if (minutes < 60) return { unit: "minutes", count: minutes };
  return { unit: "hours", count: Math.floor(minutes / 60) };
}
