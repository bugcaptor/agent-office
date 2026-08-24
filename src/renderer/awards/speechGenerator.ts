// src/renderer/awards/speechGenerator.ts
//
// 수상 소감 생성기. 구조는 diary/diaryGenerator.ts를 그대로 본떴다 — 요약기
// 파이프라인(tauriApi.summarizeText)에 다른 시스템 프롬프트 + 성격 + 근거 자료를
// 실어 보내는 같은 호출이고, 실패는 던지지 않고 사유 문자열로 돌려준다.
//
// 목적(purpose)은 "diary"를 재사용한다 — 백그라운드 배치 성격이라 넉넉한
// 타임아웃(120초)이 필요하고, 새 variant를 만들면 백엔드까지 건드려야 한다.
//
// 근거 자료로 그 달의 일기를 넣는다. 통계 숫자만으로는 "무엇을 해서" 우수사원이
// 됐는지가 안 나오기 때문이다.
//
// ## 분량은 여기서 기계적으로 정한다
//
// 백엔드 요약기는 "diary" 목적 입력을 `TEXT_MAX_CHARS`(2,000자)에서 자른다
// (`src-tauri/src/summarizer/mod.rs`의 `cap_text`). 그 절단은 head 60% +
// `…(중략)…` + tail 40%라 **월 중간이 통째로 날아간다** — 월 전체를 균등
// 간격으로 훑어 놓고 정작 가운데를 백엔드가 버리면 "한 달을 돌아보는" 소감이
// 되지 않는다. 그래서 프롬프트 총량을 프로필의 `promptBudgetChars`(백엔드
// 상한보다 작게)로 잡고, 성격·수상 정보를 먼저 확보한 뒤 **남는 예산 전부**를 일기
// 발췌에 준다. 백엔드 `cap_text`는 이제 안전망일 뿐 실제로는 걸리지 않는다.
//
// 출력도 같은 이유로 프런트에서 잠근다(`clampSpeech`). OpenRouter만 max_tokens를
// 걸 수 있고 CLI provider(claude/codex/…)는 상한이 없어 프롬프트의 "2~4문장"
// 지시가 유일한 제어인데, 그 지시는 지켜지지 않을 때가 있다. UI 카드도 씬
// 말풍선도 짧은 소감을 전제로 하므로 문장 단위로 잘라 길이를 보장한다.
//
// 시스템 프롬프트·블록 머리말·분량 상수(출력 글자/문장 상한, 발췌 한도,
// sentinel)는 UI 언어를 따른다 — `i18n/promptProfiles.ts`의
// `speechPromptProfile()`이 호출 시점에 고른다. 단 `promptBudgetChars`만은
// 언어와 무관하다(백엔드 `cap_text`의 고정 글자 상한에서 온 값이다).
import { useAppStore } from "../store/appStore";
import { tauriApi } from "../ipc/tauriApi";
import { sanitizeDiaryBody } from "../diary/diaryGenerator";
import { localDayCalendar, type DayCalendar } from "../analytics/aggregate";
import { speechPromptProfile } from "../i18n/promptProfiles";
import type { ExcerptLimits } from "../i18n/promptProfiles";
import { monthKeyOf } from "./selection";
import type {
  AgentProfile,
  AwardRecord,
  AwardSpeech,
  DiaryEntry,
  SummaryProvider,
} from "@shared/types";

export type { ExcerptLimits };

/** 발췌 한 줄의 고정 접두 `- YYYY-MM-DD: ` 길이(언어 무관). */
const LINE_PREFIX_CHARS = 14;

/** 문장 끝으로 볼 문자. */
const SENTENCE_END = /[.!?…。]/;

/** 생성 결과 사유 — diaryGenerator의 사유 어휘를 그대로 쓴다(+시상 전용 둘). */
export type SpeechFailReason =
  | "no-winner"
  | "profile-missing"
  | "disabled"
  | "cli-missing"
  | "timeout"
  | "failed";

