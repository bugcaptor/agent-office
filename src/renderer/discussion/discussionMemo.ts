// src/renderer/discussion/discussionMemo.ts
//
// 결정 메모(Markdown). 사회자의 정리 지시(wrap)에 담긴 칸을 화면 언어의 절로 엮고, 머리말(일시·진행·마무리 까닭)과
// 참가자·역할 목록은 앱이 붙인다. 역할 이름은 지금 이름으로 박아 둔다(나중에 역할이 지워져도 메모는 그대로다).
// 저장은 Rust가 `<app_data>/discussions/memos/<id>.md`에 한다. 전체 회의록은 남기지 않는다.
import type { Discussion, DiscussionLimitReason } from "@shared/types";
import { t } from "../i18n";
import { useAppStore } from "../store/appStore";
import { useRoleStore } from "./roleStore";

/** 정리 지시의 구조화된 칸. 비어 있으면 그 절은 "적힌 것 없음"으로 둔다(발산 묶음은 아예 뺀다). */
export interface WrapMemo {
  reasons: string[];
  rejected: { option: string; reason: string }[];
  openIssues: string[];
  userChecks: string[];
  ideas: { group: string; items: string[] }[];
}

export const emptyWrapMemo = (): WrapMemo => ({ reasons: [], rejected: [], openIssues: [], userChecks: [], ideas: [] });

const text = (v: unknown) => (typeof v === "string" ? v.trim() : typeof v === "number" ? String(v) : "");
const list = (v: unknown): unknown[] => (Array.isArray(v) ? v : v === undefined || v === null || v === "" ? [] : [v]);
const strings = (v: unknown) => list(v).map(text).filter(Boolean);
const field = (o: Record<string, unknown>, ...keys: string[]) => keys.map((k) => o[k]).find((v) => v !== undefined);
const record = (v: unknown): Record<string, unknown> => (v && typeof v === "object" && !Array.isArray(v) ? v as Record<string, unknown> : {});

/** 지시 객체에서 메모 칸을 읽는다. 키는 snake_case·camelCase 둘 다 받고, 하나뿐이면 문자열도 받는다. */
export function parseWrapMemo(o: Record<string, unknown>): WrapMemo {
  const rejected = list(field(o, "rejected", "rejected_alternatives", "rejectedAlternatives")).map((item) => {
    if (typeof item === "string") return { option: item.trim(), reason: "" };
    const r = record(item);
    return { option: text(field(r, "option", "alternative", "name", "title")), reason: text(field(r, "reason", "why")) };
  }).filter((r) => r.option || r.reason);
  const ideas = list(field(o, "ideas", "groups")).map((item) => {
    if (typeof item === "string") return { group: "", items: [item.trim()] };
    const r = record(item);
    return { group: text(field(r, "group", "theme", "title", "name")), items: strings(field(r, "items", "ideas")) };
  }).filter((g) => g.group || g.items.length);
  return {
    reasons: strings(field(o, "reasons", "rationale", "grounds")),
    rejected,
    openIssues: strings(field(o, "open_issues", "openIssues", "open")),
    userChecks: strings(field(o, "user_checks", "userChecks", "confirm", "needs_user")),
    ideas,
  };
}

export const wrapMemoEmpty = (m: WrapMemo) =>
  !m.reasons.length && !m.rejected.length && !m.openIssues.length && !m.userChecks.length && !m.ideas.length;

/** 메모를 쓸 때 실행기가 넘기는 것. 요약이 없으면 끝난 까닭(notice)을 결론 자리에 적는다. */
export interface MemoInput {
  summary?: string;
  memo?: WrapMemo;
  /** 강제 정리였다면 그 상한. */
  forced?: DiscussionLimitReason;
  /** 요약 없이 끝난 까닭(wrapTimeout·wrapMissing 문장). */
  notice?: string;
}

