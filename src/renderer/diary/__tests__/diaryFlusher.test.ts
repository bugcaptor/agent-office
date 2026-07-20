// src/renderer/diary/__tests__/diaryFlusher.test.ts
//
// DiaryFlusher(#60)의 신규 동작만 집중 검증한다: (1) 진행 중(running) 세션 제외
// (includeLive), (2) 부팅 복원된 세션의 중복 일기 방지(이미 있으면 스킵+로그 소진).
// 정책 공통부(3일 컷오프·MIN_ITEMS·attempted·in-flight)는 diaryAutoWriter.test가
// 이미 커버한다.
import { beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("../../ipc/tauriApi", () => ({
  tauriApi: {
    loadDiary: vi.fn().mockResolvedValue([]),
    setAppSettings: vi.fn().mockResolvedValue(undefined),
  },
}));

import { useAppStore } from "../../store/appStore";
import { AUTO_DIARY_MAX_AGE_MS, DiaryFlusher, TIMEOUT_MAX_RETRIES } from "../diaryFlusher";
import { restoredSessionKeys } from "../workLogPersister";
import { WorkLog } from "../workLog";
import type { AppSettings, DiaryEntry } from "@shared/types";
import type { DiaryResult } from "../diaryGenerator";
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
  cliEnabled: false,
};

const NOW = 1_000_000_000_000;

function logWith(pairs: Array<[string, number]>): WorkLog {
  const log = new WorkLog();
  for (const [sessionId, count] of pairs) {
    for (let i = 0; i < count; i++) {
      log.append("a1", { at: NOW, sessionId, kind: "tool", text: `${sessionId}-${i}` });
    }
  }
  return log;
}

function ok(sessionId: string): { ok: true; entry: DiaryEntry } {
  return { ok: true, entry: { at: NOW, sessionId, body: "기록." } };
}

const TIMEOUT: DiaryResult = { ok: false, reason: "timeout" };

function runningSession(): StoreSessionRuntime {
  return { agentId: "a1", status: "running", cols: 80, rows: 24, lastActivityAt: NOW };
}

beforeEach(() => {
  restoredSessionKeys.clear();
  useAppStore.setState({ agents: {}, taskLabels: {}, sessions: {} });
  useAppStore.getState().hydrateSettings(SETTINGS_ON, false);
});

describe("DiaryFlusher — 진행 중 세션 제외", () => {
  it("includeLive=false면 진행 중(running) 세션은 건너뛰고 종료된 세션만 쓴다", async () => {
    useAppStore.setState({
      taskLabels: { a1: { sessionId: "live" } },
      sessions: { a1: runningSession() },
    });
    const generate = vi.fn((_id, _d, sid?: string) => Promise.resolve(ok(sid ?? "")));
    const flusher = new DiaryFlusher({
      now: () => NOW,
      log: logWith([["ended", 4], ["live", 4]]),
      generate,
    });

    await flusher.flushAgent("a1", { includeLive: false, source: "open-diary" });

    const sessions = generate.mock.calls.map((c) => c[2]);
    expect(sessions).toEqual(["ended"]);
  });

  it("진행 중 세션은 attempted로 굳지 않아, 나중에 종료되면 그때 쓴다", async () => {
    useAppStore.setState({
      taskLabels: { a1: { sessionId: "live" } },
      sessions: { a1: runningSession() },
    });
    const generate = vi.fn((_id, _d, sid?: string) => Promise.resolve(ok(sid ?? "")));
    const log = logWith([["live", 4]]);
    const flusher = new DiaryFlusher({ now: () => NOW, log, generate });

    // 진행 중엔 스킵.
    await flusher.flushAgent("a1", { includeLive: false, source: "open-diary" });
    expect(generate).not.toHaveBeenCalled();

    // 세션이 종료됨(더 이상 running 아님) → 이제 쓴다.
    useAppStore.setState({ sessions: {} });
    await flusher.flushAgent("a1", { includeLive: false, source: "session-end" });
    expect(generate.mock.calls.map((c) => c[2])).toEqual(["live"]);
  });

  it("includeLive=true면 진행 중 세션도 포함한다", async () => {
    useAppStore.setState({
      taskLabels: { a1: { sessionId: "live" } },
      sessions: { a1: runningSession() },
    });
    const generate = vi.fn((_id, _d, sid?: string) => Promise.resolve(ok(sid ?? "")));
    const flusher = new DiaryFlusher({
      now: () => NOW,
      log: logWith([["live", 4]]),
      generate,
    });

    await flusher.flushAgent("a1", { includeLive: true, source: "quit" });
    expect(generate.mock.calls.map((c) => c[2])).toEqual(["live"]);
  });
});

