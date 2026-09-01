// src/renderer/ipc/__tests__/sessionBridge.test.ts
//
// TDD for the IPC <-> store bridge and the
// store-backed `OfficeBus` implementation. `../tauriApi` is mocked in full
// (satisfying `AgentOfficeApi`) so the bridge's event routing can be driven
// without any real Tauri runtime.
//
// Coverage:
// - onNotification -> store gains the notification; officeBus relays
//   hasPending=true to onNotificationChanged listeners; badge count synced.
// - onNotificationCleared -> store drops it; hasPending flips back to false;
//   badge count synced back down.
// - onSessionState -> store's session status is set from the wire `state`
//   (SessionState, translated into SessionStatus with no lossy cast); the
//   same `state` is relayed verbatim to officeBus's onSessionStateChanged
//   listeners.
// - Badge sync respects `muted`.
// - emitAgentClicked -> store.openTerminal(agentId) + tauriApi.clearNotifications(agentId).
// - installSessionBridge()'s cleanup detaches the store subscription (no
//   more badge syncing after teardown).

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type {
  NotificationClearedEvent,
  NotificationEvent,
  SessionStateEvent,
  TalkEvent,
} from "@shared/types";

const { mockApi, capture } = vi.hoisted(() => {
  const capture: {
    onSessionState?: (e: SessionStateEvent) => void;
    onNotification?: (e: NotificationEvent) => void;
    onNotificationCleared?: (e: NotificationClearedEvent) => void;
    onActivity?: (e: unknown) => void;
    onTurnUsage?: (e: unknown) => void;
    onTalkMessage?: (e: TalkEvent) => void;
  } = {};
  const mockApi = {
    createSession: vi.fn(),
    disposeSession: vi.fn(),
    writeInput: vi.fn(),
    resize: vi.fn(),
    clearNotifications: vi.fn(),
    listNotifications: vi.fn(),
    loadState: vi.fn(),
    saveState: vi.fn(),
    setBadgeCount: vi.fn(),
    onData: vi.fn(),
    onSessionState: vi.fn((cb: (e: SessionStateEvent) => void) => {
      capture.onSessionState = cb;
      return vi.fn();
    }),
    onNotification: vi.fn((cb: (e: NotificationEvent) => void) => {
      capture.onNotification = cb;
      return vi.fn();
    }),
    onNotificationCleared: vi.fn((cb: (e: NotificationClearedEvent) => void) => {
      capture.onNotificationCleared = cb;
      return vi.fn();
    }),
    onActivity: vi.fn((cb: (e: unknown) => void) => {
      capture.onActivity = cb;
      return vi.fn();
    }),
    onTurnUsage: vi.fn((cb: (e: unknown) => void) => {
      capture.onTurnUsage = cb;
      return vi.fn();
    }),
    onTalkMessage: vi.fn((cb: (e: TalkEvent) => void) => {
      capture.onTalkMessage = cb;
      return vi.fn();
    }),
    appendSessionTurn: vi.fn(),
  };
  return { mockApi, capture };
});

vi.mock("../tauriApi", () => ({ tauriApi: mockApi }));

const { maybeSendOsNotification } = vi.hoisted(() => ({
  maybeSendOsNotification: vi.fn(() => Promise.resolve()),
}));
vi.mock("../osNotify", () => ({ maybeSendOsNotification }));

import { useAppStore } from "../../store/appStore";
import { useAwardsStore } from "../../awards/awardsStore";
import type { AgentProfile } from "../../store/types";
import { installSessionBridge, officeBus } from "../sessionBridge";
import {
  resetNotifySuppression,
  suppressNotifications,
} from "../../agent/summonSuppress";
import type { AwardRecord } from "@shared/types";

const initialState = useAppStore.getState();
const initialAwardsState = useAwardsStore.getState();

