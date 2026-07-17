// src/renderer/timeline/todayTotal.ts
//
// Pure helpers backing the "오늘 일한 시간" headline in SessionTimePanel.
// No React/zustand/IPC dependency — `nowMs`/`sinceMs` are always passed in
// so these stay deterministically testable. `Date` is only used *inside*
// `startOfLocalDay`/`msUntilNextLocalMidnight` to read local calendar
// fields; callers supply the epoch ms.
//
// See docs/superpowers/specs/2026-07-11-today-worked-total-design.md for the
// `base + (Σ메모리 workedMs − baseline)` model these functions feed.
import type { SessionTurnRecord } from "@shared/types";

/** `records` 중 `endedAt >= sinceMs`(경계 포함)인 것들의 `workedMs` 합. */
export function sumWorkedSince(records: SessionTurnRecord[], sinceMs: number): number {
  return records.filter((r) => r.endedAt >= sinceMs).reduce((acc, r) => acc + r.workedMs, 0);
}

/** `nowMs`가 속한 로컬 날짜의 00:00:00.000 epoch ms. */
export function startOfLocalDay(nowMs: number): number {
  const d = new Date(nowMs);
  d.setHours(0, 0, 0, 0);
  return d.getTime();
}

/**
 * `nowMs`로부터 다음 로컬 자정까지 남은 ms. `setHours(24, ...)`로 계산해
 * DST 전이(23h/25h짜리 날)에도 "다음 날 00:00" 자체는 올바르게 가리킨다.
 */
export function msUntilNextLocalMidnight(nowMs: number): number {
  const d = new Date(nowMs);
  d.setHours(24, 0, 0, 0);
  return d.getTime() - nowMs;
}
