// src/renderer/power/__tests__/keepAwake.test.ts
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { AgentTurnState } from "../../timeline/turnReducer";
import {
  computeAnyWorking,
  createKeepAwakeController,
  KEEP_AWAKE_RELEASE_DELAY_MS,
} from "../keepAwake";

function turn(phase: AgentTurnState["phase"]): AgentTurnState {
  return {
    phase,
    turnStartedAt: null,
    waitingSince: null,
    waitedInTurnMs: 0,
    totalMs: 0,
    workedMs: 0,
    waitedMs: 0,
    turns: 0,
  };
}

describe("computeAnyWorking", () => {
  it("빈 맵은 false", () => {
    expect(computeAnyWorking({})).toBe(false);
  });
  it("idle/waiting만 있으면 false", () => {
    expect(computeAnyWorking({ a: turn("idle"), b: turn("waiting") })).toBe(false);
  });
  it("working이 하나라도 있으면 true", () => {
    expect(computeAnyWorking({ a: turn("idle"), b: turn("working") })).toBe(true);
  });
});

describe("createKeepAwakeController", () => {
  beforeEach(() => vi.useFakeTimers());
  afterEach(() => vi.useRealTimers());

  it("일하는 캐릭터가 생기면 즉시 acquire", () => {
    const notify = vi.fn();
    const c = createKeepAwakeController(notify);
    c.update(true, true);
    expect(notify).toHaveBeenCalledTimes(1);
    expect(notify).toHaveBeenLastCalledWith(true);
  });

  it("idle 진입 후 지연 시간 안에 다시 working이면 release하지 않는다", () => {
    const notify = vi.fn();
    const c = createKeepAwakeController(notify);
    c.update(true, true); // acquire
    notify.mockClear();
    c.update(true, false); // idle → 지연 release 예약
    vi.advanceTimersByTime(KEEP_AWAKE_RELEASE_DELAY_MS - 1000);
    c.update(true, true); // 지연 만료 전 복귀 → 타이머 취소, 이미 held라 재통지 없음
    vi.advanceTimersByTime(5000);
    expect(notify).not.toHaveBeenCalledWith(false);
  });

  it("idle이 지연 시간을 넘기면 release 통지", () => {
    const notify = vi.fn();
    const c = createKeepAwakeController(notify);
    c.update(true, true); // acquire
    notify.mockClear();
    c.update(true, false);
    vi.advanceTimersByTime(KEEP_AWAKE_RELEASE_DELAY_MS + 1);
    expect(notify).toHaveBeenCalledTimes(1);
    expect(notify).toHaveBeenLastCalledWith(false);
  });

  it("설정 OFF면 지연 없이 즉시 release", () => {
    const notify = vi.fn();
    const c = createKeepAwakeController(notify);
    c.update(true, true); // acquire
    notify.mockClear();
    c.update(false, true); // 설정 off → 즉시 해제(anyWorking과 무관)
    expect(notify).toHaveBeenCalledTimes(1);
    expect(notify).toHaveBeenLastCalledWith(false);
  });

  it("held 동안 lease 갱신을 주기적으로 재통지", () => {
    const notify = vi.fn();
    const c = createKeepAwakeController(notify, { renewIntervalMs: 1000 });
    c.update(true, true);
    expect(notify).toHaveBeenCalledTimes(1);
    vi.advanceTimersByTime(3000);
    expect(notify).toHaveBeenCalledTimes(4); // 최초 1 + 갱신 3
    expect(notify).toHaveBeenLastCalledWith(true);
  });

  it("dispose는 타이머만 정리하고 통지하지 않는다", () => {
    const notify = vi.fn();
    const c = createKeepAwakeController(notify, { renewIntervalMs: 1000 });
    c.update(true, true);
    notify.mockClear();
    c.dispose();
    vi.advanceTimersByTime(5000);
    expect(notify).not.toHaveBeenCalled();
  });
});