function mkAwardRecord(month: string, winnerAgentId: string | null): AwardRecord {
  return {
    month,
    decidedAt: 0,
    rulesVersion: 1,
    winner: winnerAgentId
      ? {
          agentId: winnerAgentId,
          name: `Name-${winnerAgentId}`,
          role: "eng",
          hasPortrait: true,
          stats: { workedMs: 0, turns: 0, toolEvents: 0, activeDays: 0, tokensIn: 0, tokensOut: 0, costUsd: 0 },
        }
      : null,
    leaderboard: [],
    speeches: [],
  };
}

function mkProfile(overrides: Partial<AgentProfile> = {}): AgentProfile {
  return {
    id: "a1",
    name: "Test Agent",
    role: "backend",
    seed: "seed-a1",
    createdAt: Date.now(),
    deskIndex: 0,
    ...overrides,
  };
}

function mkNotifEvent(overrides: Partial<NotificationEvent> = {}): NotificationEvent {
  return {
    id: "n1",
    sessionId: "s1",
    agentId: "a1",
    source: "bell",
    message: "hello",
    dedupKey: "dedup-1",
    at: Date.now(),
    ...overrides,
  };
}

let cleanup: () => void;

beforeEach(() => {
  useAppStore.setState(initialState, true);
  useAwardsStore.setState(initialAwardsState, true);
  Object.values(mockApi).forEach((fn) => fn.mockClear());
  maybeSendOsNotification.mockClear();
  capture.onSessionState = undefined;
  capture.onNotification = undefined;
  capture.onNotificationCleared = undefined;
  capture.onActivity = undefined;
  capture.onTurnUsage = undefined;
  capture.onTalkMessage = undefined;
  resetNotifySuppression();
  cleanup = installSessionBridge();
});

afterEach(() => {
  cleanup();
});

describe("installSessionBridge / onNotification", () => {
  it("adds the notification to the store", () => {
    useAppStore.getState().addAgent(mkProfile({ id: "a1" }));

    capture.onNotification?.(mkNotifEvent({ agentId: "a1" }));

    const notifications = useAppStore.getState().notifications;
    expect(notifications).toHaveLength(1);
    expect(notifications[0].id).toBe("n1");
    expect(notifications[0].agentId).toBe("a1");
  });

  it("sends an OS notification only when the window is unfocused (이슈 #39)", () => {
    useAppStore.getState().addAgent(mkProfile({ id: "a1", name: "코더" }));

    // 포커스 상태(기본): OS 알림 없음.
    capture.onNotification?.(mkNotifEvent({ agentId: "a1", source: "stop", message: "완료" }));
    expect(maybeSendOsNotification).not.toHaveBeenCalled();

    // 비포커스: 제목=에이전트 이름, 본문=메시지로 OS 알림 발송.
    useAppStore.getState().setWindowFocused(false);
    capture.onNotification?.(mkNotifEvent({ agentId: "a1", id: "n2", source: "stop", message: "완료" }));
    expect(maybeSendOsNotification).toHaveBeenCalledWith("코더", "완료");
  });

  it("relays hasPending=true to officeBus.onNotificationChanged listeners", () => {
    useAppStore.getState().addAgent(mkProfile({ id: "a1" }));
    const listener = vi.fn();
    officeBus.onNotificationChanged(listener);

    capture.onNotification?.(mkNotifEvent({ agentId: "a1" }));

    expect(listener).toHaveBeenCalledWith("a1", true);
  });

  it("syncs the dock badge count", () => {
    useAppStore.getState().addAgent(mkProfile({ id: "a1" }));

    capture.onNotification?.(mkNotifEvent({ agentId: "a1" }));

    expect(mockApi.setBadgeCount).toHaveBeenCalledWith(1);
  });

  it("does not touch the badge while muted", () => {
    useAppStore.getState().addAgent(mkProfile({ id: "a1" }));
    useAppStore.setState({ muted: true });
    mockApi.setBadgeCount.mockClear();

    capture.onNotification?.(mkNotifEvent({ agentId: "a1" }));

    expect(mockApi.setBadgeCount).not.toHaveBeenCalled();
  });
});