export type SpeechResult =
  | { ok: true; speech: AwardSpeech }
  | { ok: false; reason: SpeechFailReason };

export interface SpeechDeps {
  summarizeFn?: (
    provider: SummaryProvider,
    instruction: string,
    text: string,
  ) => Promise<string>;
  loadDiaryFn?: (agentId: string) => Promise<DiaryEntry[]>;
  /** 사용할 provider. 부재 시 설정의 summaryProvider. */
  provider?: SummaryProvider;
  /** 요약기 opt-in 게이트. 부재 시 설정의 summarizerEnabled. */
  enabled?: boolean;
  /** 일기의 월 귀속 판정에 쓸 캘린더. 부재 시 시스템 로컬. */
  cal?: DayCalendar;
  now?: () => number;
  /** 발췌 한도 강제(테스트). 부재 시 남는 프롬프트 예산에서 역산한다. */
  limits?: ExcerptLimits;
}

/** `n`개에서 `count`개를 균등 간격으로 뽑은 인덱스(양끝 포함, 오름차순·중복 없음). */
function evenIndices(n: number, count: number): number[] {
  if (count >= n) return Array.from({ length: n }, (_, i) => i);
  if (count <= 1) return [0];
  const out: number[] = [];
  let prev = -1;
  for (let i = 0; i < count; i++) {
    const idx = Math.round((i * (n - 1)) / (count - 1));
    if (idx !== prev) out.push(idx);
    prev = idx;
  }
  return out;
}

/** 코드포인트 길이(이모지·한글 안전). */
function len(s: string): number {
  return Array.from(s).length;
}

/**
 * `max`자 안에서 자르되 되도록 문장 경계에서 끊는다. 문장 중간에서 뚝 끊긴
 * 조각은 LLM이 그대로 인용해 어색한 소감을 만들기 쉽다. 경계가 너무 앞이면
 * (담을 내용의 절반도 못 채우면) 그냥 하드 컷 + `…`. 결과는 `…`를 포함해
 * 항상 `max`자 이하다(백엔드 `cap_text`와 같은 약속).
 */
export function cutAtSentence(body: string, max: number): string {
  const chars = Array.from(body);
  if (chars.length <= max) return body;
  const head = chars.slice(0, max);
  for (let i = head.length - 1; i >= Math.floor(max * 0.5); i--) {
    if (SENTENCE_END.test(head[i])) return head.slice(0, i + 1).join("").trimEnd();
  }
  return `${chars.slice(0, Math.max(0, max - 1)).join("").trimEnd()}…`;
}

/** 문장 단위 분해. 연속 부호(`…`, `!!`)는 한 문장으로 묶고 개행도 경계로 본다. */
export function splitSentences(text: string): string[] {
  const chars = Array.from(text);
  const out: string[] = [];
  let start = 0;
  for (let i = 0; i < chars.length; i++) {
    const c = chars[i];
    const isBreak =
      c === "\n" || (SENTENCE_END.test(c) && !SENTENCE_END.test(chars[i + 1] ?? ""));
    if (!isBreak) continue;
    const s = chars.slice(start, i + 1).join("").trim();
    if (s) out.push(s);
    start = i + 1;
  }
  const rest = chars.slice(start).join("").trim();
  if (rest) out.push(rest);
  return out;
}

/**
 * 소감 출력을 문장 단위로 잘라 길이를 보장한다. 문장을 통째로 버리므로 끝이
 * 깔끔하고, 한 문장이 통째로 상한을 넘길 때만 마지막 수단으로 글자 절단한다.
 */
export function clampSpeech(
  text: string,
  maxChars: number = speechPromptProfile().speechMaxChars,
  maxSentences: number = speechPromptProfile().speechMaxSentences,
): string {
  let parts = splitSentences(text).slice(0, maxSentences);
  if (parts.length === 0) return "";
  while (parts.length > 1 && len(parts.join(" ")) > maxChars) parts = parts.slice(0, -1);
  const out = parts.join(" ");
  return len(out) > maxChars ? cutAtSentence(out, maxChars) : out;
}

