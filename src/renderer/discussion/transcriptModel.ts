// src/renderer/discussion/transcriptModel.ts
//
// 속기록(토의 보기 창의 속기록 탭과 브라우저 내보내기). 기록(`transcript`)을 그대로 시간 순으로 보여 준다.
//
// 규칙
// - 동시 첫 라운드 발언은 전원이 낸 뒤에야 기록에 들어온다. 그 전에는 제출 수만 보인다(`openingProgress`).
// - 종류는 네 묶음으로 거른다: 토론 발언(첫 라운드·발언), 사회자(지목·정리), 사람과 문답(사회자 질문·사람의 답), 알림.
// - 화자를 고르면 그 참가자가 말한 것만 남는다(사람의 답·알림은 화자가 없어 빠진다).
// - 내보내기는 필터와 상관없이 전체를, 긴 발언도 펼친 전문으로 엮는다.
import type { Discussion, DiscussionUtterance, DiscussionUtteranceKind } from "@shared/types";
import { t } from "../i18n";
import { useAppStore } from "../store/appStore";
import { useRoleStore } from "./roleStore";
import { discussionElapsedMs, isActiveDiscussion, participantRoleLabel } from "./discussionModel";
import { participantLines, stamp } from "./discussionMemo";

export type TranscriptGroup = "debate" | "moderator" | "human" | "notice";
export const TRANSCRIPT_GROUPS: readonly TranscriptGroup[] = ["debate", "moderator", "human", "notice"];

export function transcriptGroup(kind: DiscussionUtteranceKind): TranscriptGroup {
  switch (kind) {
    case "opening": case "statement": return "debate";
    case "moderator": return "moderator";
    case "question": case "answer": return "human";
    case "notice": return "notice";
  }
}

export interface TranscriptFilter {
  groups: ReadonlySet<TranscriptGroup>;
  /** 고른 화자(agentId). 없으면 모두. */
  speaker?: string;
}

export interface TranscriptRow {
  /** `transcript` 안 위치(보드에서 가리킬 때 쓴다). */
  index: number;
  utterance: DiscussionUtterance;
  /** 보이는 행 가운데 이 라운드의 첫 행이면 라운드 구분선을 앞에 둔다. */
  roundStart: boolean;
}

export function transcriptRows(transcript: readonly DiscussionUtterance[], filter: TranscriptFilter): TranscriptRow[] {
  const rows: TranscriptRow[] = [];
  let round: number | undefined;
  transcript.forEach((utterance, index) => {
    if (!filter.groups.has(transcriptGroup(utterance.kind))) return;
    if (filter.speaker !== undefined && utterance.agentId !== filter.speaker) return;
    rows.push({ index, utterance, roundStart: utterance.round !== round });
    round = utterance.round;
  });
  return rows;
}

/** 접어 보이는 줄 수. 이보다 길면 앞부분만 보이고 펼치기를 단다. */
export const COLLAPSED_LINES = 10;
/** 줄이 적어도 글자가 이만큼 넘으면 접는다(한 줄짜리 긴 글). */
const COLLAPSED_CHARS = 900;

export function isLongUtterance(text: string): boolean {
  return text.split("\n").length > COLLAPSED_LINES || [...text].length > COLLAPSED_CHARS;
}

/** 동시 첫 라운드가 진행 중이면 제출한 토론자 수. 공개 전 발언 내용은 내놓지 않는다. */
export function openingProgress(d: Discussion): { submitted: number; total: number } | undefined {
  if (!isActiveDiscussion(d) || d.run?.phase !== "opening") return undefined;
  const turns = d.run.turns;
  return { submitted: turns.filter((turn) => turn.text !== undefined).length, total: turns.length };
}