describe("installSessionBridge / onNotification 소환 억제 창", () => {
  // "전체 자리로" 소환 직후 잔여 Stop이 캐릭터를 곧장 pending으로 만들던 문제
  // (agent/summonSuppress.ts). 억제 창 안에서는 스토어에 넣지 않고 백엔드의
  // 그 알림만 지우되, 시간 집계는 그대로 흘려보낸다.
  it("억제 창 안의 알림은 스토어에 들어가지 않고 백엔드에서 그 id만 지운다", () => {
    useAppStore.getState().addAgent(mkProfile({ id: "a1" }));
    suppressNotifications("a1");

    capture.onNotification?.(mkNotifEvent({ agentId: "a1", id: "n1" }));

    expect(useAppStore.getState().notifications).toHaveLength(0);
    expect(mockApi.clearNotifications).toHaveBeenCalledWith("a1", ["n1"]);
  });

  it("억제 창 안에서는 비포커스여도 OS 알림을 보내지 않는다", () => {
    useAppStore.getState().addAgent(mkProfile({ id: "a1", name: "코더" }));
    useAppStore.getState().setWindowFocused(false);
    suppressNotifications("a1");

    capture.onNotification?.(mkNotifEvent({ agentId: "a1", source: "stop", message: "완료" }));

    expect(maybeSendOsNotification).not.toHaveBeenCalled();
  });

  it("억제는 그 에이전트에만 걸린다", () => {
    const s = useAppStore.getState();
    s.addAgent(mkProfile({ id: "a1" }));
    s.addAgent(mkProfile({ id: "a2" }));
    suppressNotifications("a1");

    capture.onNotification?.(mkNotifEvent({ agentId: "a2", id: "n2" }));

    expect(useAppStore.getState().notifications.map((n) => n.agentId)).toEqual(["a2"]);
  });

  it("억제 창이 만료되면 다시 스토어에 들어간다", () => {
    useAppStore.getState().addAgent(mkProfile({ id: "a1" }));
    suppressNotifications("a1", 3000, Date.now() - 5000); // 이미 지난 창

    capture.onNotification?.(mkNotifEvent({ agentId: "a1", id: "n1" }));

    expect(useAppStore.getState().notifications).toHaveLength(1);
  });
});

describe("installSessionBridge / onNotificationCleared", () => {
  it("removes the notification from the store and flips hasPending back to false", () => {
    useAppStore.getState().addAgent(mkProfile({ id: "a1" }));
    capture.onNotification?.(mkNotifEvent({ agentId: "a1", id: "n1" }));
    const listener = vi.fn();
    officeBus.onNotificationChanged(listener);

    capture.onNotificationCleared?.({ agentId: "a1", ids: ["n1"] });

    expect(useAppStore.getState().notifications).toHaveLength(0);
    expect(listener).toHaveBeenCalledWith("a1", false);
    expect(mockApi.setBadgeCount).toHaveBeenLastCalledWith(0);
  });
});

