// src/renderer/i18n/promptProfiles.ts
//
// AI에게 보내는 **프롬프트 프로필**. 시스템 프롬프트 문자열 하나가 아니라,
// 그 프롬프트와 짝을 이루는 상수 묶음(출력 길이 상한, 거부 마커, 머리말 제거
// 정규식, 입력 블록 머리말, 자리 표시 문자열)을 언어당 객체 하나로 묶었다.
//
// 카탈로그 JSON이 아니라 여기 두는 이유(wordlists.ts와 같은 판단):
//  - 번역 문자열 하나가 아니라 **숫자·정규식·목록·포매터가 딸린 묶음**이다.
//    카탈로그는 문자열만 담고 키 파리티를 요구한다.
//  - 프롬프트 본문에 보간 아닌 중괄호·따옴표·화살표가 섞여 있어 i18next
//    보간 규칙과 충돌한다.
//  - 값이 화면에 보이는 UI 문구가 아니라 **모델 입력**이다. "번역이 정확한가"가
//    아니라 "그 언어에서 원하는 출력이 나오는가"로 평가해야 하므로, 언어마다
//    문장 수·글자 수·예시를 다르게 설계한다(en은 ko의 번역이 아니다).
// (`src/renderer/i18n/` 아래는 하드코딩 한글 스캐너의 제외 경로다.)
//
// ## 폴백 규칙 — 여기서 못 박는다
//
// 프로필이 없는 언어는 **en 프로필로 돈다**(정확 일치 → 프리픽스 일치(`en-GB`
// → `en`) → FALLBACK_LANGUAGE → SOURCE_LANGUAGE). 즉 카탈로그에 언어를 추가하고
// 여기에 프로필을 만들지 않으면 UI는 그 언어인데 요약·일기·소감만 영어로 나오는
// 상태가 되는데, **이건 버그가 아니라 정상 동작이다** — 기계 번역한 프롬프트로
// 출력 품질을 망치느니 영어로 도는 편이 낫다. 새 언어의 프로필은 그 언어를 쓰는
// 사람이 실제 출력을 보며 튜닝해서 넣어야 한다.
//
// ## ko 프롬프트는 동결이다
//
// ko의 systemPrompt와 딸린 숫자·마커는 Phase 5 이전 값에서 **이동만** 했다.
// 프롬프트 한 글자가 요약·일기·소감의 품질을 바꾸므로 튜닝은 별도 작업이다.
// `__tests__/promptProfiles.test.ts`가 예전 문자열 사본과 바이트 비교로 지킨다.
import { FALLBACK_LANGUAGE, SOURCE_LANGUAGE } from "@shared/i18n/catalog";

import { i18n } from "./index";

/**
 * 언어 코드 → 프로필 맵에서 하나를 고른다. 위 "폴백 규칙"의 구현이자
 * `wordlistsFor`와 같은 관례.
 */
function pickProfile<T>(map: Record<string, T>, lang: string | null | undefined): T {
  const code = (lang ?? "").toLowerCase();
  return (
    map[code] ?? map[code.split("-")[0]] ?? map[FALLBACK_LANGUAGE] ?? map[SOURCE_LANGUAGE]
  );
}

/**
 * 거부 마커 판정. 마커 목록은 **전부 소문자로** 적어 두고 입력도 소문자화해
 * 비교한다 — 영어 거부는 `Sorry`/`I cannot`처럼 대문자로 시작하는 경우가
 * 대부분이라 대소문자 구분 비교로는 놓친다. 한글은 `toLowerCase()`가 항등이라
 * ko 동작은 예전(`s.includes(m)`)과 완전히 같다.
 */
export function hasMetaMarker(s: string, markers: readonly string[]): boolean {
  const lower = s.toLowerCase();
  return markers.some((m) => lower.includes(m));
}

// ---------------------------------------------------------------------------
// 머리 위 라벨 요약(labels/summarizer.ts)
// ---------------------------------------------------------------------------

