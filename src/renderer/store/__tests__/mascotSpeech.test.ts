// 마스코트 말풍선 텍스트 선정(store/mascotSpeech.ts).
import { describe, expect, it } from "vitest";
import { MASCOT_SPEECH_MAX_CHARS, pickMascotSpeech, toSpeechText } from "../mascotSpeech";

const done = (agentId: string, message: string) => ({ agentId, type: "done" as const, message });

describe("toSpeechText", () => {
  it("마크다운 표식·코드 펜스를 걷어내고 줄을 이어 붙인다", () => {
    expect(toSpeechText("## 결과\n\n- **빌드** 통과\n```sh\nnpm test\n```\n`a.ts` 고침")).toBe(
      "결과 빌드 통과 npm test a.ts 고침",
    );
  });

  it("빈 값은 null, 긴 값은 잘라 말줄임", () => {
    expect(toSpeechText(undefined)).toBeNull();
    expect(toSpeechText("  \n ``` \n")).toBeNull();
    const out = toSpeechText("가".repeat(200))!;
    expect(Array.from(out)).toHaveLength(MASCOT_SPEECH_MAX_CHARS + 1);
    expect(out.endsWith("…")).toBe(true);
  });
});

describe("pickMascotSpeech", () => {
  const base = { agentId: "a1", working: true, notifications: [], taskLabels: {} };

  it("작업 중이면 턴 중 내레이션", () => {
    expect(
      pickMascotSpeech({ ...base, taskLabels: { a1: { latestAssistantText: "테스트를 돌려 보겠습니다." } } }),
    ).toBe("테스트를 돌려 보겠습니다.");
  });

  it("작업 중이 아니고 알림도 없으면 null", () => {
    expect(
      pickMascotSpeech({ ...base, working: false, taskLabels: { a1: { latestAssistantText: "x" } } }),
    ).toBeNull();
  });

  it("최근 알림이 완료면 최종 응답이 내레이션보다 먼저", () => {
    expect(
      pickMascotSpeech({
        ...base,
        notifications: [done("a1", "다 고쳤습니다.")],
        taskLabels: { a1: { latestAssistantText: "중간 말" } },
      }),
    ).toBe("다 고쳤습니다.");
  });

  it("완료 폴백 문구와 질문 알림은 말하지 않는다", () => {
    expect(pickMascotSpeech({ ...base, notifications: [done("a1", "작업이 완료되었습니다.")] })).toBeNull();
    expect(
      pickMascotSpeech({
        ...base,
        notifications: [{ agentId: "a1", type: "question", message: "Claude needs your permission" }],
        taskLabels: { a1: { latestAssistantText: "중간 말" } },
      }),
    ).toBeNull();
  });

  it("다른 에이전트의 알림은 보지 않는다", () => {
    expect(
      pickMascotSpeech({
        ...base,
        notifications: [done("b2", "남의 말")],
        taskLabels: { a1: { latestAssistantText: "내 말" } },
      }),
    ).toBe("내 말");
  });
});
