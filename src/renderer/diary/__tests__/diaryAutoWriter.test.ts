// src/renderer/diary/__tests__/diaryAutoWriter.test.ts
//
// 자동 일기 트리거(#60): 세션 종료 구독 → 자격 있는 세션 자동 생성, opt-in OFF
// 미호출, 극소 작업/3일 초과 과거 제외, 놓친 스트래글러 flush, 이중 이벤트 dedupe,
// 성공 시 알림·오버레이 갱신. api/log/now/notify/generate 전부 주입.
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("../../ipc/tauriApi", () => ({
  tauriApi: {
    loadDiary: vi.fn().mockResolvedValue([]),
    setAppSettings: vi.fn().mockResolvedValue(undefined),
  },
}));

import { useAppStore } from "../../store/appStore";
import { useDiaryStore } from "../diaryStore";
import {
  AUTO_DIARY_MAX_AGE_MS,
  AUTO_DIARY_MIN_ITEMS,
  installDiaryAutoWriter,
} from "../diaryAutoWriter";
import { WorkLog } from "../workLog";
import type { AgentProfile, AppSettings, DiaryEntry, SessionStateEvent } from "@shared/types";
import type { SessionRuntime as StoreSessionRuntime } from "../../store/types";

const SETTINGS_ON: AppSettings = {
  version: 1,
  summarizerEnabled: false,
  summaryProvider: "claude",
  diaryEnabled: true,
  observerEnabled: false,
  soundEnabled: true,
  soundVolume: 0.5,
  externalTerminal: "terminal",
  externalEditor: "system",
  attentionHoldMs: 5000,
  gitStatusEnabled: true,
  fileIndexBackend: "walker",
  cliEnabled: false,
  keepAwakeEnabled: false,
  mascotEnabled: false,
};

function profile(overrides: Partial<AgentProfile> = {}): AgentProfile {
  return { id: "a1", name: "컴파일러", role: "빌드", note: "", seed: "x", createdAt: 0, deskIndex: 0, ...overrides };
}

/** 한 세션에 n개 항목을 특정 시각에 채운 버퍼. */
function logWith(sessionId: string, count: number, at: number): WorkLog {
  const log = new WorkLog();
  for (let i = 0; i < count; i++) {
    log.append("a1", { at, sessionId, kind: "tool", text: `t${i}` });
  }
  return log;
}

/** onSessionState 구독을 잡아 이벤트를 손으로 방출하는 목 api. */
function mockApi() {
  let cb: ((e: SessionStateEvent) => void) | null = null;
  return {
    emit: (e: Partial<SessionStateEvent>) =>
      cb?.({ sessionId: "s1", agentId: "a1", state: "exited", at: 0, ...e } as SessionStateEvent),
    api: {
      onSessionState(fn: (e: SessionStateEvent) => void) {
        cb = fn;
        return () => {
          cb = null;
        };
      },
    },
  };
}

const NOW = 1_000_000_000_000;
function okResult(sessionId: string): { ok: true; entry: DiaryEntry } {
  return { ok: true, entry: { at: NOW, sessionId, body: "오늘은 빌드를 고쳤다." } };
}

function running(agentId: string): StoreSessionRuntime {
  return { agentId, status: "running", cols: 80, rows: 24, lastActivityAt: NOW };
}

beforeEach(() => {
  useAppStore.setState({ agents: { a1: profile() }, taskLabels: {}, agentOrder: ["a1"], sessions: {} });
  useAppStore.getState().hydrateSettings(SETTINGS_ON, false);
  useDiaryStore.setState({ overlay: null, entries: [] });
});

