import { beforeEach, describe, expect, it } from "vitest";
import type { Discussion } from "@shared/types";
import { t } from "../../i18n";
import { useAppStore } from "../../store/appStore";
import { useRoleStore } from "../roleStore";
import { composeDecisionMemo, emptyWrapMemo, parseWrapMemo } from "../discussionMemo";

const d: Discussion = {
  id: "d1", topic: "  Weekly digest name ", context: "Ideas only.", cwd: "/repo", moderatorAgentId: "m",
  participants: [
    { agentId: "a", roleId: "preset-ideation", agent: { cli: "codex", model: "gpt-5.5" } },
    { agentId: "m", roleId: "preset-moderator", agent: { cli: "claude", model: "opus", effort: "high" } },
    { agentId: "gone", agent: { cli: "pi" } },
  ],
  limits: { maxRounds: 4, maxMinutes: 30, maxCostUsd: 5, usageGuard: true }, state: "review", round: 3, transcript: [],
  costUsd: 0.314, createdAt: 1, updatedAt: 2, startedAt: 1_720_000_000_000, endedAt: 1_720_000_600_000, activeMs: 540_000,
};
const section = (memo: string, head: string) => memo.split(`## ${head}\n\n`)[1]?.split("\n\n## ")[0].trimEnd();

beforeEach(() => {
  useAppStore.setState({ agents: {
    m: { id: "m", name: "Mina", role: "", seed: "m", createdAt: 0, deskIndex: 0 },
    a: { id: "a", name: "Ari", role: "", seed: "a", createdAt: 0, deskIndex: 1 },
  } });
  useRoleStore.setState({ roles: [
    { id: "preset-moderator", name: "Moderator", perspective: "", stance: "moderator", criteria: "" },
    { id: "preset-ideation", name: "Ideation", perspective: "", stance: "divergent", criteria: "" },
  ], status: "ready" });
});

describe("결정 메모", () => {
  it("결론·근거·기각 대안·남은 쟁점·확인 필요·발산 묶음과 참가자·역할을 절로 엮는다", () => {
    const memo = composeDecisionMemo(d, {
      summary: "Go with **Pulse**.",
      memo: {
        reasons: ["Short", "Two\nlines"], rejected: [{ option: "Office Weekly", reason: "Too generic" }, { option: "", reason: "Only a reason" }],
        openIssues: ["Email too?"], userChecks: ["Pulse or Beat?"], ideas: [{ group: "Short", items: ["Pulse", "Beat"] }, { group: "", items: ["Desk Notes"] }],
      },
    });
    expect(memo.split("\n")[0]).toBe(`# ${t("discussion:memo.title", { topic: "Weekly digest name" })}`);
    expect(memo).toContain(t("discussion:memo.cwd", { cwd: "/repo" }));
    expect(memo).toContain(t("discussion:memo.progress", { round: 3, maxRounds: 4, minutes: 9, cost: "0.31" }));
    expect(memo).toContain(t("discussion:memo.ending", { text: t("discussion:memo.endedByModerator") }));
    expect(section(memo, t("discussion:memo.context"))).toBe("Ideas only.");
    expect(section(memo, t("discussion:memo.conclusion"))).toBe("Go with **Pulse**.");
    expect(section(memo, t("discussion:memo.reasons"))).toBe("- Short\n- Two\n  lines");
    expect(section(memo, t("discussion:memo.rejected"))).toBe("- **Office Weekly** — Too generic\n- Only a reason");
    expect(section(memo, t("discussion:memo.openIssues"))).toBe("- Email too?");
    expect(section(memo, t("discussion:memo.userChecks"))).toBe("- Pulse or Beat?");
    expect(section(memo, t("discussion:memo.ideas"))).toBe(`### Short\n\n- Pulse\n- Beat\n\n### ${t("discussion:memo.ideasOther")}\n\n- Desk Notes`);
    // 사회자가 먼저, 지워진 팀원도 자리를 지킨다. 역할 이름은 지금 이름으로 박는다.
    expect(section(memo, t("discussion:memo.participants"))!.split("\n")).toEqual([
      `- **Mina** — ${t("discussion:memo.moderator")} · Moderator · claude · opus · high`,
      `- **Ari** — ${t("discussion:memo.debater")} · Ideation · codex · gpt-5.5`,
      `- **${t("discussion:board.missingAgent")}** — ${t("discussion:memo.debater")} · ${t("discussion:board.noRole")} · pi`,
    ]);
    expect(memo.endsWith("\n")).toBe(true);
  });

  it("칸이 비면 적힌 것 없음으로 두고 발산 묶음은 빼며, 요약이 없으면 끝난 까닭을 결론 자리에 적는다", () => {
    const notice = t("discussion:run.notice.wrapTimeout");
    const memo = composeDecisionMemo({ ...d, context: "", cwd: undefined }, { notice, forced: "time" });
    const none = `_${t("discussion:memo.none")}_`;
    expect(section(memo, t("discussion:memo.conclusion"))).toBe(`_${notice}_`);
    for (const key of ["reasons", "rejected", "openIssues", "userChecks"]) expect(section(memo, t(`discussion:memo.${key}`))).toBe(none);
    expect(memo).not.toContain(`## ${t("discussion:memo.ideas")}`);
    expect(memo).not.toContain(`## ${t("discussion:memo.context")}`);
    expect(memo).toContain(t("discussion:memo.ending", { text: notice }));
    const forced = composeDecisionMemo(d, { summary: "Pulse.", forced: "rounds" });
    expect(forced).toContain(t("discussion:memo.endedForced", { reason: t("discussion:run.limitName.rounds") }));
  });

  it("메모 칸은 snake_case·camelCase와 문자열 하나도 받는다", () => {
    expect(parseWrapMemo({})).toEqual(emptyWrapMemo());
    expect(parseWrapMemo({ reasons: "One", rejected_alternatives: [{ alternative: "B", why: "slow" }], openIssues: [" x ", ""], user_checks: "ok?", ideas: ["Lone idea"] }))
      .toEqual({ reasons: ["One"], rejected: [{ option: "B", reason: "slow" }], openIssues: ["x"], userChecks: ["ok?"], ideas: [{ group: "", items: ["Lone idea"] }] });
  });
});