describe("installSessionBridge / onSessionState", () => {
  it("translates the wire event into the store's session status", () => {
    useAppStore.getState().addAgent(mkProfile({ id: "a1" }));

    capture.onSessionState?.({
      sessionId: "s1",
      agentId: "a1",
      state: "running",
      at: Date.now(),
    });

    expect(useAppStore.getState().sessions.a1.status).toBe("running");
  });

  it("relays the raw wire state to officeBus.onSessionStateChanged listeners", () => {
    useAppStore.getState().addAgent(mkProfile({ id: "a1" }));
    const listener = vi.fn();
    officeBus.onSessionStateChanged(listener);

    capture.onSessionState?.({
      sessionId: "s1",
      agentId: "a1",
      state: "exited",
      at: Date.now(),
    });

    expect(listener).toHaveBeenCalledWith("a1", "exited");
  });

  it("disposed 와이어 이벤트는 스토어에 exited로 정규화되어 저장된다", () => {
    // 의도적 종료(disposeSession) 후 백엔드가 쏘는 disposed를 스토어에 그대로
    // 두면 ensureSession(idle/exited만 재시작)과 TerminalHost 배너(exited만
    // 표시)가 막힌다 — runGuardedCreateSession과 동일한 정규화를 적용한다.
    useAppStore.getState().addAgent(mkProfile({ id: "a1" }));

    capture.onSessionState?.({
      sessionId: "s1",
      agentId: "a1",
      state: "disposed",
      at: Date.now(),
    });

    expect(useAppStore.getState().sessions.a1.status).toBe("exited");
  });

  it("disposed 이벤트도 officeBus 릴레이에는 원본 state로 전달된다", () => {
    useAppStore.getState().addAgent(mkProfile({ id: "a1" }));
    const listener = vi.fn();
    officeBus.onSessionStateChanged(listener);

    capture.onSessionState?.({
      sessionId: "s1",
      agentId: "a1",
      state: "disposed",
      at: Date.now(),
    });

    expect(listener).toHaveBeenCalledWith("a1", "disposed");
  });
});

describe("officeBus.emitAgentClicked", () => {
  it("opens the terminal in the store and clears backend notifications for that agent", () => {
    useAppStore.getState().addAgent(mkProfile({ id: "a1" }));

    officeBus.emitAgentClicked("a1");

    expect(useAppStore.getState().activeTerminalAgentId).toBe("a1");
    expect(mockApi.clearNotifications).toHaveBeenCalledWith("a1");
  });
});

