// src/renderer/analytics/aggregate.ts
//
// 세션 원천 이벤트(SessionEventRecord[]) → 분석 패널이 그리는 일별·에이전트별
// 집계. 전부 순수 함수라 vitest로 경계(자정 분할, stop 유실, 다중 prompt,
// 로컬 날짜 귀속, 삭제 에이전트 폴백)를 값싸게 검증한다.
// 설계: docs/session-analytics-design.md §4.3.
//
// 집계는 로컬 날짜 기준이다(사용자는 로컬 하루 단위로 생각). 타임존 의존을
// `DayCalendar`로 주입 가능하게 빼서, 테스트에서 고정 오프셋으로 자정 경계를
// 결정적으로 검증한다. 기본은 시스템 로컬(`localDayCalendar`).
import type { AgentProfile, SessionEventRecord } from "@shared/types";
import { grayForIndex, representativeColor } from "./colors";

const DAY_MS = 86_400_000;
const pad2 = (n: number): string => String(n).padStart(2, "0");

/** 로컬 날짜 계산 경계. 주입해서 테스트에서 타임존을 고정한다. */
export interface DayCalendar {
  /** `at`(epoch ms)이 속한 로컬 날짜 키 "YYYY-MM-DD". */
  dayKey(at: number): string;
  /** `at`이 속한 로컬 날짜의 자정(로컬 00:00) epoch ms. */
  startOfDay(at: number): number;
}

/** 시스템 로컬 타임존 기준 캘린더(프로덕션 기본). */
export const localDayCalendar: DayCalendar = {
  dayKey(at) {
    const d = new Date(at);
    return `${d.getFullYear()}-${pad2(d.getMonth() + 1)}-${pad2(d.getDate())}`;
  },
  startOfDay(at) {
    const d = new Date(at);
    return new Date(d.getFullYear(), d.getMonth(), d.getDate()).getTime();
  },
};

/**
 * 고정 오프셋(분) 캘린더 — 테스트에서 타임존을 못박아 자정 경계를 결정적으로
 * 검증하기 위한 것. 예: `fixedOffsetCalendar(540)` = UTC+9(KST). DST는 다루지
 * 않는다(고정 오프셋).
 */
export function fixedOffsetCalendar(offsetMinutes: number): DayCalendar {
  const off = offsetMinutes * 60_000;
  return {
    dayKey(at) {
      return new Date(at + off).toISOString().slice(0, 10);
    },
    startOfDay(at) {
      const shifted = at + off;
      return Math.floor(shifted / DAY_MS) * DAY_MS - off;
    },
  };
}

/** 재구성된 하나의 작업 턴(prompt 시작 ~ 마감 시각). */
export interface Turn {
  agentId: string;
  sessionId: string;
  startAt: number;
  endAt: number;
}

/**
 * 표시 대상 시간 창(epoch ms, 양끝 포함). 경계를 걸친 턴을 온전히 복원하려고
 * 백엔드는 이 창보다 앞선 lookback 이벤트까지 함께 넘겨준다. 집계는 이 창으로
 * 턴을 클립해 창 안 몫만 귀속한다(설계: 기간 경계 턴 유실 방지).
 */
export interface AggregateRange {
  fromAt: number;
  toAt: number;
}

/**
 * 턴을 `[fromAt, toAt]`로 클립한다. 창과 겹치지 않으면(또는 클립 후 길이 0)
 * `null`. lookback으로 시작이 창 밖인 턴은 시작을 `fromAt`으로 당겨 창 안
 * 몫만 남긴다.
 */
function clipTurn(turn: Turn, range: AggregateRange): Turn | null {
  const startAt = Math.max(turn.startAt, range.fromAt);
  const endAt = Math.min(turn.endAt, range.toAt);
  if (startAt >= endAt) return null;
  return { ...turn, startAt, endAt };
}

/** 에이전트별·일별 집계 셀. */
export interface AgentDailyStat {
  workedMs: number;
  turns: number;
  toolEvents: number;
}

/** 에이전트 표시 메타(이름·색·삭제 여부). */
export interface AgentMeta {
  agentId: string;
  name: string;
  color: string;
  /** 현재 프로필이 없어 스냅샷/ID로 폴백한 경우 true. */
  deleted: boolean;
}

