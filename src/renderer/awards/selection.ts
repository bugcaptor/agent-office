// src/renderer/awards/selection.ts
//
// "이 달의 우수사원" 선정의 순수 로직. LLM은 전혀 쓰지 않는다 — 같은 입력이면
// 언제 돌려도 같은 수상자가 나와야 시상 기록(write-once)이 믿을 만해진다.
//
// 입력은 analytics 파이프라인(aggregate.ts)의 `AgentSummary[]`를 그대로 쓴다.
// 시상 전용 집계를 새로 만들지 않는 이유: 분석 패널에 보이는 수치와 시상 근거가
// 어긋나면 사용자가 납득할 수 없기 때문이다.
//
// 월 경계 계산도 여기 모았다. 타임존 의존은 aggregate.ts와 같은 `DayCalendar`로
// 주입 가능하게 두어, 테스트에서 `fixedOffsetCalendar(540)`로 못박아 검증한다.
import type { AgentProfile } from "@shared/types";
import {
  localDayCalendar,
  type AggregateRange,
  type AgentSummary,
  type DayCalendar,
} from "../analytics/aggregate";

const DAY_MS = 86_400_000;

/** 기본 최소 활동일. 이틀 반짝 일한 캐릭터가 한 달 수상자가 되지 않게 한다. */
export const DEFAULT_MIN_ACTIVE_DAYS = 3;

/** 기본 최소 작업시간(30분). 잠깐 켜 둔 세션만으로 수상하는 것을 막는다. */
export const DEFAULT_MIN_WORKED_MS = 1_800_000;

/** 순위표에 남기는 최대 인원(수상자 포함). */
export const LEADERBOARD_SIZE = 5;

/** 선정 임계. 부재 시 위 기본값. */
export interface SelectionOptions {
  minActiveDays?: number;
  minWorkedMs?: number;
}

/** 선정 결과. 자격자가 없으면 `winner: null`, `leaderboard: []`. */
export interface SelectionResult {
  winner: AgentSummary | null;
  /** 자격자 상위 `LEADERBOARD_SIZE`인(순위순, 수상자 포함). */
  leaderboard: AgentSummary[];
}

/** "YYYY-MM" 파싱. 형식이 어긋나면 던진다(호출부 버그를 조용히 넘기지 않는다). */
function parseMonthKey(monthKey: string): { year: number; month: number } {
  const m = /^(\d{4})-(\d{2})$/.exec(monthKey);
  if (!m) throw new Error(`잘못된 monthKey: ${monthKey}`);
  const year = Number(m[1]);
  const month = Number(m[2]);
  if (month < 1 || month > 12) throw new Error(`잘못된 monthKey: ${monthKey}`);
  return { year, month };
}

const pad2 = (n: number): string => String(n).padStart(2, "0");

/**
 * 그 달 1일의 로컬 자정(epoch ms).
 *
 * `DayCalendar`는 "이 시각이 속한 로컬 날짜/자정"만 알려주므로, 달력에서 역으로
 * 시각을 얻으려면 탐침이 필요하다. UTC 정오를 탐침으로 잡으면 |오프셋| < 12h인
 * 타임존에서는 곧바로 그 달 1일에 떨어지고, UTC+13/+14 같은 극단 오프셋에서만
 * 하루가 어긋난다 — 그 경우 달력이 말하는 날짜를 보고 하루씩 밀어 맞춘다.
 */
function monthStartAt(monthKey: string, cal: DayCalendar): number {
  const { year, month } = parseMonthKey(monthKey);
  const targetDay = `${monthKey}-01`;
  let probe = Date.UTC(year, month - 1, 1, 12, 0, 0, 0);
  // "YYYY-MM-DD"는 사전식 비교가 곧 날짜 비교다. 최대 ±2일이면 충분하지만
  // 여유를 둔다(무한 루프 방지 상한).
  for (let i = 0; i < 4; i++) {
    const key = cal.dayKey(probe);
    if (key === targetDay) break;
    probe += key < targetDay ? DAY_MS : -DAY_MS;
  }
  return cal.startOfDay(probe);
}

/**
 * "YYYY-MM"(로컬) → aggregate에 넘길 표시 창.
 * `[그 달 1일 00:00, 다음 달 1일 00:00 - 1ms]` — 양끝 포함 창이라는 `AggregateRange`
 * 계약에 맞춰 상한을 1ms 앞당긴다(다음 달 자정 이벤트가 이번 달에 섞이지 않게).
 */
