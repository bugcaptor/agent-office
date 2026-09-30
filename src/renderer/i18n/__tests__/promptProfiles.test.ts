// 프롬프트와 머리말의 정합성, 출력 조립, 언어 선택·폴백을 검증한다.
import { afterAll, describe, expect, it } from "vitest";

import { SOURCE_LANGUAGE, i18n, initI18nForTest } from "@renderer/i18n";
import {
  diaryPromptProfile,
  hasMetaMarker,
  labelPromptProfile,
  runRecipePromptProfile,
  speechPromptProfile,
} from "../promptProfiles";

describe("수상 정보 조립", () => {
  it("월·활동 수치·수상 횟수를 한국어로 조립한다", () => {
    expect(
      speechPromptProfile("ko").formatAwardInfo({
        month: "2026-07",
        hours: 12,
        turns: 340,
        activeDays: 18,
        totalAwards: 2,
      }),
    ).toBe(
      ["월: 2026-07", "작업 시간: 약 12시간", "턴 수: 340", "활동일: 18일", "통산 수상: 2회(이번 포함)"].join(
        "\n",
      ),
    );
  });
});

describe("en 프로필", () => {
  it("ko의 번역이 아니라 언어에 맞게 조정된 값을 쓴다", () => {
    const label = labelPromptProfile("en");
    // 같은 정보에 영문이 글자를 더 먹으므로 폭주 감지선이 더 넉넉하다.
    expect(label.summaryMaxChars).toBeGreaterThan(labelPromptProfile("ko").summaryMaxChars);
    expect(label.contextMaxChars).toBeGreaterThan(labelPromptProfile("ko").contextMaxChars);
    // 길이 제약은 글자 수가 아니라 단어 수로 준다.
    expect(label.systemPrompt).toMatch(/at most \d+ words/);
    expect(label.systemPrompt).not.toMatch(/characters/);

    const speech = speechPromptProfile("en");
    expect(speech.speechMaxChars).toBeGreaterThan(speechPromptProfile("ko").speechMaxChars);
    // 프롬프트 예산만은 백엔드 cap_text에서 온 값이라 언어를 타지 않는다.
    expect(speech.promptBudgetChars).toBe(speechPromptProfile("ko").promptBudgetChars);
  });

  it("프롬프트가 자기 프로필의 머리말·자리 표시·폴백 문구를 실제로 가리킨다", () => {
    for (const lang of ["ko", "en"]) {
      const label = labelPromptProfile(lang);
      expect(label.systemPrompt).toContain(label.headers.prevGoal);
      expect(label.systemPrompt).toContain(label.headers.newInstruction);
      expect(label.systemPrompt).toContain(label.headers.context);
      expect(label.systemPrompt).toContain(label.noneText);
      expect(label.systemPrompt).toContain(label.fallbackText);

      const diary = diaryPromptProfile(lang);
      expect(diary.systemPrompt).toContain(diary.headers.personality);
      expect(diary.systemPrompt).toContain(diary.headers.workLog);

      const speech = speechPromptProfile(lang);
      expect(speech.systemPrompt).toContain(speech.headers.personality);
      expect(speech.systemPrompt).toContain(speech.headers.awardInfo);
      expect(speech.systemPrompt).toContain(speech.headers.diary);
    }
  });

  it("영어 프롬프트·자리 표시에 한글이 남아 있지 않다", () => {
    const hangul = /[가-힣]/;
    for (const p of [
      labelPromptProfile("en").systemPrompt,
      diaryPromptProfile("en").systemPrompt,
      speechPromptProfile("en").systemPrompt,
      labelPromptProfile("en").fallbackText,
      speechPromptProfile("en").noDiaryText,
      speechPromptProfile("en").formatAwardInfo({
        month: "2026-07",
        hours: 12,
        turns: 340,
        activeDays: 18,
        totalAwards: 2,
      }),
    ]) {
      expect(hangul.test(p)).toBe(false);
    }
  });

  it("머리말 제거 정규식이 영어 머리말을 대소문자 무관하게 잡는다", () => {
    const re = labelPromptProfile("en").linePrefixPattern;
    expect("Line 1: Fix login bug".replace(re, "")).toBe("Fix login bug");
    expect("line2: Fix tests".replace(re, "")).toBe("Fix tests");
    expect("Summary: Fix tests".replace(re, "")).toBe("Fix tests");
    expect("GOAL: Fix tests".replace(re, "")).toBe("Fix tests");
    // 머리말이 아닌 정상 라벨은 건드리지 않는다.
    expect("Goalkeeper sprite".replace(re, "")).toBe("Goalkeeper sprite");
  });
});