/** 요약 표 한 행: 메타 + 기간 합계. */
export interface AgentSummary extends AgentMeta {
  workedMs: number;
  turns: number;
  toolEvents: number;
  /** 활동(작업/턴/도구)이 하나라도 있던 로컬 날짜 수. */
  activeDays: number;
}

/** 집계 결과 묶음. */
export interface AnalyticsData {
  meta: Record<string, AgentMeta>;
  /** date(로컬 "YYYY-MM-DD") → agentId → 셀. */
  daily: Record<string, Record<string, AgentDailyStat>>;
  /** 작업시간 내림차순 요약. */
  summary: AgentSummary[];
}

function byAtRunSeq(a: SessionEventRecord, b: SessionEventRecord): number {
  if (a.at !== b.at) return a.at - b.at;
  if (a.runId !== b.runId) return a.runId < b.runId ? -1 : 1;
  return a.seq - b.seq;
}

function shortId(id: string): string {
  return id.length > 8 ? id.slice(0, 8) : id;
}

/**
 * `(agentId, sessionId)`별로 시간순 처리하며 작업 턴을 재구성한다.
 * - `prompt`: 열린 턴이 없으면 턴 시작. 이미 열려 있으면 무시(연속 prompt = 같은 턴).
 * - `stop`: 열린 턴을 닫는다. 열린 턴이 없으면 무시.
 * - `session_state`의 `exited`/`disposed`: 열린 턴이 있으면 그 시각으로 강제 마감(stop 유실 대비).
 * - 끝까지 안 닫힌 턴은 세션의 마지막 이벤트 시각으로 마감한다.
 * 길이가 0 이하인 턴(시작==마감 등)은 버린다.
 */
export function reconstructTurns(events: readonly SessionEventRecord[]): Turn[] {
  const groups = new Map<string, SessionEventRecord[]>();
  for (const ev of events) {
    const key = `${ev.agentId}|${ev.sessionId}`;
    let bucket = groups.get(key);
    if (!bucket) {
      bucket = [];
      groups.set(key, bucket);
    }
    bucket.push(ev);
  }

  const turns: Turn[] = [];
  for (const group of groups.values()) {
    const sorted = [...group].sort(byAtRunSeq);
    const first = sorted[0];
    const last = sorted[sorted.length - 1];
    let openStart: number | null = null;

    const close = (endAt: number): void => {
      if (openStart !== null && endAt > openStart) {
        turns.push({
          agentId: first.agentId,
          sessionId: first.sessionId,
          startAt: openStart,
          endAt,
        });
      }
      openStart = null;
    };

    for (const ev of sorted) {
      if (ev.kind === "prompt") {
        if (openStart === null) openStart = ev.at;
      } else if (ev.kind === "stop") {
        close(ev.at);
      } else if (
        ev.kind === "session_state" &&
        (ev.state === "exited" || ev.state === "disposed")
      ) {
        close(ev.at);
      }
    }
    // 데이터 끝까지 안 닫힌 턴 → 세션 마지막 이벤트 시각으로 마감.
    if (openStart !== null) close(last.at);
  }
  return turns;
}

/**
 * 턴 `[startAt, endAt)`을 로컬 날짜 경계에서 분할해 (날짜, ms) 조각들로 나눈다.
 * 자정을 걸치는 턴이 일별 합산에 올바르게 쪼개져 들어가게 한다.
 */
export function splitTurnByDay(
  startAt: number,
  endAt: number,
  cal: DayCalendar,
): Array<{ date: string; ms: number }> {
  const slices: Array<{ date: string; ms: number }> = [];
  let cursor = startAt;
  while (cursor < endAt) {
    const dayStart = cal.startOfDay(cursor);
    // 다음 로컬 자정: 현재 날 자정 + 26h로 확실히 다음 날에 들어간 뒤 그 날의
    // 자정을 취한다(고정 오프셋에선 정확히 +24h, 방어적으로 여유를 둔다).
    const nextMidnight = cal.startOfDay(dayStart + DAY_MS + 2 * 3_600_000);
    const sliceEnd = Math.min(endAt, nextMidnight);
    slices.push({ date: cal.dayKey(cursor), ms: sliceEnd - cursor });
    cursor = sliceEnd;
  }
  return slices;
}