export function monthRange(monthKey: string, cal: DayCalendar = localDayCalendar): AggregateRange {
  const fromAt = monthStartAt(monthKey, cal);
  const nextAt = monthStartAt(shiftMonth(monthKey, 1), cal);
  return { fromAt, toAt: nextAt - 1 };
}

/** epoch ms → 로컬 "YYYY-MM". */
export function monthKeyOf(at: number, cal: DayCalendar = localDayCalendar): string {
  return cal.dayKey(at).slice(0, 7);
}

/** monthKey를 `delta`달 이동한다(음수 = 과거). 연 경계를 넘어간다. */
export function shiftMonth(monthKey: string, delta: number): string {
  const { year, month } = parseMonthKey(monthKey);
  // 0-based 통산 월수로 옮긴 뒤 이동 — 12로 나눈 나머지가 음수가 되지 않게
  // floor 나눗셈을 쓴다.
  const total = year * 12 + (month - 1) + delta;
  const y = Math.floor(total / 12);
  const m = total - y * 12 + 1;
  return `${String(y).padStart(4, "0")}-${pad2(m)}`;
}

/**
 * 프로필이 봇 모드로 설정돼 있는지.
 *
 * `AgentProfile.bot`은 봇의 지속 설정만 담고, 아무것도 설정하지 않았으면 아예
 * 필드가 없다(`generate.ts`의 `buildBotConfig`가 undefined를 돌려준다). 다만 옛
 * 프로필에 빈 껍데기가 남아 있을 수 있어, 실질적인 설정이 하나라도 있을 때만
 * 봇으로 본다. 봇은 사람이 부린 캐릭터가 아니므로 시상 후보에서 뺀다.
 */
export function isBotProfile(profile: AgentProfile | undefined): boolean {
  const bot = profile?.bot;
  if (!bot) return false;
  return (
    (bot.slug ?? "").trim().length > 0 ||
    (bot.whitelist?.length ?? 0) > 0 ||
    bot.pollIntervalSec !== undefined ||
    bot.idleQuietMs !== undefined
  );
}

/**
 * 수상자와 순위표를 고른다. 결정적이고 입력을 변형하지 않는다.
 *
 * 제외: 삭제된 캐릭터, 봇 모드 설정, 최소 활동일/작업시간 미달.
 * 퇴근(clockedOut)은 제외 사유가 아니다 — 그 달에 일한 사실은 남는다.
 * 정렬: workedMs ↓ → turns ↓ → activeDays ↓ → agentId 사전식 ↑.
 */
export function pickWinner(
  summary: readonly AgentSummary[],
  profiles: Record<string, AgentProfile>,
  opts: SelectionOptions = {},
): SelectionResult {
  const minActiveDays = opts.minActiveDays ?? DEFAULT_MIN_ACTIVE_DAYS;
  const minWorkedMs = opts.minWorkedMs ?? DEFAULT_MIN_WORKED_MS;

  const eligible = summary.filter((s) => {
    if (s.deleted) return false;
    if (isBotProfile(profiles[s.agentId])) return false;
    if (s.activeDays < minActiveDays) return false;
    if (s.workedMs < minWorkedMs) return false;
    return true;
  });

  // filter가 이미 새 배열을 만들지만, 원본을 건드리지 않는다는 계약을 코드에서
  // 읽히게 두려고 정렬은 이 사본 위에서 한다.
  const ranked = eligible.slice().sort(compareForAward);
  if (ranked.length === 0) return { winner: null, leaderboard: [] };
  return { winner: ranked[0], leaderboard: ranked.slice(0, LEADERBOARD_SIZE) };
}

/** 시상 순위 비교자(작을수록 상위). 동점은 agentId 사전식으로 완전히 깬다. */
export function compareForAward(a: AgentSummary, b: AgentSummary): number {
  if (a.workedMs !== b.workedMs) return b.workedMs - a.workedMs;
  if (a.turns !== b.turns) return b.turns - a.turns;
  if (a.activeDays !== b.activeDays) return b.activeDays - a.activeDays;
  return a.agentId < b.agentId ? -1 : a.agentId > b.agentId ? 1 : 0;
}