describe("hasMetaMarker", () => {
  it("대소문자를 무시한다(영어 거부는 대개 대문자로 시작한다)", () => {
    const en = labelPromptProfile("en").metaMarkers;
    expect(hasMetaMarker("Sorry, I cannot do that", en)).toBe(true);
    expect(hasMetaMarker("I'm sorry", en)).toBe(true);
    expect(hasMetaMarker("Encoding error", en)).toBe(true);
    expect(hasMetaMarker("Fix login bug", en)).toBe(false);
  });

  it("ko 마커 판정은 이행 전(s.includes)과 같다", () => {
    const ko = labelPromptProfile("ko").metaMarkers;
    expect(hasMetaMarker("죄송하지만 요약할 수 없습니다", ko)).toBe(true);
    expect(hasMetaMarker("인코딩 오류", ko)).toBe(true);
    expect(hasMetaMarker("로그인 버그 수정", ko)).toBe(false);
  });
});

describe("언어 폴백", () => {
  it("프로필이 없는 언어는 en으로 돈다", () => {
    // 카탈로그에 fr을 추가해도 프로필을 안 만들면 영어로 도는 것이 정상 동작이다.
    expect(labelPromptProfile("fr")).toBe(labelPromptProfile("en"));
    expect(diaryPromptProfile("fr")).toBe(diaryPromptProfile("en"));
    expect(speechPromptProfile("fr")).toBe(speechPromptProfile("en"));
    expect(runRecipePromptProfile("fr")).toBe(runRecipePromptProfile("en"));
  });

  it("지역 변종은 프리픽스로 좁힌다", () => {
    expect(labelPromptProfile("en-GB")).toBe(labelPromptProfile("en"));
    expect(labelPromptProfile("ko-KR")).toBe(labelPromptProfile("ko"));
    expect(runRecipePromptProfile("ko-KR")).toBe(runRecipePromptProfile("ko"));
  });
});

describe("실행 레시피 조사 프롬프트", () => {
  it("앱이 계산한 프로젝트와 파일 절대 경로를 넣고 실행 금지를 못박는다", () => {
    const prompt = runRecipePromptProfile("ko").formatProbePrompt(
      "/work/project",
      "/app/run-recipes/project.agent.json",
    );
    expect(prompt).toContain("/work/project");
    expect(prompt).toContain("/app/run-recipes/project.agent.json");
    expect(prompt).toContain("실제로 실행하지 말고");
    expect(prompt).toContain('"version":1');
  });

  it("영어 프로필에는 한글이 없다", () => {
    const prompt = runRecipePromptProfile("en").formatProbePrompt("/work/project", "/app/run.json");
    expect(prompt).not.toMatch(/[가-힣]/);
    expect(prompt).toContain("Do not execute any commands");
  });
});

describe("호출 시점 선택", () => {
  it("인자를 생략하면 지금 UI 언어를 따른다(모듈 로드 때 굳지 않는다)", async () => {
    expect(i18n.language).toBe(SOURCE_LANGUAGE); // test-setup이 못박은 정본
    expect(labelPromptProfile()).toBe(labelPromptProfile("ko"));

    await initI18nForTest("en");
    expect(labelPromptProfile()).toBe(labelPromptProfile("en"));
    expect(diaryPromptProfile()).toBe(diaryPromptProfile("en"));
    expect(speechPromptProfile()).toBe(speechPromptProfile("en"));
  });

  afterAll(async () => {
    await initI18nForTest(SOURCE_LANGUAGE); // 정본 복구(파일 간 언어 상태 누수 방지)
  });
});
