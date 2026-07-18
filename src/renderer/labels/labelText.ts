// src/renderer/labels/labelText.ts
//
// 머리 위 라벨의 파생 텍스트 순수 헬퍼. store에 저장하지 않고
// 표시 시점에 파생한다.
//
// 이 모듈은 스토어 런타임에 의존하지 않는 순수 모듈이다 — 아래
// AgentTaskLabel은 타입만 필요하므로 반드시 `import type`으로만 가져와
// 순수성을 유지한다(런타임 import가 섞이면 이 파일을 쓰는 곳마다 스토어를
// 끌고 들어온다).
import type { AgentTaskLabel } from "../store/types";

/** cwd의 basename. `/`와 `\` 둘 다 구분자로 취급, 트레일링 구분자 무시. */
export function projectNameFromCwd(cwd: string | undefined): string | undefined {
  if (!cwd) return undefined;
  const parts = cwd.split(/[/\\]+/).filter(Boolean);
  return parts.length > 0 ? parts[parts.length - 1] : undefined;
}

/** chars 기준 max자로 절단, 넘치면 "…" 부착(멀티바이트 안전). 표시 쪽 공용 헬퍼. */
export function truncateChars(text: string, max: number): string {
  const chars = Array.from(text);
  return chars.length <= max ? text : chars.slice(0, max).join("") + "…";
}

/** 원문 폴백 표시: 첫 비공백 줄을 max자(chars)로 절단, 넘치면 "…" 부착. */
export function firstLine(text: string | undefined, max: number): string | undefined {
  if (!text) return undefined;
  const line = text
    .split("\n")
    .map((l) => l.trim())
    .find((l) => l.length > 0);
  if (!line) return undefined;
  return truncateChars(line, max);
}

// 명령·요청·소망 어미(끝 부호 제거 후 말미에서 판정). 하나라도 맞으면 요청 문장으로 가점(+2).
// 과도한 열거보다 소수의 견고한 정규식으로 다듬는다(이슈 #44 작업 A).
const REQUEST_ENDINGS: RegExp[] = [
  /줘$/, // 해줘·고쳐줘
  /주세요$/, // 해주세요
  /(해라|하라)$/, // 해라·하라
  /(하자|합시다)$/, // 하자·합시다
  /할\s?것$/, // 할 것·할것
  /해야\s?(해|한다|함)$/, // 해야 해/한다/함
  /[가-힣]해$/, // 반말 명령: 코멘트해·추가해
  /(하고\s?싶|좋겠)[가-힣]*$/, // 소망: 하고 싶다·좋겠다
];

// 인삿말뿐인 조각(짧고 인사로 시작) — 요청 후보에서 제외한다.
const GREETING_START = /^(안녕|하이|헬로|반가|hi|hello)/i;

/** 줄바꿈 → 문장 종결 부호(`. ! ? … 。`) 순으로 나눠, 내용 있는 조각만 남긴다. */
function splitSentences(text: string): string[] {
  const out: string[] = [];
  for (const line of text.split("\n")) {
    for (const raw of line.split(/[.!?…。]+/)) {
      const frag = raw.trim();
      // 부호·공백뿐인 조각 제거(문자·숫자가 하나라도 있어야 조각으로 인정).
      if (frag.length > 0 && /[가-힣A-Za-z0-9]/.test(frag)) out.push(frag);
    }
  }
  return out;
}

/** 조각 점수: 끝 부호를 무시하고 요청·명령·소망 어미로 끝나면 2, 아니면 0. */
function scoreFragment(fragment: string): number {
  const core = fragment.replace(/[\s.?!~…]+$/u, "");
  return REQUEST_ENDINGS.some((re) => re.test(core)) ? 2 : 0;
}

/** 인삿말뿐인(짧은) 조각인가. */
function isGreetingOnly(fragment: string): boolean {
  return GREETING_START.test(fragment) && Array.from(fragment).length <= 12;
}

/**
 * 프롬프트에서 "요청 문장"을 고른다(절단 없음 — 표시 쪽에서 truncateChars).
 * 한국어 프롬프트는 맥락 서술로 시작해 실제 요청이 끝에 오는 경우가 많으므로,
 * 요청·명령·소망 어미로 끝나는 조각을 우선하고 동점이면 뒤쪽(마지막) 조각을 쓴다.
 * 요청 어미가 하나도 없으면 마지막 조각, 내용 없으면 undefined(이슈 #44 작업 A).
 */
export function requestSentence(text: string | undefined): string | undefined {
  if (!text) return undefined;
  const fragments = splitSentences(text);
  if (fragments.length === 0) return undefined;
  // 인삿말뿐인 조각은 후보에서 뺀다. 전부 인삿말이면 다시 전체를 후보로.
  const candidates = fragments.filter((f) => !isGreetingOnly(f));
  const pool = candidates.length > 0 ? candidates : fragments;
  let best = pool[0];
  let bestScore = scoreFragment(best);
  for (let i = 1; i < pool.length; i++) {
    const score = scoreFragment(pool[i]);
    // >= 로 동점 시 뒤쪽(마지막) 조각을 선택한다.
    if (score >= bestScore) {
      best = pool[i];
      bestScore = score;
    }
  }
  return best;
}

/**
 * 라벨 소스에서 머리 위 라벨의 두 줄(line1/line2)을 파생한다.
 * TaskLabelLayer와 터미널 요약 표시가 같은 규칙을 공유하도록 한 곳에 모은 것
 * (이슈 #44 T1/T2). 절단 폭은 표시처마다 다르므로 opts로 받는다.
 *
 * - line1 = 프로젝트명 · 목표. 프로젝트명은 `label.cwd ?? fallbackCwd`의
 *   basename, 목표는 LLM 요약 > 저장된 요청 문장 폴백 > 첫 프롬프트의 요청 문장.
 * - line2 = 실황(assistant 내레이션 > 도구 요약) > LLM 지시 요약 > 최신 프롬프트
 *   요청 문장. currentSummary는 지시 요약이라 턴 중 실황보다 오래됐다(이슈 #43).
 *
 * 빈 결과는 undefined로 흘려 옵셔널 흐름을 유지한다.
 */
export function deriveTaskLabelLines(
  label: AgentTaskLabel | undefined,
  fallbackCwd: string | undefined,
  opts: { goalMax: number; currentMax: number }
): { line1?: string; line2?: string } {
  // 세션 실제 cwd 우선, 부재 시 폴백 cwd(프로필 cwd)로 폴백(이슈 #44 작업 D).
  const project = projectNameFromCwd(label?.cwd ?? fallbackCwd);
  const goal =
    label?.goal ??
    (truncateChars(
      label?.goalFallback ?? requestSentence(label?.firstPromptText) ?? "",
      opts.goalMax
    ) || undefined);
  const line1 = [project, goal].filter(Boolean).join(" · ") || undefined;
  const line2 =
    firstLine(label?.latestAssistantText, opts.currentMax) ??
    firstLine(label?.latestToolText, opts.currentMax) ??
    label?.currentSummary ??
    (truncateChars(requestSentence(label?.latestPromptText) ?? "", opts.currentMax) ||
      undefined);
  return { line1, line2 };
}
