// src/renderer/i18n/__tests__/promptProfiles.test.ts
//
// 프롬프트 프로필의 두 가지를 지킨다.
//
// 1) **ko 동결**: Phase 5는 프롬프트를 `promptProfiles.ts`로 옮기기만 한
//    리팩터링이고 튜닝이 아니다. 프롬프트 한 글자가 요약·일기·소감의 품질을
//    바꾸므로, 이행 전 문자열의 **사본**을 여기 박아 두고 바이트 비교한다.
//    딸린 숫자·마커도 같이 못박는다. 이 테스트가 깨지면 둘 중 하나다 —
//    실수로 프롬프트를 건드렸거나(되돌려라), 의도한 튜닝이거나(그 커밋에서
//    아래 사본도 함께 갱신하고 커밋 메시지에 품질 변경임을 남겨라).
//
// 2) **폴백**: 프로필이 없는 언어는 en으로 돈다(모듈 주석의 규칙).
import { afterAll, describe, expect, it } from "vitest";

import { SOURCE_LANGUAGE, i18n, initI18nForTest } from "@renderer/i18n";
import {
  diaryPromptProfile,
  hasMetaMarker,
  labelPromptProfile,
  speechPromptProfile,
} from "../promptProfiles";

// ---------------------------------------------------------------------------
// Phase 5 이행 직전(HEAD=dbf428d)의 ko 프롬프트 사본. **손대지 마라** —
// 여기 값을 고치는 것은 프롬프트를 고치는 것과 같은 무게의 결정이다.
// ---------------------------------------------------------------------------
const FROZEN_KO_LABEL_PROMPT = "너는 코딩 세션 라벨 생성기다. [이전 목표], [새 지시], 그리고 있을 경우 [초기 작업 정황]을 보고 정확히 두 줄을 출력하라. 1줄: 세션 목표(한국어 명사구 12자 이내). [초기 작업 정황]이 있으면 그것이 이 세션이 실제로 무엇을 하는지 보여주는 근거이므로, 이슈·티켓 번호만 가리키는 모호한 지시(예: '이슈 40 해결')보다 우선해 목표를 구체화하라. [초기 작업 정황]이 없으면: 새 지시가 새로운 작업이면 새로 뽑고, 이전 작업의 후속·보완 지시이거나 판단이 애매하면 이전 목표를 그대로 출력하라. 이전 목표가 (없음)이면 새로 뽑아라. 2줄: 새 지시 요약 — 한국어 18자 이내 한 줄. 규칙: 정확히 두 줄, 한국어만, 사과·설명·따옴표·번호·머리말 금지. 판단 불가면 1줄은 이전 목표(없으면 '작업 중'), 2줄은 '작업 중'. 예) 이전 목표: 로그인 버그 수정 / 새 지시: 테스트도 고쳐줘 → 1줄 '로그인 버그 수정', 2줄 '테스트 수정'. 예) 이전 목표: (없음) / 새 지시: 이슈 40 해결 / 초기 작업 정황: Claude 훅 설정 파일을 복구하는 중 → 1줄 '훅 설정 복구', 2줄 '이슈 40 해결'";
const FROZEN_KO_DIARY_PROMPT = "너는 한 캐릭터의 일기 작성기다. 아래 [성격]을 문체로 삼아, [작업 로그]를 1인칭 한국어 일기 한 편으로 써라. 성격에 따라 초등학생 일기처럼 쓰기도 하고 차가운 작업 일지처럼 쓰기도 한다 — [성격]의 말투·태도를 문체에 그대로 반영하라. [성격]이 비어 있으면 담백한 중립 문체로 써라. 반드시 실제로 한 일(수정한 파일·실행한 명령·목표)이 드러나야 한다(작업 로그를 겸한다). 분량은 3~8문장. 규칙: 한국어만, 사과·메타발언·머리말·따옴표·마크다운 금지, 일기 본문만 출력.";
const FROZEN_KO_SPEECH_PROMPT = "너는 사내 시상식에서 '이 달의 우수사원'으로 호명된 캐릭터 본인이다. 아래 [성격]을 문체로 삼아, [수상 정보]와 [지난달 일기]를 근거로 1인칭 한국어 수상 소감을 써라. 분량은 2~4문장, 200자 이내. [수상 정보]의 수치 하나쯤은 자연스럽게 녹여도 좋지만 통계를 나열하지 마라. [성격]이 비어 있으면 담백한 중립 문체로 써라. 규칙: 한국어만, 사과·메타발언·머리말·따옴표·마크다운 금지, 소감 본문만 출력.";

