// 업무 예약의 순수 계산. 규칙 해석·다음 실행 시각·템플릿 판정·외부 변경 방어만 둔다.
// 실행(복제·보내기·자동 진행)은 scheduleRunner.ts가 맡는다. 스토어를 import하지 않는다.
import type { WorkItem, WorkSchedule, WorkScheduleRule } from "@shared/types";

/** 반복 간격의 하한·상한(분). Rust `SCHEDULE_MIN_MINUTES`·`SCHEDULE_MAX_MINUTES`와 같다. */
export const SCHEDULE_MIN_MINUTES = 5;
export const SCHEDULE_MAX_MINUTES = 30 * 24 * 60;
const WEEKDAYS = ["sun", "mon", "tue", "wed", "thu", "fri", "sat"];

function parseTime(time: string): { h: number; m: number } | undefined {
  const match = /^(\d{1,2}):(\d{2})$/u.exec(time);
  if (!match) return undefined;
  const h = Number(match[1]), m = Number(match[2]);
  return h < 24 && m < 60 ? { h, m } : undefined;
}

/** 로컬 `YYYY-MM-DD` + `HH:MM`을 epoch ms로. 없는 날짜(2월 30일 등)는 undefined. */
export function localDateTime(date: string, time: string): number | undefined {
  const d = /^(\d{4})-(\d{2})-(\d{2})$/u.exec(date);
  const t = parseTime(time);
  if (!d || !t) return undefined;
  const [y, mo, day] = [Number(d[1]), Number(d[2]), Number(d[3])];
  const at = new Date(y, mo - 1, day, t.h, t.m);
  return at.getFullYear() === y && at.getMonth() === mo - 1 && at.getDate() === day ? at.getTime() : undefined;
}

/**
 * 수신함 예약 명세(Rust `normalize_schedule`의 정규형)를 규칙으로.
 * `none`은 null(지우기), 읽을 수 없으면 undefined.
 */
export function parseScheduleSpec(spec: string): WorkScheduleRule | null | undefined {
  const [kind, ...rest] = spec.trim().split(/\s+/u);
  if (kind === "none" && !rest.length) return null;
  if (kind === "at" && rest.length === 2) {
    const at = localDateTime(rest[0], rest[1]);
    return at === undefined ? undefined : { kind: "once", at };
  }
  if (kind === "every" && rest.length === 1) {
    const match = /^(\d+)m$/u.exec(rest[0]);
    const minutes = match ? Number(match[1]) : NaN;
    return minutes >= SCHEDULE_MIN_MINUTES && minutes <= SCHEDULE_MAX_MINUTES ? { kind: "interval", minutes } : undefined;
  }
  if (kind === "daily" && rest.length === 1) return parseTime(rest[0]) ? { kind: "daily", time: rest[0] } : undefined;
  if (kind === "weekly" && rest.length === 2) {
    const days = rest[0].split(",").map((day) => WEEKDAYS.indexOf(day));
    if (!days.length || days.some((day) => day < 0) || !parseTime(rest[1])) return undefined;
    return { kind: "weekly", days: [...new Set(days)].sort((a, b) => a - b), time: rest[1] };
  }
  return undefined;
}

/** 저장하거나 켤 수 있는 규칙인가. 한 번 예약은 `now` 뒤여야 한다. */
export function validScheduleRule(rule: WorkScheduleRule, now = Date.now()): boolean {
  switch (rule.kind) {
    case "once": return Number.isFinite(rule.at) && rule.at > now;
    case "interval": return Number.isInteger(rule.minutes) && rule.minutes >= SCHEDULE_MIN_MINUTES && rule.minutes <= SCHEDULE_MAX_MINUTES;
    case "daily": return !!parseTime(rule.time);
    case "weekly": return !!parseTime(rule.time) && rule.days.length > 0 && rule.days.every((day) => Number.isInteger(day) && day >= 0 && day < 7);
  }
}

/**
 * `after`보다 뒤인 첫 실행 시각. 반복 간격은 직전 예정 시각(`previous`)에서 간격 단위로 이어 가
 * 실행이 늦어도 밀리지 않는다. 없으면(지난 한 번 예약) undefined.
 */