// Clicking a character with no live PTY (idle after hydrate, or exited)
// must recreate its session exactly once, and surface start failures.
describe("officeBus.emitAgentClicked / ensureSession (Fix 1b)", () => {
  const flush = () => new Promise((r) => setTimeout(r, 0));

  it("starts a session for an idle (persisted) agent and marks it starting", async () => {
    useAppStore.getState().hydrate({ agents: [mkProfile({ id: "a1" })], version: 1 });
    expect(useAppStore.getState().sessions.a1.status).toBe("idle");
    mockApi.createSession.mockResolvedValueOnce({ sessionId: "s1", state: "starting" });

    officeBus.emitAgentClicked("a1");

    expect(mockApi.createSession).toHaveBeenCalledTimes(1);
    expect(mockApi.createSession).toHaveBeenCalledWith("a1", {
      agentName: "Test Agent",
      agentRole: "backend",
    });
    expect(useAppStore.getState().sessions.a1.status).toBe("starting");
    await flush();
  });

  it("passes the profile's cwd from the store when starting a session (Task 3)", async () => {
    useAppStore
      .getState()
      .hydrate({ agents: [mkProfile({ id: "a1", cwd: "/tmp/proj" })], version: 1 });
    mockApi.createSession.mockResolvedValueOnce({ sessionId: "s1", state: "starting" });

    officeBus.emitAgentClicked("a1");

    expect(mockApi.createSession).toHaveBeenCalledWith("a1", {
      agentName: "Test Agent",
      agentRole: "backend",
      cwd: "/tmp/proj",
    });
    await flush();
  });

  it("passes the profile's shell from the store when starting a session", async () => {
    useAppStore
      .getState()
      .hydrate({ agents: [mkProfile({ id: "a1", shell: "wsl" })], version: 1 });
    mockApi.createSession.mockResolvedValueOnce({ sessionId: "s1", state: "starting" });

    officeBus.emitAgentClicked("a1");

    expect(mockApi.createSession).toHaveBeenCalledWith("a1", {
      agentName: "Test Agent",
      agentRole: "backend",
      shell: "wsl",
    });
    await flush();
  });

  it("passes both cwd and shell together when both are set on the profile", async () => {
    useAppStore
      .getState()
      .hydrate({ agents: [mkProfile({ id: "a1", cwd: "/tmp/proj", shell: "wsl" })], version: 1 });
    mockApi.createSession.mockResolvedValueOnce({ sessionId: "s1", state: "starting" });

    officeBus.emitAgentClicked("a1");

    expect(mockApi.createSession).toHaveBeenCalledWith("a1", {
      agentName: "Test Agent",
      agentRole: "backend",
      cwd: "/tmp/proj",
      shell: "wsl",
    });
    await flush();
  });

  it("does not start a session for an already-running agent", () => {
    useAppStore.getState().addAgent(mkProfile({ id: "a1" }));
    useAppStore.getState().setSessionState({ agentId: "a1", status: "running" });

    officeBus.emitAgentClicked("a1");

    expect(mockApi.createSession).not.toHaveBeenCalled();
  });

  it("creates only ONE session even on a rapid double click", async () => {
    useAppStore.getState().hydrate({ agents: [mkProfile({ id: "a1" })], version: 1 });
    mockApi.createSession.mockResolvedValue({ sessionId: "s1", state: "starting" });

    officeBus.emitAgentClicked("a1");
    officeBus.emitAgentClicked("a1");

    expect(mockApi.createSession).toHaveBeenCalledTimes(1);
    await flush();
  });

  it("recreates a session for an exited agent", async () => {
    useAppStore.getState().addAgent(mkProfile({ id: "a1" }));
    useAppStore.getState().setSessionState({ agentId: "a1", status: "exited" });
    mockApi.createSession.mockResolvedValueOnce({ sessionId: "s2", state: "starting" });

    officeBus.emitAgentClicked("a1");

    expect(mockApi.createSession).toHaveBeenCalledTimes(1);
    expect(useAppStore.getState().sessions.a1.status).toBe("starting");
    await flush();
  });

  it("flips status back to exited and warns when createSession rejects", async () => {
    useAppStore.getState().hydrate({ agents: [mkProfile({ id: "a1" })], version: 1 });
    const warn = vi.spyOn(console, "warn").mockImplementation(() => {});
    mockApi.createSession.mockRejectedValueOnce(new Error("boom"));

    officeBus.emitAgentClicked("a1");

    await vi.waitFor(() =>
      expect(useAppStore.getState().sessions.a1.status).toBe("exited")
    );
    expect(warn).toHaveBeenCalled();

    // The failed start must be retryable: a subsequent click starts again.
    mockApi.createSession.mockResolvedValueOnce({ sessionId: "s3", state: "starting" });
    officeBus.emitAgentClicked("a1");
    expect(mockApi.createSession).toHaveBeenCalledTimes(2);

    warn.mockRestore();
    await flush();
  });
});

