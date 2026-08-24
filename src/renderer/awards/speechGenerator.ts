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
// 됐는지가 안 나오기 때문이다. 다만 한 달치 일기 전부는 프롬프트로 감당이 안 돼
// 균등 간격 샘플링 + 편당 절단 + 총 예산으로 줄인다(최신 몇 편만 쓰면 월초의
// 일이 통째로 빠져 "한 달을 돌아보는" 소감이 되지 않는다).
import { useAppStore } from "../store/appStore";
import { tauriApi } from "../ipc/tauriApi";
import { sanitizeDiaryBody } from "../diary/diaryGenerator";
import { localDayCalendar, type DayCalendar } from "../analytics/aggregate";
import { monthKeyOf } from "./selection";
import type {
  AgentProfile,
  AwardRecord,
  AwardSpeech,
  DiaryEntry,
  SummaryProvider,
} from "@shared/types";

export const AWARD_SPEECH_SYSTEM_PROMPT =
  "너는 사내 시상식에서 '이 달의 우수사원'으로 호명된 캐릭터 본인이다. 아래 [성격]을 문체로 삼아, [수상 정보]와 [지난달 일기]를 근거로 1인칭 한국어 수상 소감을 써라. 분량은 2~4문장. [수상 정보]의 수치 하나쯤은 자연스럽게 녹여도 좋지만 통계를 나열하지 마라. [성격]이 비어 있으면 담백한 중립 문체로 써라. 규칙: 한국어만, 사과·메타발언·머리말·따옴표·마크다운 금지, 소감 본문만 출력.";

/** 일기 발췌 분량 한도. */
export interface ExcerptLimits {
  /** 샘플링 없이 전부 담는 최대 편수. 넘으면 이 수만큼 균등 간격으로 뽑는다. */
  maxEntries: number;
  /** 한 편당 담는 최대 글자 수. 넘으면 자르고 `…`를 붙인다. */
  perEntryChars: number;
  /** 발췌 전체 글자 예산. 넘기 직전에 멈춘다. */
  totalChars: number;
}

export const DEFAULT_EXCERPT_LIMITS: ExcerptLimits = {
  maxEntries: 20,
  perEntryChars: 300,
  totalChars: 8_000,
};

/** 일기가 한 편도 없을 때 프롬프트에 넣는 자리 표시. */
const NO_DIARY = "(일기 없음)";

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

/** 편당 절단. 자른 경우에만 `…`를 붙인다. */
function truncate(body: string, max: number): string {
  const chars = Array.from(body);
  if (chars.length <= max) return body;
  return `${chars.slice(0, max).join("")}…`;
}

/**
 * 그 달 일기를 프롬프트에 넣을 발췌 한 덩이로 만든다.
 *
 * `month`에 속한 항목만 남기고(월 판정은 selection의 monthKeyOf 재사용),
 * 편수가 한도를 넘으면 월 전체 흐름이 보이게 균등 간격으로 뽑는다(최신 우선이
 * 아니다). 각 편은 날짜를 붙여 절단하고, 총 예산을 넘기 직전에 멈춘다.
 * 남는 게 없으면 `(일기 없음)`.
 */
export function buildDiaryExcerpt(
  entries: readonly DiaryEntry[],
  month: string,
  cal: DayCalendar = localDayCalendar,
  limits: ExcerptLimits = DEFAULT_EXCERPT_LIMITS,
): string {
  const inMonth = entries
    .filter((e) => monthKeyOf(e.at, cal) === month)
    .slice()
    .sort((a, b) => a.at - b.at);
  if (inMonth.length === 0) return NO_DIARY;

  const picked = evenIndices(inMonth.length, limits.maxEntries).map((i) => inMonth[i]);

  const lines: string[] = [];
  let used = 0;
  for (const e of picked) {
    const line = `- ${cal.dayKey(e.at)}: ${truncate(e.body.trim(), limits.perEntryChars)}`;
    const cost = Array.from(line).length + (lines.length > 0 ? 1 : 0); // 줄바꿈 몫
    if (used + cost > limits.totalChars) break;
    lines.push(line);
    used += cost;
  }
  return lines.length > 0 ? lines.join("\n") : NO_DIARY;
}

/** 수상 정보 블록. 작업시간은 시간 단위 반올림(초 단위 숫자는 소감에 방해된다). */
function formatAwardInfo(record: AwardRecord, priorAwardCount: number): string {
  const stats = record.winner?.stats;
  const hours = Math.round((stats?.workedMs ?? 0) / 3_600_000);
  return [
    `월: ${record.month}`,
    `작업 시간: 약 ${hours}시간`,
    `턴 수: ${stats?.turns ?? 0}`,
    `활동일: ${stats?.activeDays ?? 0}일`,
    // 이번 수상을 포함한 통산 횟수 — 첫 수상이면 1회다.
    `통산 수상: ${priorAwardCount + 1}회(이번 포함)`,
  ].join("\n");
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
  const limits = deps.limits ?? DEFAULT_EXCERPT_LIMITS;

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
    console.warn(`awards: 일기 로드 실패 — 통계만으로 소감 생성(agent=${winner.agentId})`, err);
  }

  const personality = profile.personalityPrompt?.trim() ?? "";
  const userText = [
    `[성격]\n${personality || "(없음)"}`,
    `[수상 정보]\n${formatAwardInfo(record, priorAwardCount)}`,
    `[지난달 일기]\n${buildDiaryExcerpt(entries, record.month, cal, limits)}`,
  ].join("\n\n");

  try {
    const raw = await summarizeFn(provider, AWARD_SPEECH_SYSTEM_PROMPT, userText);
    const text = sanitizeDiaryBody(raw);
    if (text === null) return { ok: false, reason: "failed" };
    return { ok: true, speech: { at: now(), provider, text } };
  } catch (err) {
    const message = err instanceof Error ? err.message : String(err);
    if (message.includes(`${provider}-not-found`)) {
      console.warn(`awards: ${provider} CLI 미설치 — 수상 소감 건너뜀`);
      return { ok: false, reason: "cli-missing" };
    }
    // 정확히 "timeout"일 때만 — provider stderr에 timeout이 섞인 exit 에러를
    // 오분류하지 않는다(diaryGenerator와 같은 판정).
    if (message === "timeout") {
      console.warn(`awards: 요약기 타임아웃(month=${record.month})`);
      return { ok: false, reason: "timeout" };
    }
    if (message.includes("summarizer-disabled")) return { ok: false, reason: "disabled" };
    console.warn(`awards: 수상 소감 생성 실패(month=${record.month})`, err);
    return { ok: false, reason: "failed" };
  }
}
