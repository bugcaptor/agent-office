// src/renderer/labels/labelText.ts
//
// 머리 위 라벨의 파생 텍스트 순수 헬퍼. store에 저장하지 않고
// 표시 시점에 파생한다.
//
// 라벨 line1의 "프로젝트명 (브랜치)"는 서로 다른 두 cwd를 본다 —
// 프로젝트명은 탭을 연 프로필 cwd(정체성 앵커), 브랜치는 세션이 지금 실제로
// 있는 cwd. 세션 cwd는 훅 프롬프트마다 갱신되므로 작업 중
// `.claude/worktrees/<브랜치>` 같은 하위 폴더로 cd하면 basename이 브랜치명으로
// 바뀌어 프로젝트명 자리를 덮어쓴다. 그래서 세션 cwd가 프로필 cwd "안"에
// 있으면 프로젝트명은 프로필 cwd를 유지하고(projectAnchorCwd), 정말 다른
// 프로젝트로 이탈했을 때만 세션 cwd를 따른다. 브랜치 조회 키는 그대로
// effectiveCwd(세션 cwd 우선)라 워크트리의 브랜치가 제대로 보인다.
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

/**
 * 라벨이 실제로 가리키는 작업 폴더. 세션 실제 cwd 우선, 부재 시 프로필 cwd.
 * 브랜치 맵(cwd→브랜치)을 조회하는 호출부가 같은 키를 쓰도록 여기 한 곳에 둔다 —
 * 브랜치는 세션이 지금 있는 폴더의 것이어야 하므로 프로젝트명 앵커
 * (projectAnchorCwd)와는 규칙이 다르다.
 */
export function effectiveCwd(
  label: AgentTaskLabel | undefined,
  fallbackCwd: string | undefined
): string | undefined {
  return label?.cwd ?? fallbackCwd;
}

/** 경로 비교용 정규화: `\`→`/`, 트레일링 구분자 제거(대소문자는 보존). */
function normalizeCwd(cwd: string): string {
  return cwd.replace(/\\/g, "/").replace(/\/+$/, "");
}

/**
 * sessionCwd가 profileCwd와 같거나 그 하위인가.
 * profileCwd가 `~`/`~/...`이면 프런트는 홈 경로를 모르므로 `~` 뒤 나머지(suffix)가
 * sessionCwd 안에 경로 경계로 등장하고 그 뒤가 끝이거나 `/`인지로 판정한다
 * (`~` 단독이면 홈 전체라 항상 안으로 본다).
 */
function isInsideCwd(sessionCwd: string, profileCwd: string): boolean {
  const session = normalizeCwd(sessionCwd);
  const profile = normalizeCwd(profileCwd);
  if (profile === "~" || profile.startsWith("~/")) {
    const suffix = profile.slice(1); // "~/dev/proj" → "/dev/proj", "~" → ""
    if (suffix === "") return true;
    // suffix는 `/`로 시작하므로 앞쪽 경계는 자동으로 맞는다. 뒤쪽만 확인
    // ("/dev/proj"가 "/x/dev/proj2"에 걸리지 않도록) — 여러 번 등장할 수 있어 전부 훑는다.
    for (let at = session.indexOf(suffix); at !== -1; at = session.indexOf(suffix, at + 1)) {
      const next = session[at + suffix.length];
      if (next === undefined || next === "/") return true;
    }
    return false;
  }
  return session === profile || session.startsWith(profile + "/");
}

/**
 * 프로젝트명을 뽑을 기준 cwd(정체성 앵커). 한쪽만 있으면 그쪽,
 * 세션 cwd가 프로필 cwd 안(같거나 하위 — 워크트리·서브폴더)이면 프로필 cwd,
 * 정말 다른 곳으로 이탈했으면 세션 cwd. 브랜치 조회 키(effectiveCwd)와는 별개다.
 */
export function projectAnchorCwd(
  sessionCwd: string | undefined,
  profileCwd: string | undefined
): string | undefined {
  if (!profileCwd) return sessionCwd;
  if (!sessionCwd) return profileCwd;
  return isInsideCwd(sessionCwd, profileCwd) ? profileCwd : sessionCwd;
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
 * - line1 = 프로젝트명 · 목표. 프로젝트명은 `projectAnchorCwd(label.cwd,
 *   fallbackCwd)`의 basename — 세션이 프로필 cwd 하위(워크트리 등)로 옮겨가도
 *   프로필 cwd 이름을 유지하고, 다른 프로젝트로 이탈했을 때만 세션 cwd를 쓴다.
 *   목표는 LLM 요약 > 저장된 요청 문장 폴백 > 첫 프롬프트의 요청 문장.
 *   `opts.branch`를 주면 프로젝트명 뒤에 `(브랜치)`를 붙인다 — 호출부가
 *   effectiveCwd(세션 cwd 우선)로 조회한 값이라 워크트리로 옮기면 그 브랜치가
 *   뜬다. 비저장소·detached HEAD는 호출부가 아예 넘기지 않으므로 여기선 유/무만
 *   본다. 프로젝트명이 없으면(cwd 부재) 브랜치도 붙이지 않는다 — 괄호만 뜬
 *   라벨은 읽을 수 없다.
 * - line2 = 실황(assistant 내레이션 > 도구 요약) > LLM 지시 요약 > 최신 프롬프트
 *   요청 문장. currentSummary는 지시 요약이라 턴 중 실황보다 오래됐다(이슈 #43).
 *
 * 빈 결과는 undefined로 흘려 옵셔널 흐름을 유지한다.
 */
export function deriveTaskLabelLines(
  label: AgentTaskLabel | undefined,
  fallbackCwd: string | undefined,
  opts: { goalMax: number; currentMax: number; branch?: string }
): { line1?: string; line2?: string } {
  // 프로젝트명은 정체성 앵커 기준 — 세션 cwd가 프로필 cwd 하위면 프로필 cwd를
  // 유지한다(워크트리로 cd해도 폴더명이 브랜치명으로 덮이지 않도록).
  const baseProject = projectNameFromCwd(projectAnchorCwd(label?.cwd, fallbackCwd));
  const project =
    baseProject && opts.branch ? `${baseProject} (${opts.branch})` : baseProject;
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
