// src/renderer/talk/__tests__/talkLogView.test.ts
//
// 대화 감사 로그의 순수 변환(대화 단위 묶기·참가자·표시 문구). DOM/스토어
// 의존이 없어 node 환경에서 돈다.
import { describe, expect, it } from "vitest";
import type { TalkLogEntry } from "@shared/types";
import { groupByConversation, kindLabel, participantsOf } from "../talkLogView";

function entry(over: Partial<TalkLogEntry> & { convId: string; at: number }): TalkLogEntry {
  return {
    kind: "send",
    id: `m${over.at}`,
    from: "a1",
    fromName: "하나",
    to: "a2",
    text: "본문",
    ...over,
  };
}

describe("groupByConversation", () => {
  it("뒤섞인 로그를 convId 단위로 묶고 각 묶음 안을 시간순으로 세운다", () => {
    const groups = groupByConversation([
      entry({ convId: "c2", at: 20 }),
      entry({ convId: "c1", at: 30 }),
      entry({ convId: "c1", at: 10 }),
      entry({ convId: "c2", at: 40 }),
    ]);
    expect(groups.map((g) => g.convId)).toEqual(["c1", "c2"]); // 시작 시각(10 < 20) 순
    expect(groups[0].startedAt).toBe(10);
    expect(groups[0].entries.map((e) => e.at)).toEqual([10, 30]);
    expect(groups[1].entries.map((e) => e.at)).toEqual([20, 40]);
  });

  it("같은 시각이면 파일 순서를 유지한다(안정 정렬)", () => {
    const groups = groupByConversation([
      entry({ convId: "c1", at: 5, id: "first" }),
      entry({ convId: "c1", at: 5, id: "second" }),
    ]);
    expect(groups[0].entries.map((e) => e.id)).toEqual(["first", "second"]);
  });

  it("빈 로그는 빈 묶음", () => {
    expect(groupByConversation([])).toEqual([]);
  });
});

describe("participantsOf", () => {
  it("등장 순으로 중복 없이 모으고, 받는 쪽 이름은 resolver로 푼다", () => {
    const [g] = groupByConversation([
      entry({ convId: "c1", at: 1, from: "a1", fromName: "하나", to: "a2" }),
      entry({ convId: "c1", at: 2, from: "a2", fromName: "두리", to: "a1" }),
    ]);
    expect(participantsOf(g, (id) => (id === "a2" ? "두리" : "하나"))).toEqual(["하나", "두리"]);
  });

  it("모르는 agentId는 id를 그대로 쓰는 resolver를 그대로 존중한다", () => {
    const [g] = groupByConversation([entry({ convId: "c1", at: 1, to: "gone" })]);
    expect(participantsOf(g, (id) => id)).toEqual(["하나", "gone"]);
  });
});

describe("kindLabel", () => {
  it("알려진 종류는 한국어로, 모르는 값은 그대로", () => {
    expect(kindLabel("send")).toBe("말함");
    expect(kindLabel("deliver")).toBe("전달됨");
    expect(kindLabel("expire")).toBe("전달 실패(만료)");
    expect(kindLabel("future-kind")).toBe("future-kind");
  });
});