/** 라벨 요약 프롬프트와 그에 딸린 정제 상수. */
export interface LabelPromptProfile {
  /** 요약기 시스템 프롬프트. 아래 `headers`·`noneText`·`fallbackText`를 그대로 가리킨다. */
  systemPrompt: string;
  /** sanitizeLine의 과길이 거부 기준(코드포인트). 라벨 폭이 아니라 "LLM이
   *  헛소리를 길게 뱉었는가" 판정이다. */
  summaryMaxChars: number;
  /** [초기 작업 정황] 주입 상한(코드포인트). */
  contextMaxChars: number;
  /** 거부 사유 마커 — **소문자로 적는다**(hasMetaMarker 참고). */
  metaMarkers: string[];
  /** "1줄:", "목표:" 류 머리말 제거. */
  linePrefixPattern: RegExp;
  /** 모델이 판단 불가일 때 쓰라고 지시한 문구. 프롬프트 안에도 같은 값이 박혀 있다. */
  fallbackText: string;
  /** 사용자 입력 블록 머리말 — 시스템 프롬프트가 이 이름들을 가리키므로 짝이다. */
  headers: { prevGoal: string; newInstruction: string; context: string };
  /** 이전 목표가 없을 때 넣는 자리 표시. */
  noneText: string;
}

const LABEL_PROMPT_PROFILES: Record<string, LabelPromptProfile> = {
  ko: {
    systemPrompt: "너는 코딩 세션 라벨 생성기다. [이전 목표], [새 지시], 그리고 있을 경우 [초기 작업 정황]을 보고 정확히 두 줄을 출력하라. 1줄: 세션 목표(한국어 명사구 12자 이내). [초기 작업 정황]이 있으면 그것이 이 세션이 실제로 무엇을 하는지 보여주는 근거이므로, 이슈·티켓 번호만 가리키는 모호한 지시(예: '이슈 40 해결')보다 우선해 목표를 구체화하라. [초기 작업 정황]이 없으면: 새 지시가 새로운 작업이면 새로 뽑고, 이전 작업의 후속·보완 지시이거나 판단이 애매하면 이전 목표를 그대로 출력하라. 이전 목표가 (없음)이면 새로 뽑아라. 2줄: 새 지시 요약 — 한국어 18자 이내 한 줄. 규칙: 정확히 두 줄, 한국어만, 사과·설명·따옴표·번호·머리말 금지. 판단 불가면 1줄은 이전 목표(없으면 '작업 중'), 2줄은 '작업 중'. 예) 이전 목표: 로그인 버그 수정 / 새 지시: 테스트도 고쳐줘 → 1줄 '로그인 버그 수정', 2줄 '테스트 수정'. 예) 이전 목표: (없음) / 새 지시: 이슈 40 해결 / 초기 작업 정황: Claude 훅 설정 파일을 복구하는 중 → 1줄 '훅 설정 복구', 2줄 '이슈 40 해결'",
    summaryMaxChars: 40,
    contextMaxChars: 120,
    metaMarkers: ["인코딩", "죄송", "할 수 없"],
    linePrefixPattern: /^(1줄|2줄|요약|목표)\s*[:：]\s*/,
    fallbackText: "작업 중",
    headers: { prevGoal: "[이전 목표]", newInstruction: "[새 지시]", context: "[초기 작업 정황]" },
    noneText: "(없음)",
  },
  en: {
    // ko의 번역이 아니다. 바뀐 곳과 이유:
    //  - 길이 제약을 **글자 수가 아니라 단어 수**로. 한글 12자가 픽셀 라벨의
    //    예산인데 그건 영문 24자쯤이고, 영어권 모델에게 "24 characters"는
    //    지키기 어려운 지시다(공백 포함/제외부터 흔들린다). 4단어/6단어가
    //    같은 폭을 더 안정적으로 맞춘다.
    //  - 예시를 영어 예시로 교체. 예시는 출력 **형식**을 가르치는 장치라
    //    한국어 예시를 남기면 "한국어로 답하라"는 신호로 읽힌다.
    systemPrompt:
      "You are a coding-session label generator. Read [Previous goal], [New instruction], and [Initial context] when present, then output exactly two lines. Line 1: the session goal (an English noun phrase, at most 4 words). If [Initial context] is present it is the evidence of what this session is actually doing, so prefer it over a vague instruction that only points at an issue or ticket number (e.g. 'fix issue 40') and make the goal concrete. If there is no [Initial context]: if the new instruction starts a different task, derive a new goal; if it is a follow-up or a refinement of the previous task, or the call is ambiguous, repeat the previous goal unchanged. If the previous goal is (none), derive a new one. Line 2: a summary of the new instruction — one line, at most 6 words in English. Rules: exactly two lines, English only, no apologies, explanations, quotes, numbering, or prefixes. If you cannot decide, line 1 is the previous goal (or 'Working' if there is none) and line 2 is 'Working'. Example) Previous goal: Fix login bug / New instruction: fix the tests too -> line 1 'Fix login bug', line 2 'Fix tests'. Example) Previous goal: (none) / New instruction: resolve issue 40 / Initial context: restoring the Claude hook config file -> line 1 'Restore hook config', line 2 'Resolve issue 40'",
    // 한글 40자에 담기는 정보는 영문으로 대략 두 배의 글자를 먹는다. 이 값은
    // 라벨이 잘리는 폭이 아니라 폭주 감지선이므로 같은 비율로 키운다.
    summaryMaxChars: 80,
    contextMaxChars: 240,
    // 영어 모델의 전형적 거부·오류 문구. 'cannot'/'unable'을 홀로 두면
    // "cannot reproduce" 같은 정상 라벨을 잡아먹으므로 거부 정형구로 좁혔다.
    metaMarkers: [
      "sorry",
      "i cannot",
      "i can't",
      "unable to",
      "as an ai",
      "i'm an ai",
      "encoding",
    ],
    linePrefixPattern: /^(line\s*1|line\s*2|summary|goal)\s*[:：]\s*/i,
    fallbackText: "Working",
    headers: {
      prevGoal: "[Previous goal]",
      newInstruction: "[New instruction]",
      context: "[Initial context]",
    },
    noneText: "(none)",
  },
};