// 2026-07-11 "터미널 영구 고착" 실사고 복구 가드 2건:
// ⓐ 백엔드가 기존 세션을 재사용하면 상태 이벤트를 방출하지 않고 결과만
//    돌려준다 — 결과 상태를 반영하지 않으면 "starting"에 영원히 고착되어
//    이후 클릭이 전부 no-op이 된다.
// ⓑ 백엔드 커맨드가 패닉 등으로 invoke를 영원히 settle하지 않으면
//    startingInFlight가 누수되고 상태도 "starting"에 고착된다 — 타임아웃으로
//    exited 복구해 재시도가 가능해야 한다.
describe("ensureSession 복구 가드 (터미널 영구 고착 방지)", () => {
  it("createSession 결과가 running이면(백엔드 재사용 경로 — 상태 이벤트 없음) 상태를 running으로 반영한다", async () => {
    useAppStore.getState().hydrate({ agents: [mkProfile({ id: "a1" })], version: 1 });
    mockApi.createSession.mockResolvedValueOnce({ sessionId: "s1", state: "running" });

    officeBus.emitAgentClicked("a1");

    await vi.waitFor(() =>
      expect(useAppStore.getState().sessions.a1.status).toBe("running")
    );
  });

  it("createSession 결과가 exited면(생성 직후 즉사) 상태를 exited로 반영해 재시도 가능하게 한다", async () => {
    useAppStore.getState().hydrate({ agents: [mkProfile({ id: "a1" })], version: 1 });
    mockApi.createSession.mockResolvedValueOnce({ sessionId: "s1", state: "exited" });

    officeBus.emitAgentClicked("a1");

    await vi.waitFor(() =>
      expect(useAppStore.getState().sessions.a1.status).toBe("exited")
    );
  });

  it("결과 반영은 상태가 아직 starting일 때만 한다 — 먼저 도착한 백엔드 이벤트를 덮어쓰지 않는다", async () => {
    useAppStore.getState().hydrate({ agents: [mkProfile({ id: "a1" })], version: 1 });
    let resolveCreate: (v: unknown) => void = () => {};
    mockApi.createSession.mockReturnValueOnce(
      new Promise((r) => {
        resolveCreate = r;
      })
    );

    officeBus.emitAgentClicked("a1");
    // invoke가 settle되기 전에 백엔드 이벤트가 exited를 먼저 반영한 상황.
    capture.onSessionState?.({ sessionId: "s1", agentId: "a1", state: "exited", at: Date.now() });
    resolveCreate({ sessionId: "s1", state: "running" }); // 낡은 결과

    await new Promise((r) => setTimeout(r, 0));
    expect(useAppStore.getState().sessions.a1.status).toBe("exited");
  });

  it("invoke가 영원히 settle되지 않으면 타임아웃 후 exited로 복구되고, 이후 클릭이 재시도한다", async () => {
    vi.useFakeTimers();
    try {
      const warn = vi.spyOn(console, "warn").mockImplementation(() => {});
      useAppStore.getState().hydrate({ agents: [mkProfile({ id: "a1" })], version: 1 });
      mockApi.createSession.mockReturnValueOnce(new Promise(() => {})); // 영구 미해결

      officeBus.emitAgentClicked("a1");
      expect(useAppStore.getState().sessions.a1.status).toBe("starting");

      await vi.advanceTimersByTimeAsync(15_001);
      expect(useAppStore.getState().sessions.a1.status).toBe("exited");

      // in-flight 가드도 해제돼 다음 클릭이 실제로 재시도해야 한다.
      mockApi.createSession.mockResolvedValueOnce({ sessionId: "s2", state: "starting" });
      officeBus.emitAgentClicked("a1");
      expect(mockApi.createSession).toHaveBeenCalledTimes(2);
      warn.mockRestore();
    } finally {
      vi.useRealTimers();
    }
  });
});

describe("officeBus.onVacationModeChanged / emitBossDeskClicked (vacation mode)", () => {
  it("구독 즉시 현재값 방출 + 토글 relay, bossDeskClicked → toggle", () => {
    const seen: boolean[] = [];
    const off = officeBus.onVacationModeChanged((on) => seen.push(on));
    expect(seen).toEqual([false]);
    officeBus.emitBossDeskClicked();
    expect(useAppStore.getState().vacationMode).toBe(true);
    expect(seen).toEqual([false, true]);
    off();
  });
});

