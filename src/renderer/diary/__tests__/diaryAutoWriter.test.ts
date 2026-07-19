// src/renderer/diary/__tests__/diaryAutoWriter.test.ts
//
// 자동 일기 트리거(#60): 세션 종료 구독 → 자격 있는 세션 자동 생성, opt-in OFF
// 미호출, 극소 작업/3일 초과 과거 제외, 놓친 스트래글러 flush, 이중 이벤트 dedupe,
// 성공 시 알림·오버레이 갱신. api/log/now/notify/generate 전부 주입.
import { beforeEach, describe, expect, it, vi } from "vitest";

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

beforeEach(() => {
  useAppStore.setState({ agents: { a1: profile() }, taskLabels: {} });
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