/** 지금(또는 지정한) 언어의 라벨 요약 프로필. **호출 시점에** 고른다 —
 *  모듈 최상위에서 굳히면 언어를 바꿔도 옛 프로필이 계속 나온다. */
export function labelPromptProfile(lang?: string): LabelPromptProfile {
  return pickProfile(LABEL_PROMPT_PROFILES, lang ?? i18n.language);
}

// ---------------------------------------------------------------------------
// 캐릭터 일기(diary/diaryGenerator.ts)
// ---------------------------------------------------------------------------

/** 일기 프롬프트와 그에 딸린 상수. */
export interface DiaryPromptProfile {
  /** 일기 시스템 프롬프트. 아래 `headers`를 그대로 가리킨다. */
  systemPrompt: string;
  /** 사용자 입력 블록 머리말. */
  headers: { personality: string; workLog: string };
  /** 성격이 비었을 때 넣는 자리 표시. */
  noneText: string;
  /** 일기 본문 최소 길이(공백 제외 코드포인트). 이보다 짧으면 생성 실패로 본다. */
  bodyMinChars: number;
}

const DIARY_PROMPT_PROFILES: Record<string, DiaryPromptProfile> = {
  ko: {
    systemPrompt: "너는 한 캐릭터의 일기 작성기다. 아래 [성격]을 문체로 삼아, [작업 로그]를 1인칭 한국어 일기 한 편으로 써라. 성격에 따라 초등학생 일기처럼 쓰기도 하고 차가운 작업 일지처럼 쓰기도 한다 — [성격]의 말투·태도를 문체에 그대로 반영하라. [성격]이 비어 있으면 담백한 중립 문체로 써라. 반드시 실제로 한 일(수정한 파일·실행한 명령·목표)이 드러나야 한다(작업 로그를 겸한다). 분량은 3~8문장. 규칙: 한국어만, 사과·메타발언·머리말·따옴표·마크다운 금지, 일기 본문만 출력.",
    headers: { personality: "[성격]", workLog: "[작업 로그]" },
    noneText: "(없음)",
    bodyMinChars: 4,
  },
  en: {
    // 분량 제약이 글자 수가 아니라 **문장 수(3~8)**라 언어를 타지 않는다 —
    // ko와 같은 값을 유지했다.
    systemPrompt:
      "You are the diary writer for one character. Take [Personality] as your voice and turn [Work log] into a first-person diary entry in English. Depending on the personality it may read like a schoolchild's diary or like a cold engineering journal — mirror the tone and attitude in [Personality] directly in the prose. If [Personality] is empty, write in a plain, neutral voice. The entry must make the actual work visible (files changed, commands run, goals) — it doubles as a work log. Length: 3 to 8 sentences. Rules: English only, no apologies, meta commentary, prefixes, quotes, or markdown, output the diary body only.",
    headers: { personality: "[Personality]", workLog: "[Work log]" },
    noneText: "(none)",
    // 공백을 뺀 한글 4자(≈ 두세 단어)가 "본문이 있다"의 하한이었다. 영문은
    // 같은 분량이 두 배쯤의 글자라 8로 둔다.
    bodyMinChars: 8,
  },
};

