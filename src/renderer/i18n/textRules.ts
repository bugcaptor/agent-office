// src/renderer/i18n/textRules.ts
//
// **사용자가 친 텍스트를 판정하는 언어별 규칙**. 화면에 나가는 문구도(→ 카탈로그),
// AI에게 보내는 프롬프트도(→ promptProfiles.ts) 아니고, "이 문장이 목적을 담고
// 있나", "이건 그냥 맞장구인가"처럼 **언어의 표기 습관에 매인 판정 상수**다.
// 그래서 세 번째 자리로 여기 둔다(wordlists.ts와 같은 "언어별 데이터 + 폴백" 관례).
//
// 폴백 규칙은 promptProfiles.ts와 같다: 정확 일치 → 프리픽스 일치(`en-GB` → `en`)
// → FALLBACK_LANGUAGE → SOURCE_LANGUAGE. 규칙이 없는 언어는 영어 규칙으로 돈다.
// (`src/renderer/i18n/` 아래는 하드코딩 한글 스캐너의 제외 경로다.)
import { FALLBACK_LANGUAGE, SOURCE_LANGUAGE } from "@shared/i18n/catalog";

import { i18n } from "./index";

/** 한 언어의 텍스트 판정 규칙. */
export interface TextRules {
  /**
   * 맞장구성 지시 판정: 이 토큰으로 **시작**하고 뒤에 토큰 경계가 올 때만 참.
   *
   * 경계를 요구하는 이유(이슈 #44 작업 A): 한국어는 "네"가 "네트워크"의
   * 앞머리이기도 해서 경계 없이 접두 일치만 보면 "네트워크 설정 고쳐줘"가
   * 맞장구로 오분류된다. 영어도 같은 성질이 필요하다 — "ok"는 "okra"의,
   * "sure"는 "surefire"의 앞머리다. 한국어에는 `\b`가 쓸모없어(한글 사이에
   * 단어 경계가 생기지 않는다) 명시적 구두점 lookahead를 쓰고, 영어에서는
   * `\b`가 정확히 같은 일을 하므로 `\b`를 쓴다.
   */
  backchannelStart: RegExp;
  /**
   * goalFallback 갱신 최소 글자 수 — 이보다 짧은 요청 문장은 목적을 담기
   * 어렵다(이슈 #44). 한글은 한 글자가 한 음절이라 6자면 두세 낱말이지만,
   * 영문 6자는 낱말 하나도 안 되므로 언어마다 값이 다르다.
   */
  goalFallbackMinChars: number;
  /**
   * "이 조각이 요청 문장인가" 판정(하나라도 맞으면 가점). 끝 부호를 뗀
   * 조각 전체에 대고 test한다 — **닻(`^`/`$`)은 각 정규식이 스스로 갖는다.**
   *
   * 언어마다 닻이 반대인 게 이 필드의 요점이다: 한국어는 요청이 어미에
   * 실리므로(`…해줘`, `…하고 싶다`) `$`에 붙이고, 영어는 명령문이 동사로
   * 시작하거나 `please`/`can you`로 열리므로 `^`에 붙인다. 그래서 "어미 목록"
   * 같은 이름을 쓰지 않고, 판정 규칙 묶음으로 둔다.
   */
  requestPatterns: RegExp[];
  /** 인삿말 시작 — `greetingMaxChars` 이하로 짧으면 요청 후보에서 뺀다. */
  greetingStart: RegExp;
  /** 인삿말뿐인 조각으로 볼 최대 글자 수(한글 1자 ≈ 영문 2자). */
  greetingMaxChars: number;
}

