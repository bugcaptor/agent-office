// src/renderer/i18n/wordlists.ts
//
// 랜덤 프로필 초안(generate.ts)이 쓰는 **언어별 낱말 데이터**. 순수 데이터 —
// 로직은 고르는 함수 셋뿐이다.
//
// 카탈로그 JSON이 아니라 여기 두는 이유: 이건 UI 라벨이 아니라 "그 언어권에서
// 그럴듯한 캐릭터 이름/역할/성격"이라는 **언어별 자료**다. 키 대 키로 대응하지
// 않고(한국어 "야근요정"의 영어 번역이 있을 리 없다) 항목 수도 언어마다 달라질
// 수 있으므로, 키 집합 파리티를 요구하는 카탈로그에 넣으면 맞지 않는다.
// (`src/renderer/i18n/` 아래는 하드코딩 한글 스캐너의 제외 경로다.)
import { FALLBACK_LANGUAGE, SOURCE_LANGUAGE } from "@shared/i18n/catalog";

import { i18n } from "./index";

/** 한 언어의 낱말 묶음. 셋 다 같은 성격의 목록이고 길이는 서로 달라도 된다. */
export interface WordLists {
  /** 이름: 픽셀 캐릭터에 어울리는 별명. */
  names: string[];
  /** 역할: 직군 이름. */
  roles: string[];
  /** 성격: 성격 프롬프트의 앞머리에 붙는 수식어. */
  personalities: string[];
}

/**
 * 언어 코드 → 낱말 묶음. 카탈로그와 달리 **여기 없는 언어는 폴백(en)**을 쓴다 —
 * 번역이 아니라 자료라서 "빠진 항목"이라는 개념이 없기 때문이다.
 */
export const WORDLISTS: Record<string, WordLists> = {
  ko: {
    names: [
      "방구석코더", "야근요정", "카페인킴", "버그사냥꾼", "리팩토리",
      "깃발든자", "무한루프", "세미콜론", "널포인터", "스택오버",
      "컴파일러", "핫픽스박", "메모리조", "스레드리", "캐시최",
      "로그남작", "픽셀공주", "터미널곰", "주니어양", "시니어형",
    ],
    roles: [
      "프론트엔드", "백엔드", "데브옵스", "QA엔지니어", "풀스택",
      "AI리서처", "데이터분석", "보안담당", "PM", "테크리드",
      "UX디자이너", "SRE", "모바일개발", "게임개발", "임베디드",
      "플랫폼", "인프라", "DBA", "ML엔지니어", "아키텍트",
    ],
    personalities: [
      "꼼꼼한", "느긋한", "열정적인", "침착한", "엉뚱한",
      "완벽주의", "낙천적인", "분석적인", "창의적인", "집요한",
      "수다스러운", "과묵한", "호기심많은", "신중한", "대담한",
      "유쾌한", "까칠한", "성실한", "즉흥적인", "전략적인",
    ],
  },
  en: {
    names: [
      "CouchCoder", "CrunchFairy", "CaffeineKim", "BugHunter", "Refactory",
      "FlagBearer", "InfiniteLoop", "Semicolon", "NullPointer", "StackOver",
      "Compiler", "HotfixPark", "MemoryCho", "ThreadLee", "CacheChoi",
      "LogBaron", "PixelPrincess", "TerminalBear", "JuniorYang", "SeniorHyung",
    ],
    roles: [
      "Frontend", "Backend", "DevOps", "QA Engineer", "Full-stack",
      "AI Researcher", "Data Analyst", "Security", "PM", "Tech Lead",
      "UX Designer", "SRE", "Mobile Dev", "Game Dev", "Embedded",
      "Platform", "Infra", "DBA", "ML Engineer", "Architect",
    ],
    personalities: [
      "meticulous", "easygoing", "passionate", "composed", "quirky",
      "perfectionist", "optimistic", "analytical", "creative", "tenacious",
      "chatty", "taciturn", "curious", "cautious", "bold",
      "cheerful", "prickly", "diligent", "impulsive", "strategic",
    ],
  },
};

/** 언어 코드에 맞는 목록. 없으면 프리픽스(`en-GB` → `en`) → 폴백 → 정본 순. */
export function wordlistsFor(lang: string | null | undefined): WordLists {
  const code = (lang ?? "").toLowerCase();
  return (
    WORDLISTS[code] ??
    WORDLISTS[code.split("-")[0]] ??
    WORDLISTS[FALLBACK_LANGUAGE] ??
    WORDLISTS[SOURCE_LANGUAGE]
  );
}

/** 지금 UI 언어의 목록. **호출 시점에** 고른다 — 모듈 로드 때 굳히면 언어를
 *  바꿔도 옛 목록이 계속 나온다. */
export function currentWordlists(): WordLists {
  return wordlistsFor(i18n.language);
}