describe("officeBus.onAwardeeChanged (이 달의 우수사원, docs/employee-of-the-month-design.md §7)", () => {
  it("구독 즉시 현재값(기본 null)을 replay한다", () => {
    const seen: unknown[] = [];
    const off = officeBus.onAwardeeChanged((a) => seen.push(a));
    expect(seen).toEqual([null]);
    off();
  });

  it("useAwardsStore.awards가 바뀌면 최신 확정 수상자를 relay한다", () => {
    const seen: unknown[] = [];
    const off = officeBus.onAwardeeChanged((a) => seen.push(a));
    seen.length = 0; // 초기 replay 제거

    useAwardsStore.setState({ awards: [mkAwardRecord("2026-07", "a1")] });

    expect(seen).toEqual([
      { agentId: "a1", name: "Name-a1", month: "2026-07", hasPortrait: true },
    ]);
    off();
  });

  it("수상자가 바뀌면(다음 달 확정) 최신값으로 다시 relay한다", () => {
    useAwardsStore.setState({ awards: [mkAwardRecord("2026-07", "a1")] });
    const seen: unknown[] = [];
    const off = officeBus.onAwardeeChanged((a) => seen.push(a));
    seen.length = 0;

    useAwardsStore.setState({
      awards: [mkAwardRecord("2026-07", "a1"), mkAwardRecord("2026-08", "a2")],
    });

    expect(seen).toEqual([
      { agentId: "a2", name: "Name-a2", month: "2026-08", hasPortrait: true },
    ]);
    off();
  });

  it("같은 값으로 귀결되는 변화는 재발화하지 않는다(예: 무관한 필드만 바뀜)", () => {
    useAwardsStore.setState({ awards: [mkAwardRecord("2026-07", "a1")] });
    const seen: unknown[] = [];
    const off = officeBus.onAwardeeChanged((a) => seen.push(a));
    seen.length = 0;

    // awards 내용은 그대로, 무관한 필드(loaded)만 바뀜 -> latestAwardee 결과 불변.
    useAwardsStore.setState({ loaded: true });
    // 같은 winner를 담은 새 배열 인스턴스로 교체해도(참조만 다름) 값은 동일.
    useAwardsStore.setState({ awards: [mkAwardRecord("2026-07", "a1")] });

    expect(seen).toEqual([]);
    off();
  });

  it("확정 수상자가 없어지면(winner:null만 있는 배열) null로 relay한다", () => {
    useAwardsStore.setState({ awards: [mkAwardRecord("2026-07", "a1")] });
    const seen: unknown[] = [];
    const off = officeBus.onAwardeeChanged((a) => seen.push(a));
    seen.length = 0;

    useAwardsStore.setState({ awards: [mkAwardRecord("2026-08", null)] });

    expect(seen).toEqual([null]);
    off();
  });
});

describe("installSessionBridge / mute toggle (Task 4G)", () => {
  it("forces the badge to 0 the instant muted flips on, even with pending notifications", () => {
    useAppStore.getState().addAgent(mkProfile({ id: "a1" }));
    capture.onNotification?.(mkNotifEvent({ agentId: "a1" }));
    mockApi.setBadgeCount.mockClear();

    useAppStore.getState().toggleMuted();

    expect(useAppStore.getState().muted).toBe(true);
    expect(mockApi.setBadgeCount).toHaveBeenCalledWith(0);
  });

  it("resyncs the badge to the current pending count the instant muted flips off", () => {
    useAppStore.getState().addAgent(mkProfile({ id: "a1" }));
    useAppStore.getState().addAgent(mkProfile({ id: "a2" }));
    useAppStore.setState({ muted: true });
    // Arrives while muted: store gains it, but the badge stays untouched (existing behavior).
    capture.onNotification?.(mkNotifEvent({ agentId: "a1", id: "n1" }));
    mockApi.setBadgeCount.mockClear();

    useAppStore.getState().toggleMuted();

    expect(useAppStore.getState().muted).toBe(false);
    expect(mockApi.setBadgeCount).toHaveBeenCalledWith(1);
  });
});