const TEXT_RULES: Record<string, TextRules> = {
  ko: {
    backchannelStart: /^(응|네|넵|예|그래|좋아|오케이|오케|ㅇㅋ|알겠|고마|감사)(?=[\s,.!?~…]|$)/,
    goalFallbackMinChars: 6,
    // 명령·요청·소망 어미. 과도한 열거보다 소수의 견고한 정규식으로 다듬는다
    // (이슈 #44 작업 A). labelText.ts에서 옮겨 온 것으로, 내용은 그대로다.
    requestPatterns: [
      /줘$/, // 해줘·고쳐줘
      /주세요$/, // 해주세요
      /(해라|하라)$/, // 해라·하라
      /(하자|합시다)$/, // 하자·합시다
      /할\s?것$/, // 할 것·할것
      /해야\s?(해|한다|함)$/, // 해야 해/한다/함
      /[가-힣]해$/, // 반말 명령: 코멘트해·추가해
      /(하고\s?싶|좋겠)[가-힣]*$/, // 소망: 하고 싶다·좋겠다
    ],
    greetingStart: /^(안녕|하이|헬로|반가|hi|hello)/i,
    greetingMaxChars: 12,
  },
  en: {
    // ko 목록의 성격을 옮긴 것이지 낱말 대응이 아니다. "good"처럼 명령문의
    // 첫 낱말로도 자연스러운 말("good, now…" vs "good first issue를 …")은
    // 오탐이 크므로 뺐고, 대신 확실한 맞장구 정형구만 담았다.
    // `i` 플래그: 영어는 "OK"/"Ok"/"ok"가 모두 흔하다(한글은 대소문자가 없어
    // ko 규칙에는 불필요).
    backchannelStart:
      /^(ok|okay|kk|yes|yeah|yep|yup|sure|alright|got it|will do|sounds good|looks good|thanks|thank you|thx|ty)\b/i,
    // 한글 6자에 담기는 정보는 영문으로 대략 두 배의 글자를 먹는다.
    goalFallbackMinChars: 12,
    // 영어 요청은 **문두**에 실린다 — ko 목록을 낱말 대응으로 옮기는 게 아니라
    // 닻을 뒤집어 다시 설계한 것이다. 정중형(please·can you)·청유형(let's)·
    // 희망형(I want/need/'d like)과, 개발 대화에서 실제로 명령문을 여는 동사들.
    // 동사 목록을 무한히 늘리지 않는 이유는 ko와 같다: 오탐이 미탐보다 나쁘다
    // (가점을 못 받아도 "마지막 조각" 규칙이 여전히 답을 낸다).
    requestPatterns: [
      /^(please|pls)\b/i,
      /^(can|could|would|will) you\b/i,
      /^let'?s\b/i,
      /^i (want|need)\b/i,
      /^i'?d like\b/i,
      // 명령문 앞에 흔히 붙는 이음말은 건너뛰고 동사를 본다 — "Then deploy the
      // build"는 누가 봐도 요청인데 문두 닻만 보면 놓친다. ko에는 대응물이
      // 없다(어미 판정이라 문두에 뭐가 오든 상관없다).
      /^(?:(?:then|next|also|now|first|finally|after that)[,\s]+)?(add|build|change|check|clean|create|debug|delete|deploy|document|drop|explain|extract|find|fix|handle|implement|improve|install|investigate|make|migrate|move|optimi[sz]e|refactor|remove|rename|replace|review|revert|run|set ?up|support|test|update|write)\b/i,
    ],
    greetingStart: /^(hi|hello|hey|yo|good (morning|afternoon|evening))\b/i,
    greetingMaxChars: 24,
  },
};

/** 언어 코드에 맞는 규칙. 없으면 프리픽스 → 폴백(en) → 정본 순. */
export function textRulesFor(lang: string | null | undefined): TextRules {
  const code = (lang ?? "").toLowerCase();
  return (
    TEXT_RULES[code] ??
    TEXT_RULES[code.split("-")[0]] ??
    TEXT_RULES[FALLBACK_LANGUAGE] ??
    TEXT_RULES[SOURCE_LANGUAGE]
  );
}

/** 지금 UI 언어의 규칙. **호출 시점에** 고른다 — 모듈 로드 때 굳히면 언어를
 *  바꿔도 옛 규칙이 계속 나온다. */
export function currentTextRules(): TextRules {
  return textRulesFor(i18n.language);
}
