import { describe, expect, it } from "vitest";
import type { Discussion } from "@shared/types";
import { boardAge, EXCERPT_CHARS, meetingBoardRows, utteranceExcerpt } from "../meetingBoardModel";

const now = 1_800_000_000_000;
function discussion(patch: Partial<Discussion> = {}): Discussion {
  return {
    id: "d1", topic: "점심", context: "", moderatorAgentId: "m",
    participants: [
      { agentId: "a", agent: { cli: "claude" } },
      { agentId: "m", agent: { cli: "claude" } },
      { agentId: "b", agent: { cli: "codex" } },
    ],
    limits: { maxRounds: 4, maxMinutes: 30, maxCostUsd: 5, usageGuard: true }, state: "running", round: 1, transcript: [],
    costUsd: 0, createdAt: now, updatedAt: now, ...patch,
  };
}

describe("meetingBoardRows", () => {
  it("의자 순서(사회자 먼저)로 참가자마다 최근 공개 발언과 말하는 중을 낸다", () => {
    const rows = meetingBoardRows(discussion({
      transcript: [
        { at: 1, round: 1, kind: "opening", agentId: "a", text: "첫 의견" },
        { at: 2, round: 1, kind: "opening", agentId: "b", text: "국밥" },
        { at: 3, round: 2, kind: "moderator", agentId: "m", text: "- a: 예산은?" },
        { at: 4, round: 2, kind: "statement", agentId: "a", text: "만 원" },
        { at: 5, round: 2, kind: "notice", text: "알림" },
      ],
      run: { phase: "moderator", seen: {}, turns: [{ agentId: "m", sentAt: 6 }] },
    }));
    expect(rows.map((r) => [r.agentId, r.moderator, r.speakingSince, r.last?.at, r.last?.excerpt])).toEqual([
      ["m", true, 6, 3, "→ a: 예산은?"],
      ["a", false, undefined, 4, "만 원"],
      ["b", false, undefined, 2, "국밥"],
    ]);
  });

  it("동시 첫 라운드에서 받았지만 공개 전인 발언은 보이지 않는다", () => {
    const rows = meetingBoardRows(discussion({
      run: { phase: "opening", seen: {}, turns: [{ agentId: "a", sentAt: 1, text: "비공개" }, { agentId: "b", sentAt: 1 }] },
    }));
    expect(rows.map((r) => [r.agentId, r.speakingSince, r.last])).toEqual([
      ["m", undefined, undefined],
      ["a", undefined, undefined],
      ["b", 1, undefined],
    ]);
  });

  it("멈춘 토의는 차례가 남아 있어도 말하는 중이 아니다", () => {
    const rows = meetingBoardRows(discussion({ state: "paused", run: { phase: "moderator", seen: {}, turns: [{ agentId: "m", sentAt: 6 }] } }));
    expect(rows.every((r) => r.speakingSince === undefined)).toBe(true);
  });
});

describe("utteranceExcerpt", () => {
  it("사회자 지목은 화살표로 잇는다", () => {
    expect(utteranceExcerpt({ kind: "moderator", text: "- 가: 예산?\n- 나: 시간?" }, 80)).toBe("→ 가: 예산? / → 나: 시간?");
  });

  it("정리처럼 지목이 아닌 사회자 발언과 일반 발언은 마크다운 기호를 걷고 한 줄로 편다", () => {
    expect(utteranceExcerpt({ kind: "moderator", text: "## 결론\n- **국밥**으로 간다" }, 80)).toBe("결론 국밥으로 간다");
    expect(utteranceExcerpt({ kind: "statement", text: "> 인용\n1. `첫째`\n\n  둘째   줄" }, 80)).toBe("인용 첫째 둘째 줄");
  });

  it("길면 글자 단위로 자르고 말줄임을 붙인다", () => {
    const out = utteranceExcerpt({ kind: "statement", text: "가".repeat(EXCERPT_CHARS + 3) }, EXCERPT_CHARS);
    expect([...out]).toHaveLength(EXCERPT_CHARS + 1);
    expect(out.endsWith("…")).toBe(true);
  });
});

describe("boardAge", () => {
  it("1분 미만은 방금, 60분부터는 시간, 미래 시각은 방금", () => {
    expect(boardAge(now - 59_999, now)).toEqual({ unit: "now" });
    expect(boardAge(now - 60_000, now)).toEqual({ unit: "minutes", count: 1 });
    expect(boardAge(now - 59 * 60_000, now)).toEqual({ unit: "minutes", count: 59 });
    expect(boardAge(now - 125 * 60_000, now)).toEqual({ unit: "hours", count: 2 });
    expect(boardAge(now + 5_000, now)).toEqual({ unit: "now" });
  });
});