/** 지금(또는 지정한) 언어의 일기 프로필. 호출 시점에 고른다. */
export function diaryPromptProfile(lang?: string): DiaryPromptProfile {
  return pickProfile(DIARY_PROMPT_PROFILES, lang ?? i18n.language);
}

// ---------------------------------------------------------------------------
// 이 달의 우수사원 수상 소감(awards/speechGenerator.ts)
// ---------------------------------------------------------------------------

/** 일기 발췌 분량 한도. */
export interface ExcerptLimits {
  /** 담을 최대 편수. 예산이 모자라면 이보다 줄어든다. */
  maxEntries: number;
  /** 한 편당 담는 최대 글자 수(예산이 넉넉해도 이 이상은 안 담는다). */
  perEntryChars: number;
  /** 발췌 전체 글자 예산. 렌더 결과가 이 안에 들어가는 편수를 고른다. */
  totalChars: number;
}

/** `formatAwardInfo`에 넘기는 값들 — 포매팅 전의 순수 수치. */
export interface AwardInfoValues {
  /** `YYYY-MM`. */
  month: string;
  /** 작업 시간(시간 단위 반올림). */
  hours: number;
  turns: number;
  activeDays: number;
  /** 이번 수상을 포함한 통산 횟수. */
  totalAwards: number;
}

/** 수상 소감 프롬프트와 그에 딸린 분량 상수. */
export interface SpeechPromptProfile {
  /** 소감 시스템 프롬프트. 아래 `headers`를 그대로 가리킨다. */
  systemPrompt: string;
  /** 사용자 입력 블록 머리말. */
  headers: { personality: string; awardInfo: string; diary: string };
  /** 성격이 비었을 때 넣는 자리 표시. */
  noneText: string;
  /** 그 달 일기가 한 편도 없을 때 넣는 자리 표시(sentinel). */
  noDiaryText: string;
  /**
   * 프롬프트(성격 + 수상 정보 + 일기 발췌) 총 글자 예산.
   *
   * **언어와 무관하게 같은 값이다.** 백엔드 `TEXT_MAX_CHARS`(2,000자)의
   * `cap_text` 중략을 피하려고 그보다 작게 잡은 값이라, 근거가 한국어 글자
   * 밀도가 아니라 백엔드의 고정 상한이다. 영어에서 같은 예산이 담는 정보가
   * 적어지는 건 사실이지만, 예산을 키우면 백엔드가 월 중간을 잘라 버린다.
   */
  promptBudgetChars: number;
  /** 성격 프롬프트 절단 한도. 성격이 길어도 일기 발췌 예산을 다 먹지 못하게 한다. */
  personalityMaxChars: number;
  /** 소감 출력 상한(글자). 프롬프트의 분량 지시보다 약간 넉넉한 안전망. */
  speechMaxChars: number;
  /** 소감 출력 상한(문장). 프롬프트의 문장 수 지시와 같은 값. */
  speechMaxSentences: number;
  /** 일기 발췌 기본 한도. */
  excerptLimits: ExcerptLimits;
  /** 발췌 한 편의 최소 본문 길이. 이보다 잘게 쪼갤 바에는 편수를 줄인다. */
  excerptMinBodyChars: number;
  /** [수상 정보] 블록 본문. 수치를 그 언어의 문장으로 조립한다. */
  formatAwardInfo: (v: AwardInfoValues) => string;
}