describe("DiaryFlusher — 복원 세션 중복 방지", () => {
  it("복원된 세션에 이미 일기가 있으면 재생성하지 않고 로그만 소진한다", async () => {
    restoredSessionKeys.add("a1:restored");
    const log = logWith([["restored", 4]]);
    const generate = vi.fn((_id, _d, sid?: string) => Promise.resolve(ok(sid ?? "")));
    // 이미 그 세션 일기가 디스크에 있음(at >= 마지막 항목 at).
    const loadDiary = vi.fn().mockResolvedValue([{ at: NOW, sessionId: "restored", body: "이미 씀" }]);
    const flusher = new DiaryFlusher({ now: () => NOW, log, generate, loadDiary });

    await flusher.flushAgent("a1", { includeLive: false, source: "session-end" });

    expect(generate).not.toHaveBeenCalled();
    expect(log.items("a1")).toEqual([]); // 소진됨
    expect(restoredSessionKeys.has("a1:restored")).toBe(false);
  });

  it("복원된 세션이지만 일기가 아직 없으면 정상 생성한다", async () => {
    restoredSessionKeys.add("a1:restored");
    const log = logWith([["restored", 4]]);
    const generate = vi.fn((_id, _d, sid?: string) => Promise.resolve(ok(sid ?? "")));
    const loadDiary = vi.fn().mockResolvedValue([]); // 기존 일기 없음
    const flusher = new DiaryFlusher({ now: () => NOW, log, generate, loadDiary });

    await flusher.flushAgent("a1", { includeLive: false, source: "session-end" });

    expect(generate.mock.calls.map((c) => c[2])).toEqual(["restored"]);
  });

  it("런타임(비복원) 세션은 중복 검사 없이 바로 생성한다(loadDiary 미호출)", async () => {
    const log = logWith([["runtime", 4]]);
    const generate = vi.fn((_id, _d, sid?: string) => Promise.resolve(ok(sid ?? "")));
    const loadDiary = vi.fn().mockResolvedValue([]);
    const flusher = new DiaryFlusher({ now: () => NOW, log, generate, loadDiary });

    await flusher.flushAgent("a1", { includeLive: false, source: "session-end" });

    expect(generate).toHaveBeenCalledTimes(1);
    expect(loadDiary).not.toHaveBeenCalled();
  });
});

describe("DiaryFlusher — 타임아웃 재시도 보존(#66)", () => {
  it("타임아웃은 attempted로 굳지 않아 다음 flush에서 재시도된다", async () => {
    const log = logWith([["s1", 4]]);
    const generate = vi.fn(
      (_id: string, _d: unknown, sid?: string): Promise<DiaryResult> => Promise.resolve(ok(sid ?? "")),
    );
    generate.mockResolvedValueOnce(TIMEOUT); // 첫 시도는 타임아웃, 이후 기본 성공
    const flusher = new DiaryFlusher({ now: () => NOW, log, generate });

    await flusher.flushAgent("a1", { includeLive: false, source: "background" });
    await flusher.flushAgent("a1", { includeLive: false, source: "background" });

    // 첫 타임아웃은 attempted로 굳지 않아, 둘째 flush에서 같은 세션을 재시도한다.
    expect(generate).toHaveBeenCalledTimes(2);
    expect(generate.mock.calls.map((c) => c[2])).toEqual(["s1", "s1"]);

    // 성공 후엔 attempted로 확정 — 셋째 flush는 더는 시도하지 않는다.
    await flusher.flushAgent("a1", { includeLive: false, source: "background" });
    expect(generate).toHaveBeenCalledTimes(2);
  });

  it("연속 타임아웃이 상한을 넘으면 attempted로 확정해 더는 시도하지 않는다", async () => {
    const log = logWith([["s1", 4]]);
    const generate = vi.fn(() => Promise.resolve(TIMEOUT)); // 매번 타임아웃
    const flusher = new DiaryFlusher({ now: () => NOW, log, generate });

    // 상한까지는 매 트리거마다 재시도, 상한 초과 트리거에서 확정 → 그 뒤엔 스킵.
    for (let i = 0; i < TIMEOUT_MAX_RETRIES + 3; i++) {
      await flusher.flushAgent("a1", { includeLive: false, source: "background" });
    }

    // TIMEOUT_MAX_RETRIES회 재시도 + 상한 초과 1회 = 상한+1번 호출 후 확정.
    expect(generate).toHaveBeenCalledTimes(TIMEOUT_MAX_RETRIES + 1);
  });

  it("타임아웃 재시도 중 3일 컷오프를 넘기면 확정되어 재시도가 소급 금지를 우회하지 않는다", async () => {
    // 마지막 활동이 3일보다 오래된 로그.
    const oldAt = NOW - AUTO_DIARY_MAX_AGE_MS - 1;
    const log = new WorkLog();
    for (let i = 0; i < 4; i++) {
      log.append("a1", { at: oldAt, sessionId: "old", kind: "tool", text: `old-${i}` });
    }
    const generate = vi.fn(() => Promise.resolve(TIMEOUT));
    const flusher = new DiaryFlusher({ now: () => NOW, log, generate });

    await flusher.flushAgent("a1", { includeLive: false, source: "background" });
    await flusher.flushAgent("a1", { includeLive: false, source: "background" });

    // 컷오프에 걸려 생성 자체를 시도하지 않고 확정.
    expect(generate).not.toHaveBeenCalled();
  });
});
