// src/renderer/labels/__tests__/labelText.test.ts
import { describe, expect, it } from "vitest";
import {
  deriveTaskLabelLines,
  firstLine,
  projectNameFromCwd,
  requestSentence,
  truncateChars,
} from "../labelText";
import type { AgentTaskLabel } from "../../store/types";

describe("projectNameFromCwd", () => {
  it("basename을 돌려준다 (POSIX)", () => {
    expect(projectNameFromCwd("/Users/me/dev/agent-office")).toBe("agent-office");
  });
  it("트레일링 슬래시를 무시한다", () => {
    expect(projectNameFromCwd("/Users/me/dev/agent-office/")).toBe("agent-office");
  });
  it("윈도우 구분자도 처리한다", () => {
    expect(projectNameFromCwd("C:\\dev\\my-proj")).toBe("my-proj");
  });
  it("빈/undefined/루트는 undefined", () => {
    expect(projectNameFromCwd(undefined)).toBeUndefined();
    expect(projectNameFromCwd("")).toBeUndefined();
    expect(projectNameFromCwd("/")).toBeUndefined();
  });
});

describe("firstLine", () => {
  it("첫 줄만 취해 max 초과 시 …로 절단한다", () => {
    expect(firstLine("버그를 고쳐줘\n그리고 테스트도", 30)).toBe("버그를 고쳐줘");
    expect(firstLine("아주 긴 지시문입니다 정말로 깁니다", 8)).toBe("아주 긴 지시문…");
  });
  it("빈/공백뿐/undefined는 undefined", () => {
    expect(firstLine(undefined, 10)).toBeUndefined();
    expect(firstLine("   \n  ", 10)).toBeUndefined();
  });
});

describe("truncateChars", () => {
  it("max 이하면 그대로, 초과면 …로 절단한다(멀티바이트 안전)", () => {
    expect(truncateChars("버그 수정", 24)).toBe("버그 수정");
    expect(truncateChars("아주 긴 지시문입니다", 4)).toBe("아주 긴…");
    expect(truncateChars("", 24)).toBe("");
  });
});

describe("requestSentence", () => {
  it("맥락 서술 뒤 요청 어미로 끝나는 조각을 고른다", () => {
    expect(
      requestSentence(
        "43번 이슈가 최신에 반영되긴 했는데, 첫째줄에 표시되는 것은 여전히 프롬프트야. 프롬프트가 충분히 현재 작업의 목적을 나타내게 하고 싶은데 아이디에이션 해보자. 44번 이슈에 코멘트해."
      )
    ).toBe("44번 이슈에 코멘트해");
  });

  it("단일 명령문은 그대로 돌려준다", () => {
    expect(requestSentence("로그인 버그 고쳐줘")).toBe("로그인 버그 고쳐줘");
  });

  it("소망(좋겠다) 뒤 명령(추가해)이 오면 뒤쪽 명령을 고른다", () => {
    expect(
      requestSentence(
        "고치는 김에 터미널 열었을 때도 요약을 볼 수 있으면 좋겠다. 그에 관한 아이디어도 추가해"
      )
    ).toBe("그에 관한 아이디어도 추가해");
  });

  it("동점이면 마지막(뒤쪽) 조각을 고른다", () => {
    // 둘 다 요청 어미(주세요/해줘) → 뒤쪽.
    expect(requestSentence("먼저 확인해주세요. 그다음 배포해줘")).toBe("그다음 배포해줘");
  });

  it("요청 어미가 하나도 없으면 마지막 조각", () => {
    expect(requestSentence("첫 문장이다. 두 번째 문장이다")).toBe("두 번째 문장이다");
  });

  it("인삿말뿐인 조각은 후보에서 뺀다", () => {
    expect(requestSentence("안녕하세요. 로그인 버그 고쳐줘")).toBe("로그인 버그 고쳐줘");
  });

  it("undefined/공백만/부호만은 undefined", () => {
    expect(requestSentence(undefined)).toBeUndefined();
    expect(requestSentence("   \n  ")).toBeUndefined();
    expect(requestSentence("...!!")).toBeUndefined();
  });
});