function ensureCell(
  daily: Record<string, Record<string, AgentDailyStat>>,
  date: string,
  agentId: string,
): AgentDailyStat {
  const perAgent = (daily[date] ??= {});
  return (perAgent[agentId] ??= { workedMs: 0, turns: 0, toolEvents: 0 });
}

/**
 * 로컬 날짜별·에이전트별 집계. 작업시간은 턴을 자정 경계로 분할해 귀속하고,
 * 턴 수는 턴 시작 시각의 로컬 날짜에, 도구 이벤트는 발생 시각의 로컬 날짜에
 * 귀속한다.
 *
 * `range`가 주어지면: 턴은 창으로 클립해 창 안 몫만 작업시간에 귀속하고(겹치지
 * 않는 턴은 버림), 턴 수는 `dayKey(max(startAt, fromAt))`에, 도구 이벤트는 창
 * 안 이벤트만 귀속한다. `range` 미지정 시 전체를 그대로 집계(기존 동작).
 */
export function dailySummary(
  events: readonly SessionEventRecord[],
  turns: readonly Turn[],
  cal: DayCalendar,
  range?: AggregateRange,
): Record<string, Record<string, AgentDailyStat>> {
  const daily: Record<string, Record<string, AgentDailyStat>> = {};
  for (const turn of turns) {
    const clipped = range ? clipTurn(turn, range) : turn;
    if (!clipped) continue; // 창과 겹치지 않는 턴은 버림
    for (const slice of splitTurnByDay(clipped.startAt, clipped.endAt, cal)) {
      ensureCell(daily, slice.date, turn.agentId).workedMs += slice.ms;
    }
    // 턴 수: 창이 있으면 창 안으로 당긴 시작(=max(startAt, fromAt))의 로컬 날짜.
    const turnDayAt = range ? Math.max(turn.startAt, range.fromAt) : turn.startAt;
    ensureCell(daily, cal.dayKey(turnDayAt), turn.agentId).turns += 1;
  }
  for (const ev of events) {
    if (ev.kind !== "tool") continue;
    if (range && (ev.at < range.fromAt || ev.at > range.toAt)) continue; // 창 밖 이벤트 제외
    ensureCell(daily, cal.dayKey(ev.at), ev.agentId).toolEvents += 1;
  }
  return daily;
}

/**
 * 에이전트별 표시 메타. 이름은 현재 프로필 우선 → 없으면 마지막
 * `session_started.agentName` → 그것도 없으면 ID 축약. 색은 프로필 대표색,
 * 삭제된 에이전트는 중립 회색을 등장 순서로 순환 배정한다.
 *
 * 이름 폴백은 lookback을 포함한 전체 이벤트에서 찾는다(창 앞 스냅샷도 유효).
 * `activeAgents`가 주어지면 그 집합에 든 에이전트만 메타로 내보낸다 — lookback
 * 에만 등장한 유령 에이전트가 요약/색에 새지 않게 하기 위함이다.
 */
export function agentMeta(
  events: readonly SessionEventRecord[],
  profiles: Record<string, AgentProfile>,
  activeAgents?: ReadonlySet<string>,
): Record<string, AgentMeta> {
  const firstSeen = new Map<string, number>();
  const lastStartedName = new Map<string, string>();
  const lastStartedAt = new Map<string, number>();
  for (const ev of events) {
    if (!firstSeen.has(ev.agentId)) firstSeen.set(ev.agentId, ev.at);
    if (ev.kind === "session_started" && ev.agentName) {
      const prev = lastStartedAt.get(ev.agentId) ?? -Infinity;
      if (ev.at >= prev) {
        lastStartedName.set(ev.agentId, ev.agentName);
        lastStartedAt.set(ev.agentId, ev.at);
      }
    }
  }
  // 회색 순환 배정 결정성을 위해 등장(첫 이벤트) 순서, 동시각은 ID로 안정 정렬.
  const ids = [...firstSeen.keys()]
    .filter((id) => !activeAgents || activeAgents.has(id))
    .sort((a, b) => {
      const fa = firstSeen.get(a) ?? 0;
      const fb = firstSeen.get(b) ?? 0;
      if (fa !== fb) return fa - fb;
      return a < b ? -1 : a > b ? 1 : 0;
    });

  const meta: Record<string, AgentMeta> = {};
  let grayIndex = 0;
  for (const id of ids) {
    const profile = profiles[id];
    if (profile) {
      meta[id] = {
        agentId: id,
        name: profile.name,
        color: representativeColor(profile),
        deleted: false,
      };
    } else {
      meta[id] = {
        agentId: id,
        name: lastStartedName.get(id) ?? shortId(id),
        color: grayForIndex(grayIndex++),
        deleted: true,
      };
    }
  }
  return meta;
}