const SPEECH_PROMPT_PROFILES: Record<string, SpeechPromptProfile> = {
  ko: {
    systemPrompt: "너는 사내 시상식에서 '이 달의 우수사원'으로 호명된 캐릭터 본인이다. 아래 [성격]을 문체로 삼아, [수상 정보]와 [지난달 일기]를 근거로 1인칭 한국어 수상 소감을 써라. 분량은 2~4문장, 200자 이내. [수상 정보]의 수치 하나쯤은 자연스럽게 녹여도 좋지만 통계를 나열하지 마라. [성격]이 비어 있으면 담백한 중립 문체로 써라. 규칙: 한국어만, 사과·메타발언·머리말·따옴표·마크다운 금지, 소감 본문만 출력.",
    headers: { personality: "[성격]", awardInfo: "[수상 정보]", diary: "[지난달 일기]" },
    noneText: "(없음)",
    noDiaryText: "(일기 없음)",
    promptBudgetChars: 1_900,
    personalityMaxChars: 300,
    speechMaxChars: 240,
    speechMaxSentences: 4,
    excerptLimits: { maxEntries: 10, perEntryChars: 200, totalChars: 1_500 },
    excerptMinBodyChars: 60,
    formatAwardInfo: (v) =>
      [
        `월: ${v.month}`,
        `작업 시간: 약 ${v.hours}시간`,
        `턴 수: ${v.turns}`,
        `활동일: ${v.activeDays}일`,
        // 이번 수상을 포함한 통산 횟수 — 첫 수상이면 1회다.
        `통산 수상: ${v.totalAwards}회(이번 포함)`,
      ].join("\n"),
  },
  en: {
    // 분량 지시는 "2~4문장"은 그대로 두고 글자 상한만 200 → 400으로 옮겼다.
    // 같은 2~4문장을 영어로 쓰면 글자 수가 두 배쯤 되므로, 200자로 두면
    // 문장 수 지시와 글자 지시가 서로 모순돼 모델이 문장을 잘라 버린다.
    systemPrompt:
      "You are the character who has just been called up as 'Employee of the Month' at a company award ceremony. Take [Personality] as your voice and write a first-person acceptance speech in English, grounded in [Award info] and [Last month's diary]. Length: 2 to 4 sentences, at most 400 characters. You may weave in one of the numbers from [Award info], but do not recite statistics. If [Personality] is empty, write in a plain, neutral voice. Rules: English only, no apologies, meta commentary, prefixes, quotes, or markdown, output the speech body only.",
    headers: {
      personality: "[Personality]",
      awardInfo: "[Award info]",
      diary: "[Last month's diary]",
    },
    noneText: "(none)",
    noDiaryText: "(no diary)",
    // 백엔드 cap_text(2,000자)에서 나온 값 — 언어를 타지 않는다. 위 주석 참고.
    promptBudgetChars: 1_900,
    // 고정 예산 안에서 배분만 조정한다. 영문은 같은 성격 서술이 대략 두 배의
    // 글자를 먹지만 300 → 600으로 두 배를 주면 일기 발췌가 굶으므로 500에서
    // 끊었다(남는 예산 ≈ 1,300자).
    personalityMaxChars: 500,
    // 프롬프트의 "400자 이내"보다 약간 넉넉하게 — ko의 200/240과 같은 비율.
    speechMaxChars: 480,
    speechMaxSentences: 4,
    // 편수를 줄이고 편당 분량을 키웠다. 영문 200자는 짧은 두 문장이라
    // 10편을 담으면 전부 문장 조각이 되어 근거로 못 쓴다.
    excerptLimits: { maxEntries: 8, perEntryChars: 400, totalChars: 1_500 },
    excerptMinBodyChars: 120,
    formatAwardInfo: (v) =>
      [
        `Month: ${v.month}`,
        `Time worked: about ${v.hours} hours`,
        `Turns: ${v.turns}`,
        `Active days: ${v.activeDays}`,
        `Awards to date: ${v.totalAwards} (including this one)`,
      ].join("\n"),
  },
};