/** 남는 프롬프트 예산으로 발췌 한도를 만든다. 기본 한도를 넘지는 않는다. */
export function excerptLimitsFor(
  budget: number,
  base: ExcerptLimits = speechPromptProfile().excerptLimits,
): ExcerptLimits {
  return { ...base, totalChars: Math.max(0, Math.min(base.totalChars, budget)) };
}

/**
 * 그 달 일기를 프롬프트에 넣을 발췌 한 덩이로 만든다.
 *
 * `month`에 속한 항목만 남기고(월 판정은 selection의 monthKeyOf 재사용),
 * **예산 안에 실제로 들어가는 가장 많은 편수**를 고른다 — 편수를 위에서부터
 * 내려가며 렌더해 보고 처음 들어맞는 것을 쓴다. 뽑기는 최신 우선이 아니라
 * 균등 간격(양끝 포함)이라 월 전체 흐름이 남고, 편당 분량도 그 편수에 맞춰
 * 다시 정해진다(편수↑ → 편당↓, 단 프로필의 `excerptMinBodyChars` 아래로는 안
 * 간다). 남는 게 없으면 프로필의 "일기 없음" sentinel.
 */
export function buildDiaryExcerpt(
  entries: readonly DiaryEntry[],
  month: string,
  cal: DayCalendar = localDayCalendar,
  limits?: ExcerptLimits,
): string {
  const profile = speechPromptProfile();
  const lim = limits ?? profile.excerptLimits;
  const inMonth = entries
    .filter((e) => monthKeyOf(e.at, cal) === month)
    .slice()
    .sort((a, b) => a.at - b.at);
  if (inMonth.length === 0) return profile.noDiaryText;

  // 접두 + 개행 몫(절단 표시 `…`는 cutAtSentence가 자기 예산 안에서 쓴다).
  const overhead = LINE_PREFIX_CHARS + 1;

  for (let count = Math.min(lim.maxEntries, inMonth.length); count >= 1; count--) {
    const perEntry = Math.min(
      lim.perEntryChars,
      Math.max(profile.excerptMinBodyChars, Math.floor(lim.totalChars / count) - overhead),
    );
    const lines = evenIndices(inMonth.length, count).map((i) => {
      const e = inMonth[i];
      return `- ${cal.dayKey(e.at)}: ${cutAtSentence(e.body.trim(), perEntry)}`;
    });
    const cost = lines.reduce((n, l) => n + len(l) + 1, 0) - 1; // 마지막 개행 제외
    if (cost <= lim.totalChars) return lines.join("\n");
  }
  return profile.noDiaryText;
}

/** 수상 정보 블록. 작업시간은 시간 단위 반올림(초 단위 숫자는 소감에 방해된다).
 *  문장 조립은 언어별이라 프로필의 포매터에 맡긴다. */
function formatAwardInfo(record: AwardRecord, priorAwardCount: number): string {
  const stats = record.winner?.stats;
  return speechPromptProfile().formatAwardInfo({
    month: record.month,
    hours: Math.round((stats?.workedMs ?? 0) / 3_600_000),
    turns: stats?.turns ?? 0,
    activeDays: stats?.activeDays ?? 0,
    // 이번 수상을 포함한 통산 횟수 — 첫 수상이면 1회다.
    totalAwards: priorAwardCount + 1,
  });
}

/**
 * 한 달치 시상 레코드로 수상 소감 한 편을 생성한다. 영속화는 하지 않는다
 * (호출부인 awardsStore가 appendAwardSpeech로 저장한다).
 *
 * 요약기 OFF·CLI 미설치·타임아웃·실패는 던지지 않고 사유로 돌려준다.
 * `profile`이 없으면(캐릭터 삭제) 말투를 재현할 수 없으므로 생성하지 않는다.
 */