describe("ko 프롬프트 동결", () => {
  it("라벨 요약 시스템 프롬프트가 이행 전과 바이트 단위로 같다", () => {
    expect(labelPromptProfile("ko").systemPrompt).toBe(FROZEN_KO_LABEL_PROMPT);
  });

  it("일기 시스템 프롬프트가 이행 전과 바이트 단위로 같다", () => {
    expect(diaryPromptProfile("ko").systemPrompt).toBe(FROZEN_KO_DIARY_PROMPT);
  });

  it("수상 소감 시스템 프롬프트가 이행 전과 바이트 단위로 같다", () => {
    expect(speechPromptProfile("ko").systemPrompt).toBe(FROZEN_KO_SPEECH_PROMPT);
  });
});

describe("ko 파라미터 동결", () => {
  it("라벨 요약 상수가 이행 전 값 그대로다", () => {
    const p = labelPromptProfile("ko");
    expect(p.summaryMaxChars).toBe(40);
    expect(p.contextMaxChars).toBe(120);
    expect(p.metaMarkers).toEqual(["인코딩", "죄송", "할 수 없"]);
    expect(p.fallbackText).toBe("작업 중");
    expect(p.noneText).toBe("(없음)");
    expect(p.headers).toEqual({
      prevGoal: "[이전 목표]",
      newInstruction: "[새 지시]",
      context: "[초기 작업 정황]",
    });
    // 이행 전 sanitizeLine에 인라인돼 있던 정규식 그대로.
    expect(p.linePrefixPattern.source).toBe(String.raw`^(1줄|2줄|요약|목표)\s*[:：]\s*`);
    expect(p.linePrefixPattern.flags).toBe("");
  });

  it("일기 상수가 이행 전 값 그대로다", () => {
    const p = diaryPromptProfile("ko");
    expect(p.bodyMinChars).toBe(4);
    expect(p.noneText).toBe("(없음)");
    expect(p.headers).toEqual({ personality: "[성격]", workLog: "[작업 로그]" });
  });

  it("수상 소감 상수가 이행 전 값 그대로다", () => {
    const p = speechPromptProfile("ko");
    expect(p.promptBudgetChars).toBe(1_900);
    expect(p.personalityMaxChars).toBe(300);
    expect(p.speechMaxChars).toBe(240);
    expect(p.speechMaxSentences).toBe(4);
    expect(p.excerptLimits).toEqual({ maxEntries: 10, perEntryChars: 200, totalChars: 1_500 });
    expect(p.excerptMinBodyChars).toBe(60);
    expect(p.noDiaryText).toBe("(일기 없음)");
    expect(p.noneText).toBe("(없음)");
    expect(p.headers).toEqual({
      personality: "[성격]",
      awardInfo: "[수상 정보]",
      diary: "[지난달 일기]",
    });
  });

  it("[수상 정보] 블록 조립이 이행 전 문자열 그대로다", () => {
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
  });

  it("지역 변종은 프리픽스로 좁힌다", () => {
    expect(labelPromptProfile("en-GB")).toBe(labelPromptProfile("en"));
    expect(labelPromptProfile("ko-KR")).toBe(labelPromptProfile("ko"));
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