/**
 * 최상위 집계: 턴 재구성 → 일별 요약 → 에이전트 메타 → 작업시간 내림차순 요약.
 *
 * `range`가 주어지면 턴 재구성은 lookback 포함 전체 이벤트로 하되(경계 걸친 턴
 * 복원), 일별 집계와 요약/메타는 창 `[fromAt, toAt]`으로 한정한다. 요약 대상은
 * "창 안 이벤트 보유 ∪ 창에 걸친(클립된) 턴 보유" 에이전트로 제한한다.
 */
export function aggregate(
  events: readonly SessionEventRecord[],
  profiles: Record<string, AgentProfile>,
  cal: DayCalendar = localDayCalendar,
  range?: AggregateRange,
): AnalyticsData {
  const turns = reconstructTurns(events);
  const daily = dailySummary(events, turns, cal, range);

  // 요약/메타 대상 에이전트: 창이 있으면 창 안 활동 보유자로 한정.
  let activeAgents: Set<string> | undefined;
  if (range) {
    activeAgents = new Set<string>();
    for (const ev of events) {
      if (ev.at >= range.fromAt && ev.at <= range.toAt) activeAgents.add(ev.agentId);
    }
    for (const turn of turns) {
      if (clipTurn(turn, range)) activeAgents.add(turn.agentId);
    }
  }
  const meta = agentMeta(events, profiles, activeAgents);

  interface Totals {
    workedMs: number;
    turns: number;
    toolEvents: number;
    days: Set<string>;
  }
  const totals = new Map<string, Totals>();
  const totalFor = (agentId: string): Totals => {
    let t = totals.get(agentId);
    if (!t) {
      t = { workedMs: 0, turns: 0, toolEvents: 0, days: new Set() };
      totals.set(agentId, t);
    }
    return t;
  };
  for (const [date, perAgent] of Object.entries(daily)) {
    for (const [agentId, stat] of Object.entries(perAgent)) {
      const t = totalFor(agentId);
      t.workedMs += stat.workedMs;
      t.turns += stat.turns;
      t.toolEvents += stat.toolEvents;
      if (stat.workedMs > 0 || stat.turns > 0 || stat.toolEvents > 0) t.days.add(date);
    }
  }

  const summary: AgentSummary[] = Object.values(meta)
    .map((m) => {
      const t = totals.get(m.agentId);
      return {
        ...m,
        workedMs: t?.workedMs ?? 0,
        turns: t?.turns ?? 0,
        toolEvents: t?.toolEvents ?? 0,
        activeDays: t?.days.size ?? 0,
      };
    })
    .sort((a, b) => {
      if (b.workedMs !== a.workedMs) return b.workedMs - a.workedMs;
      return a.name < b.name ? -1 : a.name > b.name ? 1 : 0;
    });

  return { meta, daily, summary };
}

/**
 * `fromAt..=toAt`를 로컬 날짜 키 목록으로 나열한다(빈 날 포함, 차트 x축용).
 */
export function dayRange(fromAt: number, toAt: number, cal: DayCalendar = localDayCalendar): string[] {
  if (fromAt > toAt) return [];
  const days: string[] = [];
  let cursor = cal.startOfDay(fromAt);
  const endKey = cal.dayKey(toAt);
  // 기간 상한(≤31일)보다 넉넉한 가드로 무한 루프를 막는다.
  for (let guard = 0; guard < 400; guard++) {
    const key = cal.dayKey(cursor);
    days.push(key);
    if (key === endKey) break;
    cursor = cal.startOfDay(cursor + DAY_MS + 2 * 3_600_000);
  }
  return days;
}