describe("deriveTaskLabelLines", () => {
  const opts = { goalMax: 24, currentMax: 30 };
  function label(patch: Partial<AgentTaskLabel>): AgentTaskLabel {
    return { sessionId: "s1", ...patch };
  }

  it("LLM 목표와 실황을 프로젝트명과 함께 두 줄로 파생한다", () => {
    const { line1, line2 } = deriveTaskLabelLines(
      label({ cwd: "/w/agent-office", goal: "버그 수정", latestAssistantText: "원인 좁히는 중" }),
      undefined,
      opts
    );
    expect(line1).toBe("agent-office · 버그 수정");
    expect(line2).toBe("원인 좁히는 중");
  });

  it("목표 폴백 체인: goal > goalFallback > firstPromptText 요청 문장", () => {
    // goal 부재 → goalFallback 사용.
    expect(
      deriveTaskLabelLines(label({ cwd: "/w/proj", goalFallback: "로그인 고쳐줘" }), undefined, opts)
        .line1
    ).toBe("proj · 로그인 고쳐줘");
    // goal·goalFallback 부재 → firstPromptText의 요청 문장.
    expect(
      deriveTaskLabelLines(
        label({ cwd: "/w/proj", firstPromptText: "맥락 설명. 로그인 고쳐줘" }),
        undefined,
        opts
      ).line1
    ).toBe("proj · 로그인 고쳐줘");
  });

  it("cwd 폴백: 라벨 cwd 부재 시 fallbackCwd(프로필 cwd)를 쓴다", () => {
    // 라벨 cwd가 있으면 그게 이긴다.
    expect(
      deriveTaskLabelLines(label({ cwd: "/w/other-proj", goal: "작업" }), "/w/profile-proj", opts)
        .line1
    ).toBe("other-proj · 작업");
    // 라벨 cwd 부재 → 폴백.
    expect(deriveTaskLabelLines(label({ goal: "작업" }), "/w/profile-proj", opts).line1).toBe(
      "profile-proj · 작업"
    );
  });

  it("실황 우선순위: assistant > tool > currentSummary > 최신 프롬프트 요청 문장", () => {
    const src = {
      currentSummary: "버그 고치는 중",
      latestToolText: "Bash: npm test",
      latestAssistantText: "원인을 좁히는 중",
      latestPromptText: "테스트 추가해줘",
    };
    expect(deriveTaskLabelLines(label(src), undefined, opts).line2).toBe("원인을 좁히는 중");
    expect(
      deriveTaskLabelLines(label({ ...src, latestAssistantText: undefined }), undefined, opts).line2
    ).toBe("Bash: npm test");
    expect(
      deriveTaskLabelLines(
        label({ ...src, latestAssistantText: undefined, latestToolText: undefined }),
        undefined,
        opts
      ).line2
    ).toBe("버그 고치는 중");
    expect(
      deriveTaskLabelLines(label({ latestPromptText: "테스트 추가해줘" }), undefined, opts).line2
    ).toBe("테스트 추가해줘");
  });

  it("절단 폭은 opts를 따른다(실황/폴백만 절단, LLM goal은 통과)", () => {
    const src = label({ goal: "아주길고긴목표문장입니다정말로", latestAssistantText: "원인을좁히는중입니다정말로" });
    const { line1, line2 } = deriveTaskLabelLines(src, undefined, { goalMax: 5, currentMax: 6 });
    // goal은 이미 있으면 절단하지 않는다(LLM 요약은 그대로) — 폴백만 절단 대상.
    expect(line1).toBe("아주길고긴목표문장입니다정말로");
    expect(line2).toBe("원인을좁히는…");
  });

  it("빈 라벨/undefined는 두 줄 모두 undefined", () => {
    expect(deriveTaskLabelLines(undefined, undefined, opts)).toEqual({
      line1: undefined,
      line2: undefined,
    });
    // cwd만 있으면 line1은 프로젝트명, line2는 undefined.
    expect(deriveTaskLabelLines(undefined, "/w/proj", opts)).toEqual({
      line1: "proj",
      line2: undefined,
    });
  });
});
