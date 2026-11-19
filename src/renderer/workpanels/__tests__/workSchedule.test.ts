import { describe, expect, it } from "vitest";
import type { WorkItem } from "@shared/types";
import {
  disableTouchedSchedules, enabledSchedule, hasOpenRun, localDateTime, nextOccurrence, parseScheduleSpec, scheduleTemplateOf, validScheduleRule,
} from "../workSchedule";

const at = (y: number, mo: number, d: number, h = 0, m = 0) => new Date(y, mo - 1, d, h, m).getTime();
function item(id: string, patch: Partial<WorkItem> = {}): WorkItem {
  return { id, title: id, body: "", status: "todo", order: 0, sessionIds: [], createdAt: 1, updatedAt: 1, ...patch };
}

describe("예약 명세", () => {
  it("Rust 정규형을 규칙으로 읽는다", () => {
    expect(parseScheduleSpec("at 2026-11-20 09:05")).toEqual({ kind: "once", at: at(2026, 11, 20, 9, 5) });
    expect(parseScheduleSpec("every 90m")).toEqual({ kind: "interval", minutes: 90 });
    expect(parseScheduleSpec("daily 07:30")).toEqual({ kind: "daily", time: "07:30" });
    expect(parseScheduleSpec("weekly wed,mon 09:00")).toEqual({ kind: "weekly", days: [1, 3], time: "09:00" });
    expect(parseScheduleSpec("none")).toBeNull();
    for (const bad of ["", "every 4m", "daily 25:00", "at 2026-02-30 09:00", "weekly funday 09:00", "hourly"]) expect(parseScheduleSpec(bad)).toBeUndefined();
  });
  it("없는 날짜는 받지 않는다", () => {
    expect(localDateTime("2026-02-29", "09:00")).toBeUndefined();
    expect(localDateTime("2028-02-29", "09:00")).toBe(at(2028, 2, 29, 9));
  });
});

describe("다음 실행 시각", () => {
  const now = at(2026, 11, 19, 10, 0); // 목요일
  it("매일·매주는 지금보다 뒤의 첫 시각이다", () => {
    expect(nextOccurrence({ kind: "daily", time: "09:00" }, now)).toBe(at(2026, 11, 20, 9));
    expect(nextOccurrence({ kind: "daily", time: "10:30" }, now)).toBe(at(2026, 11, 19, 10, 30));
    expect(nextOccurrence({ kind: "daily", time: "10:00" }, now)).toBe(at(2026, 11, 20, 10));
    expect(nextOccurrence({ kind: "weekly", days: [1], time: "09:00" }, now)).toBe(at(2026, 11, 23, 9));
    expect(nextOccurrence({ kind: "weekly", days: [4], time: "09:00" }, now)).toBe(at(2026, 11, 26, 9));
  });
  it("간격은 직전 예정 시각에서 이어 가 늦게 실행해도 밀리지 않는다", () => {
    const rule = { kind: "interval" as const, minutes: 30 };
    expect(nextOccurrence(rule, now)).toBe(now + 30 * 60_000);
    expect(nextOccurrence(rule, now + 2 * 60_000, now)).toBe(now + 30 * 60_000);
    // 여러 회차를 놓쳤으면 한 번으로 합쳐 다음 칸으로 건너뛴다.
    expect(nextOccurrence(rule, now + 95 * 60_000, now)).toBe(now + 120 * 60_000);
  });
  it("한 번 예약은 지나면 다음이 없다", () => {
    expect(nextOccurrence({ kind: "once", at: now + 1 }, now)).toBe(now + 1);
    expect(nextOccurrence({ kind: "once", at: now }, now)).toBeUndefined();
    expect(validScheduleRule({ kind: "once", at: now - 1 }, now)).toBe(false);
    expect(enabledSchedule({ kind: "once", at: now - 1 }, now)).toBeUndefined();
    expect(enabledSchedule({ kind: "weekly", days: [], time: "09:00" }, now)).toBeUndefined();
    expect(enabledSchedule({ kind: "daily", time: "09:00" }, now, { rule: { kind: "daily", time: "08:00" }, enabled: false, lastRunAt: 5 }))
      .toEqual({ rule: { kind: "daily", time: "09:00" }, enabled: true, nextAt: at(2026, 11, 20, 9), lastRunAt: 5 });
  });
});

describe("템플릿", () => {
  const daily = { rule: { kind: "daily" as const, time: "09:00" }, enabled: true, nextAt: 1 };
  it("반복 예약 업무와 그 플랜 단계가 템플릿이다. 한 번 예약은 아니다", () => {
    const items = [item("a"), item("b", { dependsOn: ["a"] }), item("t", { dependsOn: ["b"], schedule: daily }), item("o", { schedule: { rule: { kind: "once", at: 5 }, enabled: true } }), item("x")];
    expect(["a", "b", "t"].map((id) => scheduleTemplateOf({ id }, items)?.id)).toEqual(["t", "t", "t"]);
    expect(scheduleTemplateOf({ id: "o" }, items)).toBeUndefined();
    expect(scheduleTemplateOf({ id: "x" }, items)).toBeUndefined();
  });
  it("끝나지 않은 회차를 찾는다", () => {
    expect(hasOpenRun("t", [item("c", { scheduledFrom: "t", status: "review" })])).toBe(false);
    expect(hasOpenRun("t", [item("c", { scheduledFrom: "t", status: "in_progress" })])).toBe(true);
    expect(hasOpenRun("t", [item("c", { scheduledFrom: "t", status: "review", autoRun: { state: "paused", judgeSessionId: "j", judgeStarted: false, followUps: 0, costUsd: 0, log: [], startedAt: 1 } })])).toBe(true);
  });
  it("실행 내용(자신·단계·지워진 단계)이 바뀐 켜진 예약만 끈다", () => {
    const items = [item("a"), item("t", { dependsOn: ["a", "gone"], schedule: daily }), item("u", { schedule: daily })];
    expect(disableTouchedSchedules(items, new Set())).toBe(items);
    for (const touched of ["a", "t", "gone"]) {
      const next = disableTouchedSchedules(items, new Set([touched]));
      expect(next.find((i) => i.id === "t")!.schedule).toEqual({ rule: daily.rule, enabled: false });
      expect(next.find((i) => i.id === "u")!.schedule).toBe(daily);
    }
  });
});