function stamp(ms: number | undefined): string {
  if (ms === undefined) return "-";
  const d = new Date(ms);
  const p = (n: number) => String(n).padStart(2, "0");
  return `${d.getFullYear()}-${p(d.getMonth() + 1)}-${p(d.getDate())} ${p(d.getHours())}:${p(d.getMinutes())}`;
}

/** 한 줄 항목. 여러 줄이면 다음 줄을 들여 써 목록이 끊기지 않게 한다. */
const item = (s: string) => `- ${s.trim().replace(/\n/g, "\n  ")}`;
const none = () => `_${t("discussion:memo.none")}_`;
const bullets = (xs: string[]) => (xs.length ? xs.map(item).join("\n") : none());

function participantLines(d: Discussion): string {
  const agents = useAppStore.getState().agents;
  const roles = useRoleStore.getState().roles;
  const ordered = [...d.participants].sort((a, b) => Number(b.agentId === d.moderatorAgentId) - Number(a.agentId === d.moderatorAgentId));
  return ordered.map((p) => {
    const name = agents[p.agentId]?.name.trim() || t("discussion:board.missingAgent");
    const role = !p.roleId ? t("discussion:board.noRole") : roles.find((r) => r.id === p.roleId)?.name.trim() || t("discussion:board.missingRole");
    const duty = p.agentId === d.moderatorAgentId ? t("discussion:memo.moderator") : t("discussion:memo.debater");
    const cli = [p.agent.cli, p.agent.model, p.agent.effort].filter(Boolean).join(" · ");
    return item(`**${name}** — ${duty} · ${role} · ${cli}`);
  }).join("\n");
}

function endingText(input: MemoInput): string {
  if (input.notice && !input.summary) return input.notice;
  if (input.forced) return t("discussion:memo.endedForced", { reason: t(`discussion:run.limitName.${input.forced}`) });
  return t("discussion:memo.endedByModerator");
}

/** 결정 메모 본문. `d`는 정리를 마친 토의(끝난 시각이 있다). */
export function composeDecisionMemo(d: Discussion, input: MemoInput): string {
  const memo = input.memo ?? emptyWrapMemo();
  const summary = input.summary?.trim();
  const facts = [
    item(t("discussion:memo.when", { start: stamp(d.startedAt), end: stamp(d.endedAt) })),
    item(t("discussion:memo.progress", {
      round: d.round, maxRounds: d.limits.maxRounds, minutes: Math.round((d.activeMs ?? 0) / 60_000), cost: d.costUsd.toFixed(2),
    })),
    item(t("discussion:memo.ending", { text: endingText(input) })),
    ...(d.cwd ? [item(t("discussion:memo.cwd", { cwd: d.cwd }))] : []),
  ];
  const sections: [string, string][] = [
    ...(d.context.trim() ? [[t("discussion:memo.context"), d.context.trim()] as [string, string]] : []),
    [t("discussion:memo.conclusion"), summary || `_${input.notice ?? t("discussion:memo.noSummary")}_`],
    [t("discussion:memo.reasons"), bullets(memo.reasons)],
    [t("discussion:memo.rejected"), memo.rejected.length
      ? memo.rejected.map((r) => item(r.option && r.reason ? `**${r.option}** — ${r.reason}` : r.option ? `**${r.option}**` : r.reason)).join("\n")
      : none()],
    [t("discussion:memo.openIssues"), bullets(memo.openIssues)],
    [t("discussion:memo.userChecks"), bullets(memo.userChecks)],
  ];
  if (memo.ideas.length) {
    sections.push([t("discussion:memo.ideas"), memo.ideas.map((g) =>
      [`### ${g.group || t("discussion:memo.ideasOther")}`, bullets(g.items)].join("\n\n")).join("\n\n")]);
  }
  sections.push([t("discussion:memo.participants"), participantLines(d)]);
  return [
    `# ${t("discussion:memo.title", { topic: d.topic.trim() })}`,
    facts.join("\n"),
    ...sections.map(([head, body]) => `## ${head}\n\n${body}`),
  ].join("\n\n") + "\n";
}