describe("installDiaryAutoWriter", () => {
  it("세션 종료 시 자격 있는 세션의 일기를 그 sessionId로 자동 생성한다", async () => {
    const m = mockApi();
    const generate = vi.fn().mockResolvedValue(okResult("s1"));
    const notify = vi.fn();
    const off = installDiaryAutoWriter({
      api: m.api,
      now: () => NOW,
      log: logWith("s1", AUTO_DIARY_MIN_ITEMS, NOW),
      generate,
      notify,
    });

    m.emit({ state: "exited" });
    await Promise.resolve();
    await Promise.resolve();

    expect(generate).toHaveBeenCalledTimes(1);
    expect(generate).toHaveBeenCalledWith("a1", {}, "s1");
    expect(notify).toHaveBeenCalledTimes(1);
    expect(notify.mock.calls[0][0]).toContain("컴파일러");
    off();
  });

  it("diaryEnabled=false면 생성기를 호출하지 않는다", async () => {
    useAppStore.getState().hydrateSettings({ ...SETTINGS_ON, diaryEnabled: false }, false);
    const m = mockApi();
    const generate = vi.fn();
    const off = installDiaryAutoWriter({ api: m.api, now: () => NOW, log: logWith("s1", 5, NOW), generate });

    m.emit({ state: "exited" });
    await Promise.resolve();

    expect(generate).not.toHaveBeenCalled();
    off();
  });

  it("작업 로그가 없는 종료는 조용히 건너뛴다", async () => {
    const m = mockApi();
    const generate = vi.fn();
    const off = installDiaryAutoWriter({ api: m.api, now: () => NOW, log: new WorkLog(), generate });

    m.emit({ state: "exited" });
    await Promise.resolve();

    expect(generate).not.toHaveBeenCalled();
    off();
  });

  it("작업량이 극히 적은 세션은 제외한다", async () => {
    const m = mockApi();
    const generate = vi.fn().mockResolvedValue(okResult("s1"));
    const off = installDiaryAutoWriter({
      api: m.api,
      now: () => NOW,
      log: logWith("s1", AUTO_DIARY_MIN_ITEMS - 1, NOW),
      generate,
    });

    m.emit({ state: "exited" });
    await Promise.resolve();

    expect(generate).not.toHaveBeenCalled();
    off();
  });

  it("마지막 활동이 3일보다 오래된 과거 세션은 자동 생성하지 않는다", async () => {
    const m = mockApi();
    const generate = vi.fn().mockResolvedValue(okResult("old"));
    const stale = NOW - AUTO_DIARY_MAX_AGE_MS - 1;
    const off = installDiaryAutoWriter({
      api: m.api,
      now: () => NOW,
      log: logWith("old", 5, stale),
      generate,
    });

    m.emit({ state: "exited", sessionId: "old" });
    await Promise.resolve();

    expect(generate).not.toHaveBeenCalled();
    off();
  });

  it("놓친 스트래글러 세션도 함께 기록한다(3일 이내·충분한 작업)", async () => {
    const log = new WorkLog();
    for (let i = 0; i < 4; i++) log.append("a1", { at: NOW, sessionId: "s1", kind: "tool", text: `a${i}` });
    for (let i = 0; i < 4; i++) log.append("a1", { at: NOW, sessionId: "s2", kind: "tool", text: `b${i}` });
    const m = mockApi();
    const generate = vi.fn((_id: string, _d: unknown, sid?: string) => Promise.resolve(okResult(sid ?? "")));
    const off = installDiaryAutoWriter({ api: m.api, now: () => NOW, log, generate });

    m.emit({ state: "exited", sessionId: "s2" });
    await Promise.resolve();
    await Promise.resolve();
    await Promise.resolve();

    const sessions = generate.mock.calls.map((c) => c[2]).sort();
    expect(sessions).toEqual(["s1", "s2"]);
    off();
  });

  it("이중 이벤트(exited→disposed)에 같은 세션을 두 번 생성하지 않는다", async () => {
    const m = mockApi();
    const generate = vi.fn().mockResolvedValue(okResult("s1"));
    const off = installDiaryAutoWriter({
      api: m.api,
      now: () => NOW,
      log: logWith("s1", 5, NOW),
      generate,
    });

    m.emit({ state: "exited" });
    await Promise.resolve();
    await Promise.resolve();
    m.emit({ state: "disposed" });
    await Promise.resolve();

    expect(generate).toHaveBeenCalledTimes(1);
    off();
  });

  it("겹친 종료 이벤트(exited/disposed)에서 자격 있는 세션을 잃지 않는다", async () => {
    // 두 세션이 버퍼에 남은 채 종료 콜백이 겹칠 때, generateDiary의 per-agent
    // in-flight와 얽혀 한 세션이 유실되던 레이스(리뷰 P2)의 회귀 방지.
    const log = new WorkLog();
    for (let i = 0; i < 4; i++) log.append("a1", { at: NOW, sessionId: "s1", kind: "tool", text: `a${i}` });
    for (let i = 0; i < 4; i++) log.append("a1", { at: NOW, sessionId: "s2", kind: "tool", text: `b${i}` });

    // 실제 generateDiary처럼 per-agent in-flight를 흉내낸다: 동시 호출은 거절.
    let active = false;
    const written: string[] = [];
    const generate = vi.fn(async (_id: string, _d: unknown, sid?: string) => {
      if (active) return { ok: false as const, reason: "in-flight" as const };
      active = true;
      await Promise.resolve();
      active = false;
      written.push(sid ?? "");
      return okResult(sid ?? "");
    });

    const m = mockApi();
    const off = installDiaryAutoWriter({ api: m.api, now: () => NOW, log, generate, notify: vi.fn() });

    m.emit({ state: "exited", sessionId: "s1" });
    m.emit({ state: "disposed", sessionId: "s1" });
    await new Promise((r) => setTimeout(r, 0));
    await new Promise((r) => setTimeout(r, 0));

    // 두 세션 모두 성공적으로 기록됐고, 어느 것도 한 번보다 많이 쓰지 않았다.
    expect([...written].sort()).toEqual(["s1", "s2"]);
    off();
  });

  it("오버레이가 그 캐릭터를 열고 있으면 성공 후 갱신한다", async () => {
    useDiaryStore.setState({ overlay: { agentId: "a1", agentName: "컴파일러" }, entries: [] });
    const refresh = vi.spyOn(useDiaryStore.getState(), "refresh").mockResolvedValue();
    const m = mockApi();
    const generate = vi.fn().mockResolvedValue(okResult("s1"));
    const off = installDiaryAutoWriter({
      api: m.api,
      now: () => NOW,
      log: logWith("s1", 5, NOW),
      generate,
      notify: vi.fn(),
    });

    m.emit({ state: "exited" });
    await Promise.resolve();
    await Promise.resolve();

    expect(refresh).toHaveBeenCalledWith("a1");
    off();
  });

  it("running·starting 이벤트는 무시한다", async () => {
    const m = mockApi();
    const generate = vi.fn();
    const off = installDiaryAutoWriter({ api: m.api, now: () => NOW, log: logWith("s1", 5, NOW), generate });

    m.emit({ state: "running" });
    m.emit({ state: "starting" });
    await Promise.resolve();

    expect(generate).not.toHaveBeenCalled();
    off();
  });
});