export async function generateSpeech(
  record: AwardRecord,
  profile: AgentProfile | undefined,
  priorAwardCount: number,
  deps: SpeechDeps = {},
): Promise<SpeechResult> {
  const winner = record.winner;
  if (!winner) return { ok: false, reason: "no-winner" };
  if (!profile) return { ok: false, reason: "profile-missing" };

  const summarizeFn =
    deps.summarizeFn ??
    // 일기와 같은 "diary" 목적 — 백그라운드 배치라 넉넉한 타임아웃이 필요하다(#66).
    ((provider, instruction, text) =>
      tauriApi.summarizeText(provider, instruction, text, "diary"));
  const loadDiaryFn = deps.loadDiaryFn ?? ((agentId: string) => tauriApi.loadDiary(agentId));
  const now = deps.now ?? Date.now;
  const cal = deps.cal ?? localDayCalendar;

  // 설정은 필요한 것만 늦게 읽는다(전부 주입한 테스트는 스토어를 안 건드린다).
  const settings =
    deps.provider === undefined || deps.enabled === undefined
      ? useAppStore.getState().appSettings
      : undefined;
  const enabled = deps.enabled ?? settings!.summarizerEnabled;
  if (!enabled) return { ok: false, reason: "disabled" };
  const provider = deps.provider ?? settings!.summaryProvider;

  // 일기를 못 읽는 것은 치명적이지 않다 — 통계만으로도 소감은 쓸 수 있다.
  let entries: DiaryEntry[] = [];
  try {
    entries = await loadDiaryFn(winner.agentId);
  } catch (err) {
    console.warn(`awards: diary load failed — writing speech from stats only (agent=${winner.agentId})`, err);
  }

  // 프롬프트·머리말·분량 상수는 한 벌이므로 한 번만 고른다.
  const prof = speechPromptProfile();

  // 고정 블록(성격 + 수상 정보)을 먼저 확보하고, 남는 예산 전부를 일기에 준다.
  const personality = cutAtSentence(
    profile.personalityPrompt?.trim() ?? "",
    prof.personalityMaxChars,
  );
  const head = [
    `${prof.headers.personality}\n${personality || prof.noneText}`,
    `${prof.headers.awardInfo}\n${formatAwardInfo(record, priorAwardCount)}`,
  ].join("\n\n");
  const diaryHeader = `\n\n${prof.headers.diary}\n`;
  const limits =
    deps.limits ??
    excerptLimitsFor(prof.promptBudgetChars - len(head) - len(diaryHeader), prof.excerptLimits);

  const userText = `${head}${diaryHeader}${buildDiaryExcerpt(entries, record.month, cal, limits)}`;

  try {
    const raw = await summarizeFn(provider, prof.systemPrompt, userText);
    const sanitized = sanitizeDiaryBody(raw);
    if (sanitized === null) return { ok: false, reason: "failed" };
    const text = clampSpeech(sanitized, prof.speechMaxChars, prof.speechMaxSentences);
    if (text === "") return { ok: false, reason: "failed" };
    return { ok: true, speech: { at: now(), provider, text } };
  } catch (err) {
    const message = err instanceof Error ? err.message : String(err);
    if (message.includes(`${provider}-not-found`)) {
      console.warn(`awards: ${provider} CLI not found — skipping award speech`);
      return { ok: false, reason: "cli-missing" };
    }
    // 정확히 "timeout"일 때만 — provider stderr에 timeout이 섞인 exit 에러를
    // 오분류하지 않는다(diaryGenerator와 같은 판정).
    if (message === "timeout") {
      console.warn(`awards: summarizer timeout (month=${record.month})`);
      return { ok: false, reason: "timeout" };
    }
    if (message.includes("summarizer-disabled")) return { ok: false, reason: "disabled" };
    console.warn(`awards: speech generation failed (month=${record.month})`, err);
    return { ok: false, reason: "failed" };
  }
}