export function nextOccurrence(rule: WorkScheduleRule, after: number, previous?: number): number | undefined {
  switch (rule.kind) {
    case "once": return rule.at > after ? rule.at : undefined;
    case "interval": {
      const step = rule.minutes * 60_000;
      if (previous === undefined) return after + step;
      if (previous > after) return previous;
      return previous + (Math.floor((after - previous) / step) + 1) * step;
    }
    case "daily":
    case "weekly": {
      const time = parseTime(rule.time);
      if (!time) return undefined;
      const base = new Date(after);
      for (let offset = 0; offset <= 8; offset++) {
        const at = new Date(base.getFullYear(), base.getMonth(), base.getDate() + offset, time.h, time.m);
        if (at.getTime() > after && (rule.kind === "daily" || rule.days.includes(at.getDay()))) return at.getTime();
      }
      return undefined;
    }
  }
}

/** 규칙을 켠 예약. 켤 수 없는 규칙이면 undefined. */
export function enabledSchedule(rule: WorkScheduleRule, now = Date.now(), previous?: WorkSchedule): WorkSchedule | undefined {
  if (!validScheduleRule(rule, now)) return undefined;
  const nextAt = nextOccurrence(rule, now);
  if (nextAt === undefined) return undefined;
  return { ...previous, rule, enabled: true, nextAt };
}

/** 끈 예약. 다음 실행 시각을 버린다. */
export function disabledSchedule(schedule: WorkSchedule): WorkSchedule {
  const { nextAt: _nextAt, ...rest } = schedule;
  return { ...rest, enabled: false };
}

/** 반복 예약(템플릿)인가. 한 번 예약은 그 업무 자체를 실행하므로 템플릿이 아니다. */
export function isRecurring(item: Pick<WorkItem, "schedule">): boolean {
  return !!item.schedule && item.schedule.rule.kind !== "once";
}

/**
 * 루트가 직접·간접으로 기다리는 업무 id(선행 작업의 전이 폐포). 목록에 없는(방금 지워진) id도 넣어
 * 외부 삭제를 실행 내용 변경으로 알아챌 수 있게 한다. 있는 업무만 보면 `autoRun.planItems`와 같은 집합이다.
 */
export function planMemberIds(root: Pick<WorkItem, "id" | "dependsOn">, byId: ReadonlyMap<string, Pick<WorkItem, "id" | "dependsOn">>): string[] {
  const seen = new Set<string>();
  const stack = [...root.dependsOn ?? []];
  while (stack.length) {
    const id = stack.pop()!;
    if (id === root.id || seen.has(id)) continue;
    seen.add(id);
    stack.push(...byId.get(id)?.dependsOn ?? []);
  }
  return [...seen];
}

/**
 * 이 업무가 속한 반복 예약 템플릿(자신이 템플릿이거나 템플릿 플랜의 단계). 템플릿은 회차마다
 * 복제해 실행하므로 원본을 직접 보내거나 자동 진행하지 않는다.
 */
export function scheduleTemplateOf(item: Pick<WorkItem, "id">, items: WorkItem[]): WorkItem | undefined {
  const byId = new Map(items.map((other) => [other.id, other]));
  return items.find((other) => isRecurring(other) && (other.id === item.id || planMemberIds(other, byId).includes(item.id)));
}

/** 템플릿의 지난 회차 중 아직 끝나지 않은 것이 있는가(할 일·진행 중, 또는 끝나지 않은 자동 진행). */
export function hasOpenRun(templateId: string, items: WorkItem[]): boolean {
  return items.some((item) => item.scheduledFrom === templateId
    && (item.status === "todo" || item.status === "in_progress" || (!!item.autoRun && item.autoRun.state !== "finished")));
}

/** 켜진 예약 중 실행 내용(자신이나 플랜 단계)이 `touched`에 걸린 것을 끈다. 바뀔 것이 없으면 같은 배열. */
export function disableTouchedSchedules(items: WorkItem[], touched: ReadonlySet<string>): WorkItem[] {
  if (!touched.size) return items;
  const byId = new Map(items.map((item) => [item.id, item]));
  let changed = false;
  const next = items.map((item) => {
    if (!item.schedule?.enabled) return item;
    if (!touched.has(item.id) && !planMemberIds(item, byId).some((id) => touched.has(id))) return item;
    changed = true;
    return { ...item, schedule: disabledSchedule(item.schedule) };
  });
  return changed ? next : items;
}

/** 실행 내용으로 보는 필드. 사람 없이 바뀌면 켜진 예약을 끈다(상태·정렬·실행 기록은 뺀다). */
export const RUN_CONTENT_KEYS = ["title", "body", "assigneeAgentId", "agent", "dependsOn", "plan", "maxFollowUps", "permissionWaitMinutes", "repo"] as const;