describe("officeBus 구독 시점 상태 replay (씬 늦은 마운트/HMR 재마운트)", () => {
  it("onNotificationChanged: 구독 즉시 에이전트별 현재 pending 상태를 replay한다", () => {
    useAppStore.getState().addAgent(mkProfile({ id: "a1" }));
    useAppStore.getState().addAgent(mkProfile({ id: "a2" }));
    capture.onNotification?.(mkNotifEvent({ agentId: "a1" }));

    const seen: Array<[string, boolean]> = [];
    const off = officeBus.onNotificationChanged((id, p) => seen.push([id, p]));

    expect(seen).toContainEqual(["a1", true]);
    expect(seen.map(([id]) => id)).not.toContain("a2");
    off();
  });

  it("onSessionStateChanged: 구독 즉시 현재 세션 상태를 replay한다 (idle은 생략)", () => {
    useAppStore.getState().hydrate({
      agents: [mkProfile({ id: "a1" }), mkProfile({ id: "a2" })],
      version: 1,
    });
    capture.onSessionState?.({ sessionId: "s1", agentId: "a1", state: "running", at: 1 });

    const seen: Array<[string, string]> = [];
    const off = officeBus.onSessionStateChanged((id, st) => seen.push([id, st]));

    expect(seen).toContainEqual(["a1", "running"]);
    expect(seen.map(([id]) => id)).not.toContain("a2");
    off();
  });
});

describe("boot-time vacationMode delivery (모듈 스코프 relay)", () => {
  it("delivers hydrated vacationMode even when the scene subscribed before hydrate, bridge installed or not", () => {
    cleanup(); // 브리지 미설치 상태 재현
    const seen: boolean[] = [];
    const off = officeBus.onVacationModeChanged((on) => seen.push(on));
    seen.length = 0;

    useAppStore.getState().hydrate({ agents: [], version: 1, vacationMode: true });

    expect(seen).toContain(true);
    cleanup = installSessionBridge();
    off();
  });
});

describe("installSessionBridge cleanup", () => {
  it("detaches the store subscription so the badge no longer syncs", () => {
    useAppStore.getState().addAgent(mkProfile({ id: "a1" }));
    cleanup();
    mockApi.setBadgeCount.mockClear();

    useAppStore.getState().pushNotification(mkNotifEvent({ agentId: "a1" }));

    expect(mockApi.setBadgeCount).not.toHaveBeenCalled();
  });

  it("also detaches the mute-toggle subscription", () => {
    useAppStore.getState().addAgent(mkProfile({ id: "a1" }));
    cleanup();
    mockApi.setBadgeCount.mockClear();

    useAppStore.getState().toggleMuted();

    expect(mockApi.setBadgeCount).not.toHaveBeenCalled();
  });
});

describe("talk-message -> officeBus.onTalkBubble 릴레이", () => {
  const mkTalkEvent = (overrides: Partial<TalkEvent> = {}): TalkEvent => ({
    convId: "c1",
    from: "a1",
    fromName: "하나",
    to: "a2",
    toName: "두리",
    text: "이거 어떻게 생각해?",
    at: Date.now(),
    ...overrides,
  });

  it("발신자에겐 본문(say), 수신자에겐 짧은 도착 표시(hear)를 한 번씩 준다", () => {
    const seen: Array<[string, string, string]> = [];
    const off = officeBus.onTalkBubble((id, text, tone) => seen.push([id, text, tone]));

    capture.onTalkMessage!(mkTalkEvent());

    expect(seen).toEqual([
      ["a1", "이거 어떻게 생각해?", "say"],
      ["a2", "하나 →", "hear"],
    ]);
    off();
  });

  it("자기 자신에게 보낸 메시지는 도착 표시를 겹쳐 띄우지 않는다", () => {
    const seen: string[] = [];
    const off = officeBus.onTalkBubble((_id, _text, tone) => seen.push(tone));

    capture.onTalkMessage!(mkTalkEvent({ to: "a1", toName: "하나" }));

    expect(seen).toEqual(["say"]);
    off();
  });

  it("구독 해제 후에는 더 이상 받지 않는다(replay도 없다)", () => {
    const seen: string[] = [];
    const off = officeBus.onTalkBubble((id) => seen.push(id));
    off();

    capture.onTalkMessage!(mkTalkEvent());

    expect(seen).toEqual([]);
  });
});
