// soundManager 조립 검증 — backend/api를 목으로 대체하고 실제 zustand
// 스토어를 조작해 구독·틱·설정 반영을 확인한다. (appStore가 import하는
// tauriApi는 appStore.test.ts와 같은 방식으로 목 처리.)
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("../../ipc/tauriApi", () => ({
  tauriApi: {
    setAppSettings: vi.fn().mockResolvedValue(undefined),
    appendSessionTurn: vi.fn(),
  },
}));

import { useAppStore } from "../../store/appStore";
import { installSoundManager } from "../soundManager";
import type { SoundBackend } from "../backend";
import type { NotificationEvent, SessionStateEvent } from "@shared/types";

function mockBackend(): SoundBackend & { calls: Record<string, unknown[][]> } {
  const calls: Record<string, unknown[][]> = {};
  const rec =
    (name: string) =>
    (...args: unknown[]) => {
      (calls[name] ??= []).push(args);
    };
  return {
    calls,
    playClicks: rec("playClicks") as SoundBackend["playClicks"],
    playDing: rec("playDing"),
    playSessionStart: rec("playSessionStart"),
    playSessionEnd: rec("playSessionEnd"),
    setVolume: rec("setVolume") as SoundBackend["setVolume"],
    dispose: rec("dispose"),
  };
}

function mockApi() {
  const dataCbs = new Map<string, (d: string) => void>();
  let notifCb: ((n: NotificationEvent) => void) | null = null;
  let sessionCb: ((e: SessionStateEvent) => void) | null = null;
  const dataUnsubs: string[] = [];
  return {
    dataCbs,
    dataUnsubs,
    emitNotification: (n: NotificationEvent) => notifCb?.(n),
    emitSession: (e: SessionStateEvent) => sessionCb?.(e),
    api: {
      onData(agentId: string, cb: (d: string) => void) {
        dataCbs.set(agentId, cb);
        return () => {
          dataCbs.delete(agentId);
          dataUnsubs.push(agentId);
        };
      },
      onNotification(cb: (n: NotificationEvent) => void) {
        notifCb = cb;
        return () => {
          notifCb = null;
        };
      },
      onSessionState(cb: (e: SessionStateEvent) => void) {
        sessionCb = cb;
        return () => {
          sessionCb = null;
        };
      },
    },
  };
}

import type { AgentProfile } from "../../store/types";

const AGENT: AgentProfile = {
  id: "a1",
  name: "테스트",
  role: "dev",
  note: "",
  seed: "seed",
  createdAt: 0,
  deskIndex: 0,
};

function notif(agentId: string): NotificationEvent {
  return { id: "n1", sessionId: "s1", agentId, source: "hook", message: "m", dedupKey: "k", at: 1 };
}

describe("installSoundManager", () => {
  const initial = useAppStore.getState();
  let now = 0;

  beforeEach(() => {
    vi.useFakeTimers();
    now = 0;
    useAppStore.setState(initial, true);
  });
  afterEach(() => {
    vi.useRealTimers();
  });

  function install(backend = mockBackend()) {
    const m = mockApi();
    const off = installSoundManager({
      backend,
      api: m.api,
      now: () => now,
      tickMs: 100,
    });
    return { backend, m, off };
  }

  it("에이전트 추가/제거에 따라 onData 구독을 동기화한다", () => {
    const { m, off } = install();
    useAppStore.getState().addAgent(AGENT);
    expect(m.dataCbs.has("a1")).toBe(true);
    useAppStore.getState().removeAgent("a1");
    expect(m.dataCbs.has("a1")).toBe(false);
    expect(m.dataUnsubs).toContain("a1");
    off();
  });

  it("출력이 흐르면 타이핑 시간 동안 playClicks가 호출된다", () => {
    const { backend, m, off } = install();
    useAppStore.getState().addAgent(AGENT);
    m.dataCbs.get("a1")!("x".repeat(600)); // 타이핑 시간 확보
    // 차분한 타속(최저 초당 3클릭)이라 첫 클릭까지 몇 틱 걸릴 수 있다
    for (let i = 0; i < 10; i++) {
      now += 100;
      vi.advanceTimersByTime(100);
    }
    expect(backend.calls.playClicks?.[0]?.[0]).toBe("a1");
    expect(backend.calls.playClicks?.[0]?.[1]).toBeGreaterThan(0);
    off();
  });

  it("스피너 리페인트 같은 잡음 청크는 소리를 내지 않는다", () => {
    const { backend, m, off } = install();
    useAppStore.getState().addAgent(AGENT);
    // 유효 글자가 적은 TUI 프레임이 반복돼도(대기 중 스피너) 무음이어야 한다.
    const frame = "\x1b[2K\x1b[1G✳ Deliberating… (esc to interrupt · 12s)";
    for (let i = 0; i < 10; i++) {
      m.dataCbs.get("a1")!(frame);
      now += 100;
      vi.advanceTimersByTime(100);
    }
    expect(backend.calls.playClicks).toBeUndefined();
    off();
  });

  it("soundEnabled=false면 클릭을 재생하지 않는다", () => {
    const { backend, m, off } = install();
    useAppStore.getState().addAgent(AGENT);
    useAppStore.getState().updateAppSettings({ soundEnabled: false });
    m.dataCbs.get("a1")!("x".repeat(600));
    now += 100;
    vi.advanceTimersByTime(100);
    expect(backend.calls.playClicks).toBeUndefined();
    off();
  });

  it("볼륨 변경이 backend.setVolume으로 전파된다", () => {
    const { backend, off } = install();
    useAppStore.getState().updateAppSettings({ soundVolume: 0.8 });
    const vols = backend.calls.setVolume!;
    expect(vols[vols.length - 1][0]).toBe(0.8);
    off();
  });

  it("알림 도착 시 딩 — 단, 무음 모드(muted)면 침묵", () => {
    const { backend, m, off } = install();
    m.emitNotification(notif("a1"));
    expect(backend.calls.playDing).toHaveLength(1);
    useAppStore.getState().toggleMuted();
    m.emitNotification(notif("a1"));
    expect(backend.calls.playDing).toHaveLength(1); // 그대로
    off();
  });

  it("세션 running→시작음, exited→종료음, disposed→무음", () => {
    const { backend, m, off } = install();
    const base = { sessionId: "s1", agentId: "a1", at: 1 } as const;
    m.emitSession({ ...base, state: "running" });
    m.emitSession({ ...base, state: "exited" });
    m.emitSession({ ...base, state: "disposed" });
    expect(backend.calls.playSessionStart).toHaveLength(1);
    expect(backend.calls.playSessionEnd).toHaveLength(1);
    off();
  });

  it("teardown이 타이머·구독을 정리하고 backend를 dispose한다", () => {
    const { backend, m, off } = install();
    useAppStore.getState().addAgent(AGENT);
    off();
    expect(m.dataCbs.size).toBe(0);
    expect(backend.calls.dispose).toHaveLength(1);
    m.dataCbs.get("a1"); // 없음
    now += 100;
    vi.advanceTimersByTime(200); // 틱이 죽었으므로 playClicks 없음
    expect(backend.calls.playClicks).toBeUndefined();
  });

  it("backend가 null이면 아무것도 설치하지 않는다", () => {
    const m = mockApi();
    const off = installSoundManager({ backend: null, api: m.api, now: () => 0 });
    useAppStore.getState().addAgent(AGENT);
    expect(m.dataCbs.size).toBe(0);
    off(); // no-op, 예외 없음
  });
});