/** 화자 이름(지금 이름). 사람의 답·알림처럼 화자가 없으면 그 종류의 이름. */
export function speakerName(u: Pick<DiscussionUtterance, "agentId" | "kind">): string {
  if (!u.agentId) return u.kind === "answer" ? t("discussion:transcript.human") : t("discussion:transcript.system");
  return useAppStore.getState().agents[u.agentId]?.name.trim() || t("discussion:board.missingAgent");
}

/** 화자의 토의 역할 이름. 참가자가 아니면 없다. */
export function speakerRole(d: Discussion, agentId: string | undefined): string | undefined {
  const p = agentId ? d.participants.find((x) => x.agentId === agentId) : undefined;
  return p ? participantRoleLabel(p.roleId, useRoleStore.getState().roles) : undefined;
}

export function clockTime(ms: number): string {
  const d = new Date(ms);
  return `${String(d.getHours()).padStart(2, "0")}:${String(d.getMinutes()).padStart(2, "0")}`;
}

/**
 * 발언 본문 안의 머리글을 세 단계 내린다(속기록의 라운드·발언 머리글보다 크지 않게). 코드 울타리 안은 건드리지 않는다.
 */
export function demoteHeadings(text: string): string {
  let fence: string | undefined;
  return text.split("\n").map((line) => {
    const open = /^\s*(`{3,}|~{3,})/.exec(line);
    if (open) {
      if (fence === undefined) fence = open[1][0];
      else if (open[1][0] === fence) fence = undefined;
      return line;
    }
    if (fence !== undefined) return line;
    return line.replace(/^(#{1,6})(\s)/, (_, hashes: string, space: string) => `${"#".repeat(Math.min(6, hashes.length + 3))}${space}`);
  }).join("\n");
}

function entryHeading(d: Discussion, u: DiscussionUtterance): string {
  const name = speakerName(u);
  const role = speakerRole(d, u.agentId);
  const who = u.agentId === d.moderatorAgentId && u.agentId ? `${name} (${t("discussion:memo.moderator")})` : name;
  const parts = [role && u.agentId !== d.moderatorAgentId ? `${who} · ${role}` : who, t(`discussion:transcript.kind.${u.kind}`), clockTime(u.at)];
  return `### ${parts.join(" — ")}`;
}

/** 브라우저로 여는 속기록 전문. 머리말(일시·진행·상태), 참가자, 라운드별 발언 순. */
export function composeTranscriptMarkdown(d: Discussion, now = Date.now()): string {
  const item = (s: string) => `- ${s}`;
  const facts = [
    item(t("discussion:memo.when", { start: stamp(d.startedAt), end: stamp(d.endedAt) })),
    item(t("discussion:memo.progress", {
      round: d.round, maxRounds: d.limits.maxRounds, minutes: Math.round(discussionElapsedMs(d, now) / 60_000), cost: d.costUsd.toFixed(2),
    })),
    item(t("discussion:transcript.stateLine", { state: t(`discussion:states.${d.state}`) })),
    ...(d.cwd ? [item(t("discussion:memo.cwd", { cwd: d.cwd }))] : []),
  ];
  const parts = [`# ${t("discussion:transcript.title", { topic: d.topic.trim() })}`, facts.join("\n")];
  if (d.context.trim()) parts.push(`## ${t("discussion:memo.context")}`, d.context.trim());
  parts.push(`## ${t("discussion:memo.participants")}`, participantLines(d));
  const rows = transcriptRows(d.transcript, { groups: new Set(TRANSCRIPT_GROUPS) });
  if (!rows.length) parts.push(`_${t("discussion:transcript.empty")}_`);
  for (const row of rows) {
    if (row.roundStart) parts.push(`## ${t("discussion:transcript.round", { round: row.utterance.round })}`);
    parts.push(entryHeading(d, row.utterance), demoteHeadings(row.utterance.text.trim()) || `_${t("discussion:memo.none")}_`);
  }
  const pending = openingProgress(d);
  if (pending) parts.push(`_${t("discussion:transcript.openingPending", pending)}_`);
  return parts.join("\n\n") + "\n";
}