/** 지금(또는 지정한) 언어의 수상 소감 프로필. 호출 시점에 고른다. */
export function speechPromptProfile(lang?: string): SpeechPromptProfile {
  return pickProfile(SPEECH_PROMPT_PROFILES, lang ?? i18n.language);
}

// ---------------------------------------------------------------------------
// 프로젝트 실행 레시피 조사(run/execute.ts)
// ---------------------------------------------------------------------------

export interface RunRecipePromptProfile {
  formatProbePrompt(root: string, agentFilePath: string, projectPath?: string): string;
}

const RUN_RECIPE_PROMPT_PROFILES: Record<string, RunRecipePromptProfile> = {
  ko: {
    formatProbePrompt: (root, agentFilePath, projectPath = root) => `이 프로젝트(${projectPath})의 실행·테스트·빌드 방법을 조사해서 ${agentFilePath}에 JSON으로 정리해 줘. package.json scripts(패키지 매니저는 락파일로 판단), Cargo.toml, Makefile/justfile, pyproject.toml, CI 워크플로, AGENTS.md와 README를 읽고, 사람이 적어 둔 명령을 우선해. 파일이 이미 있으면 먼저 읽고 여전히 유효한 항목은 id를 유지하고, 없어진 것은 지우고, 새 항목은 더해. 형식은 {"version":1,"root":"${root}","updatedAt":"ISO 8601","recipes":[{"id":"stable-slug","label":"표시 이름","command":"실행할 셸 명령","cwd":"선택: 프로젝트 상대 하위 폴더","longRunning":false,"note":"선택: 한 줄 근거"}]}이다. command가 없는 항목은 만들지 말고, dev 서버나 watch처럼 끝나지 않는 명령은 longRunning:true로 적어. 명령은 실제로 실행하지 말고 파일만 읽어. clean, reset, deploy, publish 같은 파괴적 명령은 넣지 마. 끝나면 몇 개를 적었는지 한 줄로만 답해.`,
  },
  en: {
    formatProbePrompt: (root, agentFilePath, projectPath = root) => `Inspect the project at ${projectPath} and write its useful development, test, and build commands to ${agentFilePath} as JSON. Read package.json scripts (infer the package manager from lockfiles), Cargo.toml, Makefile/justfile, pyproject.toml, CI workflows, AGENTS.md, and README files; prefer commands explicitly documented by people. If the file already exists, read it first, keep the ids of commands that are still valid, remove obsolete entries, and add new ones. Use this shape: {"version":1,"root":"${root}","updatedAt":"ISO 8601","recipes":[{"id":"stable-slug","label":"display name","command":"shell command","cwd":"optional project-relative subdirectory","longRunning":false,"note":"optional one-line source"}]}. Omit entries without a command and mark dev servers or watchers with longRunning:true. Do not execute any commands; only inspect files. Exclude destructive commands such as clean, reset, deploy, and publish. When finished, reply with one line stating how many recipes you wrote.`,
  },
};

/** 지금(또는 지정한) 언어의 실행 레시피 조사 프롬프트. 없는 언어는 영어 폴백. */
export function runRecipePromptProfile(lang?: string): RunRecipePromptProfile {
  return pickProfile(RUN_RECIPE_PROMPT_PROFILES, lang ?? i18n.language);
}
