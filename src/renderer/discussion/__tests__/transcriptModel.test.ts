import { beforeEach, describe, expect, it } from "vitest";
import type { Discussion, DiscussionUtterance } from "@shared/types";
import { t } from "../../i18n";
import { useAppStore } from "../../store/appStore";
import { useRoleStore } from "../roleStore";
import {
  TRANSCRIPT_GROUPS, composeTranscriptMarkdown, demoteHeadings, isLongUtterance, openingProgress, transcriptRows,
} from "../transcriptModel";

const said = (round: number, kind: DiscussionUtterance["kind"], text: string, agentId?: string): DiscussionUtterance =>
  ({ at: 1_720_000_000_000 + round * 60_000, round, kind, text, ...(agentId ? { agentId } : {}) });
const transcript: DiscussionUtterance[] = [
  said(1, "opening", "Keep a CHANGELOG.", "a"),
  said(1, "opening", "Use a docs folder.", "b"),
  said(2, "moderator", "- Ari: Why?\n- Bo: Who owns it?", "m"),
  said(2, "statement", "Tooling reads it.", "a"),
  said(2, "question", "Which audience?", "m"),
  said(2, "answer", "End users."),
  said(3, "notice", "Round limit reached."),
];
const d: Discussion = {
  id: "d1", topic: " Release notes ", context: "", moderatorAgentId: "m",
  participants: [
    { agentId: "a", roleId: "critic", agent: { cli: "codex" } },
    { agentId: "m", roleId: "mod", agent: { cli: "claude" } },
    { agentId: "b", agent: { cli: "pi" } },
  ],
  limits: { maxRounds: 4, maxMinutes: 30, maxCostUsd: 5, usageGuard: true }, state: "review", round: 3, transcript,
  costUsd: 0.5, createdAt: 1, updatedAt: 2, startedAt: 1_720_000_000_000, endedAt: 1_720_000_600_000, activeMs: 600_000,
};
const all = new Set(TRANSCRIPT_GROUPS);

beforeEach(() => {
  useAppStore.setState({ agents: {
    m: { id: "m", name: "Mina", role: "", seed: "m", createdAt: 0, deskIndex: 0 },
    a: { id: "a", name: "Ari", role: "", seed: "a", createdAt: 0, deskIndex: 1 },
    b: { id: "b", name: "Bo", role: "", seed: "b", createdAt: 0, deskIndex: 2 },
  } });
  useRoleStore.setState({ roles: [
    { id: "mod", name: "Moderator", perspective: "", stance: "moderator", criteria: "" },
    { id: "critic", name: "Critic", perspective: "", stance: "critical", criteria: "" },
  ], status: "ready" });
});

describe("속기록 행", () => {
  it("종류 묶음과 화자로 거르고, 보이는 행 기준으로 라운드 구분선을 단다", () => {
    const rows = transcriptRows(transcript, { groups: all });
    expect(rows.map((r) => r.index)).toEqual([0, 1, 2, 3, 4, 5, 6]);
    expect(rows.filter((r) => r.roundStart).map((r) => r.index)).toEqual([0, 2, 6]);

    // 사회자를 끄면 2라운드의 첫 행은 Ari의 발언이 된다. 사람과 문답은 질문과 답을 함께 뺀다.
    const debate = transcriptRows(transcript, { groups: new Set(["debate", "notice"]) });
    expect(debate.map((r) => r.index)).toEqual([0, 1, 3, 6]);
    expect(debate.filter((r) => r.roundStart).map((r) => r.index)).toEqual([0, 3, 6]);

    // 화자를 고르면 화자가 없는 답·알림은 빠지고 사회자의 질문은 사회자 몫이다.
    expect(transcriptRows(transcript, { groups: all, speaker: "m" }).map((r) => r.index)).toEqual([2, 4]);
    expect(transcriptRows(transcript, { groups: all, speaker: "a" }).map((r) => [r.index, r.roundStart])).toEqual([[0, true], [3, true]]);
  });

  it("10줄이나 긴 한 줄을 넘는 발언만 접는다", () => {
    expect(isLongUtterance(Array(10).fill("x").join("\n"))).toBe(false);
    expect(isLongUtterance(Array(11).fill("x").join("\n"))).toBe(true);
    expect(isLongUtterance("가".repeat(901))).toBe(true);
  });

  it("발언 안의 머리글을 내리되 코드 울타리 안은 그대로 둔다", () => {
    const text = "# Plan\n```sh\n# comment\n```\n## Next\n~~~\n# also code\n~~~\n###### deep";
    expect(demoteHeadings(text)).toBe("#### Plan\n```sh\n# comment\n```\n##### Next\n~~~\n# also code\n~~~\n###### deep");
  });
});

describe("비공개 첫 라운드", () => {
  it("진행 중인 첫 라운드에서만 제출 수를 센다", () => {
    const opening: Discussion = {
      ...d, state: "running", round: 1, transcript: [],
      run: { phase: "opening", seen: {}, turns: [{ agentId: "a", text: "secret" }, { agentId: "b", sentAt: 1 }] },
    };
    expect(openingProgress(opening)).toEqual({ submitted: 1, total: 2 });
    expect(openingProgress({ ...opening, state: "review" })).toBeUndefined();
    expect(openingProgress({ ...opening, run: { ...opening.run!, phase: "moderator" } })).toBeUndefined();
  });
});

describe("속기록 내보내기", () => {
  it("머리말·참가자·라운드별 전체 발언을 엮고 화자·역할·종류를 머리글에 적는다", () => {
    const md = composeTranscriptMarkdown(d, d.endedAt);
    expect(md.split("\n")[0]).toBe(`# ${t("discussion:transcript.title", { topic: "Release notes" })}`);
    expect(md).toContain(t("discussion:transcript.stateLine", { state: t("discussion:states.review") }));
    for (const round of [1, 2, 3]) expect(md).toContain(`## ${t("discussion:transcript.round", { round })}`);
    for (const u of transcript) expect(md).toContain(u.text);
    expect(md).toContain(`### Ari · Critic — ${t("discussion:transcript.kind.opening")}`);
    expect(md).toContain(`### Mina (${t("discussion:memo.moderator")}) — ${t("discussion:transcript.kind.question")}`);
    expect(md).toContain(`### ${t("discussion:transcript.human")} — ${t("discussion:transcript.kind.answer")}`);
    // 라운드 순서대로 나온다.
    expect(md.indexOf("Keep a CHANGELOG.")).toBeLessThan(md.indexOf("Tooling reads it."));
    expect(md.indexOf("Tooling reads it.")).toBeLessThan(md.indexOf("Round limit reached."));
  });

  it("공개 전 첫 라운드 발언은 내보내지 않고 제출 수만 적는다", () => {
    const opening: Discussion = {
      ...d, state: "running", round: 1, transcript: [], endedAt: undefined, activeSince: 1,
      run: { phase: "opening", seen: {}, turns: [{ agentId: "a", text: "secret plan" }, { agentId: "b", sentAt: 1 }] },
    };
    const md = composeTranscriptMarkdown(opening, 2);
    expect(md).not.toContain("secret plan");
    expect(md).toContain(t("discussion:transcript.openingPending", { submitted: 1, total: 2 }));
    expect(md).toContain(t("discussion:transcript.empty"));
  });
});
