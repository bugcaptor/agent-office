// src/renderer/i18n/__tests__/textRules.test.ts
//
// 언어별 텍스트 판정 규칙. 핵심은 **토큰 경계**다 — 접두 일치만 보면 ko는
// "네트워크", en은 "okra"가 맞장구로 오분류된다(이슈 #44 작업 A의 이유).
import { afterAll, describe, expect, it } from "vitest";

import { SOURCE_LANGUAGE, initI18nForTest } from "@renderer/i18n";
import { currentTextRules, textRulesFor } from "../textRules";

describe("ko 규칙 동결", () => {
  const ko = textRulesFor("ko");

  it("이행 전 값 그대로다", () => {
    expect(ko.goalFallbackMinChars).toBe(6);
    expect(ko.backchannelStart.source).toBe(
      String.raw`^(응|네|넵|예|그래|좋아|오케이|오케|ㅇㅋ|알겠|고마|감사)(?=[\s,.!?~…]|$)`,
    );
    expect(ko.backchannelStart.flags).toBe("");
  });

  it("맞장구를 잡고 토큰 경계로 오탐을 막는다", () => {
    expect(ko.backchannelStart.test("네")).toBe(true);
    expect(ko.backchannelStart.test("응 그래")).toBe(true);
    // 경계를 요구하므로 "감사합니다"는 잡히지 않는다(짧아서 길이 조건에 걸린다).
    expect(ko.backchannelStart.test("감사합니다")).toBe(false);
    expect(ko.backchannelStart.test("네트워크 설정 고쳐줘")).toBe(false);
    expect(ko.backchannelStart.test("예약 화면 고쳐줘")).toBe(false);
  });
});

describe("en 규칙", () => {
  const en = textRulesFor("en");

  it("영문은 글자 밀도가 낮아 목표 폴백 하한이 더 크다", () => {
    expect(en.goalFallbackMinChars).toBeGreaterThan(textRulesFor("ko").goalFallbackMinChars);
  });

  it("대소문자를 무시하고 맞장구를 잡는다", () => {
    for (const s of ["ok", "OK", "Okay", "yes", "Yeah", "sure", "Thanks!", "thank you", "got it"]) {
      expect(en.backchannelStart.test(s)).toBe(true);
    }
  });

  it("단어 경계(\\b)로 부분 일치 오탐을 막는다 — ko의 lookahead와 같은 성질", () => {
    for (const s of [
      "okra parser fix",
      "surefire plugin upgrade",
      "yesterday's regression",
      "typescript config cleanup", // "ty"로 시작
      "kkonfig rename",
    ]) {
      expect(en.backchannelStart.test(s)).toBe(false);
    }
  });

  it("판정 대상은 '시작 토큰'뿐이다 — 뒤에 지시가 붙어도 시작은 맞장구다", () => {
    // ko도 같다("응 그래 로그인 고쳐줘"). 실제 목표 폴백 갱신 여부는
    // appStore의 isMeaningfulGoalFallback이 길이 조건과 AND로 묶어 정한다.
    expect(en.backchannelStart.test("ok, now fix the login bug")).toBe(true);
  });
});

describe("언어 폴백", () => {
  it("규칙이 없는 언어는 en으로 돈다", () => {
    expect(textRulesFor("fr")).toBe(textRulesFor("en"));
    expect(textRulesFor("en-GB")).toBe(textRulesFor("en"));
    expect(textRulesFor("ko-KR")).toBe(textRulesFor("ko"));
  });

  it("currentTextRules는 호출 시점의 UI 언어를 따른다", async () => {
    expect(currentTextRules()).toBe(textRulesFor(SOURCE_LANGUAGE));
    await initI18nForTest("en");
    expect(currentTextRules()).toBe(textRulesFor("en"));
  });

  afterAll(async () => {
    await initI18nForTest(SOURCE_LANGUAGE); // 정본 복구
  });
});