describe("installDiaryAutoWriter — 백그라운드 유휴 스윕(#66)", () => {
  beforeEach(() => vi.useFakeTimers());
  afterEach(() => vi.useRealTimers());

  it("유휴일 때 백스톱 주기에 밀린(이벤트 없이 쌓인) 세션을 생성한다", async () => {
    const m = mockApi();
    const generate = vi.fn().mockResolvedValue(okResult("s1"));
    const off = installDiaryAutoWriter({
      api: m.api,
      now: () => NOW,
      log: logWith("s1", AUTO_DIARY_MIN_ITEMS, NOW),
      generate,
      notify: vi.fn(),
      backstopMs: 1000,
      settleMs: 500,
    });

    // 종료 이벤트가 없어도 백스톱이 백로그를 비운다. 첫 주기 전엔 아무 것도 안 함.
    expect(generate).not.toHaveBeenCalled();
    await vi.advanceTimersByTimeAsync(1000);
    expect(generate).toHaveBeenCalledWith("a1", {}, "s1");
    off();
  });

  it("활성(running) 세션이 있으면 백스톱이 돌아도 스윕하지 않는다", async () => {
    useAppStore.setState({ sessions: { other: running("other") } });
    const m = mockApi();
    const generate = vi.fn().mockResolvedValue(okResult("s1"));
    const off = installDiaryAutoWriter({
      api: m.api,
      now: () => NOW,
      log: logWith("s1", AUTO_DIARY_MIN_ITEMS, NOW),
      generate,
      notify: vi.fn(),
      backstopMs: 1000,
    });

    await vi.advanceTimersByTimeAsync(3000);
    expect(generate).not.toHaveBeenCalled();
    off();
  });

  it("종료 즉시 생성이 타임아웃이면 정착 지연 뒤 재시도한다", async () => {
    const m = mockApi();
    const generate = vi
      .fn()
      .mockResolvedValueOnce({ ok: false, reason: "timeout" })
      .mockResolvedValue(okResult("s1"));
    const off = installDiaryAutoWriter({
      api: m.api,
      now: () => NOW,
      log: logWith("s1", AUTO_DIARY_MIN_ITEMS, NOW),
      generate,
      notify: vi.fn(),
      settleMs: 500,
      backstopMs: 100000,
    });

    m.emit({ state: "exited", sessionId: "s1" });
    await vi.advanceTimersByTimeAsync(0); // 즉시 flush(타임아웃) 소화
    expect(generate).toHaveBeenCalledTimes(1);

    await vi.advanceTimersByTimeAsync(500); // 유휴 정착 → 스윕 재시도
    expect(generate).toHaveBeenCalledTimes(2);
    off();
  });

  it("종료 flush가 타임아웃이어도 앱이 계속 활성이면 유휴-비의존 재시도로 결국 쓴다(#75)", async () => {
    // 다른 에이전트가 running이라 앱은 유휴가 아니다 → 정착·백스톱 스윕은 안 돈다.
    useAppStore.setState({ sessions: { other: running("other") } });
    const m = mockApi();
    const generate = vi
      .fn()
      .mockResolvedValueOnce({ ok: false, reason: "timeout" })
      .mockResolvedValue(okResult("s1"));
    const off = installDiaryAutoWriter({
      api: m.api,
      now: () => NOW,
      log: logWith("s1", AUTO_DIARY_MIN_ITEMS, NOW),
      generate,
      notify: vi.fn(),
      settleMs: 100000, // 유휴 스윕은 사실상 비활성
      backstopMs: 100000,
      endRetryMs: 500,
    });

    m.emit({ state: "exited", sessionId: "s1" });
    await vi.advanceTimersByTimeAsync(0); // 즉시 flush(타임아웃) 소화
    expect(generate).toHaveBeenCalledTimes(1);

    await vi.advanceTimersByTimeAsync(500); // 유휴와 무관한 재시도 → 성공
    expect(generate).toHaveBeenCalledTimes(2);
    off();
  });

  it("유휴-비의존 재시도는 상한(maxEndRetries)에서 멈춘다(#75)", async () => {
    useAppStore.setState({ sessions: { other: running("other") } });
    const m = mockApi();
    // 계속 in-flight → hasPendingWork가 계속 참이라 상한이 없으면 무한 재시도.
    const generate = vi.fn().mockResolvedValue({ ok: false, reason: "in-flight" });
    const off = installDiaryAutoWriter({
      api: m.api,
      now: () => NOW,
      log: logWith("s1", AUTO_DIARY_MIN_ITEMS, NOW),
      generate,
      notify: vi.fn(),
      settleMs: 100000,
      backstopMs: 100000,
      endRetryMs: 100,
      maxEndRetries: 3,
    });

    m.emit({ state: "exited", sessionId: "s1" });
    await vi.advanceTimersByTimeAsync(0); // 즉시 시도(1회)
    await vi.advanceTimersByTimeAsync(10_000); // 재시도가 상한까지만
    // 즉시 1 + 재시도 3 = 4회에서 멈춘다.
    expect(generate).toHaveBeenCalledTimes(1 + 3);
    off();
  });

  it("정착 대기 중 새 세션이 시작되면 스윕이 재시도하지 않는다(활성 가드)", async () => {
    const m = mockApi();
    // 계속 타임아웃 → 세션이 재시도 대상으로 남는다(즉시 flush가 소진하지 않음).
    const generate = vi.fn().mockResolvedValue({ ok: false, reason: "timeout" });
    const off = installDiaryAutoWriter({
      api: m.api,
      now: () => NOW,
      log: logWith("s1", AUTO_DIARY_MIN_ITEMS, NOW),
      generate,
      notify: vi.fn(),
      settleMs: 500,
      backstopMs: 100000,
    });

    m.emit({ state: "exited", sessionId: "s1" });
    await vi.advanceTimersByTimeAsync(0); // 즉시 flush 1회(타임아웃, 재시도 대상 유지)
    expect(generate).toHaveBeenCalledTimes(1);

    // 정착 만료 전 새 세션 시작 → 활성 상태로 전이.
    useAppStore.setState({ sessions: { a1: running("a1") } });
    m.emit({ state: "running", sessionId: "s9" });
    await vi.advanceTimersByTimeAsync(1000);

    // 활성이므로 정착 스윕이 재시도하지 않는다 → 여전히 1회.
    expect(generate).toHaveBeenCalledTimes(1);
    off();
  });

  it("해제하면 백스톱 타이머가 더는 돌지 않는다", async () => {
    const m = mockApi();
    const generate = vi.fn().mockResolvedValue(okResult("s1"));
    const off = installDiaryAutoWriter({
      api: m.api,
      now: () => NOW,
      log: logWith("s1", AUTO_DIARY_MIN_ITEMS, NOW),
      generate,
      notify: vi.fn(),
      backstopMs: 1000,
    });

    off();
    await vi.advanceTimersByTimeAsync(5000);
    expect(generate).not.toHaveBeenCalled();
  });
});
