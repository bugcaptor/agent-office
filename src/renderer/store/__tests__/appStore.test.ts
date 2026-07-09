// src/renderer/store/__tests__/appStore.test.ts
//
// Tests for the zustand app store core.
//
// Coverage:
// - T1: addAgent seeds a `starting` session (80x24) for the new agent.
// - T2: openTerminal clears only that agent's notifications and updates
//   active/recent.
// - T3: pushNotification is suppressed for the currently-active agent.
// Plus a couple of directly-adjacent cases (removeAgent cleanup, excerpt
// truncation, hydrate) that exercise the same action surface without
// needing a separate file.

import { beforeEach, describe, expect, it, vi } from "vitest";
import type { AgentProfile, NotificationEvent } from "../types";

const { setAppSettingsMock } = vi.hoisted(() => ({
  setAppSettingsMock: vi.fn().mockResolvedValue(undefined),
}));
vi.mock("../../ipc/tauriApi", () => ({ tauriApi: { setAppSettings: setAppSettingsMock } }));

import { useAppStore } from "../appStore";

let notifSeq = 0;

function mkProfile(overrides: Partial<AgentProfile> = {}): AgentProfile {
  return {
    id: "a1",
    name: "Test Agent",
    role: "backend",
    note: "",
    seed: "seed-a1",
    createdAt: Date.now(),
    deskIndex: 0,
    ...overrides,
  };
}

function mkNotifEvent(overrides: Partial<NotificationEvent> = {}): NotificationEvent {
  notifSeq += 1;
  return {
    id: `n${notifSeq}`,
    sessionId: "s1",
    agentId: "a1",
    source: "bell",
    message: "test message",
    dedupKey: `dedup-${notifSeq}`,
    at: Date.now(),
    ...overrides,
  };
}

const initialState = useAppStore.getState();

beforeEach(() => {
  notifSeq = 0;
  setAppSettingsMock.mockClear();
  useAppStore.setState(initialState, true);
});

describe("addAgent", () => {
  it("seeds session as starting (T1)", () => {
    const s = useAppStore.getState();
    s.addAgent(mkProfile({ id: "a1" }));

    const st = useAppStore.getState();
    expect(st.agentOrder).toContain("a1");
    expect(st.sessions.a1.status).toBe("starting");
    expect(st.sessions.a1.cols).toBe(80);
    expect(st.sessions.a1.rows).toBe(24);
    expect(st.agents.a1.id).toBe("a1");
  });
});

describe("openTerminal", () => {
  it("clears only that agent's notifications and updates active/recent (T2)", () => {
    const s = useAppStore.getState();
    s.addAgent(mkProfile({ id: "a1" }));
    s.addAgent(mkProfile({ id: "a2" }));
    s.pushNotification(mkNotifEvent({ agentId: "a1" }));
    s.pushNotification(mkNotifEvent({ agentId: "a2" }));

    s.openTerminal("a1");

    const st = useAppStore.getState();
    expect(st.activeTerminalAgentId).toBe("a1");
    expect(st.recentAgentIds[0]).toBe("a1");
    expect(st.notifications.every((n) => n.agentId !== "a1")).toBe(true);
    expect(st.notifications.some((n) => n.agentId === "a2")).toBe(true);
  });

  it("does nothing for an unknown agentId", () => {
    const s = useAppStore.getState();
    s.openTerminal("ghost");
    expect(useAppStore.getState().activeTerminalAgentId).toBeNull();
  });
});

describe("pushNotification", () => {
  it("suppresses notifications for the active agent (T3)", () => {
    const s = useAppStore.getState();
    s.addAgent(mkProfile({ id: "a1" }));
    s.openTerminal("a1");

    s.pushNotification(mkNotifEvent({ agentId: "a1", source: "hook", message: "need input" }));

    expect(useAppStore.getState().notifications).toHaveLength(0);
  });

  it("derives type from source and keeps newest first", () => {
    const s = useAppStore.getState();
    s.addAgent(mkProfile({ id: "a1" }));
    s.pushNotification(mkNotifEvent({ agentId: "a1", source: "hook", at: 100 }));
    s.pushNotification(mkNotifEvent({ agentId: "a1", source: "stop", at: 200 }));

    const [first, second] = useAppStore.getState().notifications;
    expect(first.type).toBe("done");
    expect(second.type).toBe("question");
  });

  it("truncates long messages to an 80-char excerpt", () => {
    const s = useAppStore.getState();
    s.addAgent(mkProfile({ id: "a1" }));
    const long = "x".repeat(200);
    s.pushNotification(mkNotifEvent({ agentId: "a1", message: long }));

    const n = useAppStore.getState().notifications[0];
    expect(n.message).toBe(long);
    expect(n.excerpt.length).toBe(80);
    expect(n.excerpt.endsWith("…")).toBe(true);
  });
});

describe("removeAgent", () => {
  it("cleans up sessions, notifications, recent list and active terminal", () => {
    const s = useAppStore.getState();
    s.addAgent(mkProfile({ id: "a1" }));
    s.openTerminal("a1");
    s.pushNotification(mkNotifEvent({ agentId: "a1" }));

    s.removeAgent("a1");

    const st = useAppStore.getState();
    expect(st.agents.a1).toBeUndefined();
    expect(st.sessions.a1).toBeUndefined();
    expect(st.agentOrder).not.toContain("a1");
    expect(st.recentAgentIds).not.toContain("a1");
    expect(st.notifications).toHaveLength(0);
    expect(st.activeTerminalAgentId).toBeNull();
  });
});

describe("assignDesk (책상 수동 지정)", () => {
  it("지정한 에이전트에 assignedDeskIndex를 기록한다", () => {
    const s = useAppStore.getState();
    s.addAgent(mkProfile({ id: "a1" }));

    s.assignDesk(3, "a1");

    expect(useAppStore.getState().agents.a1.assignedDeskIndex).toBe(3);
  });

  it("같은 책상을 다른 에이전트에 지정하면 기존 주인 지정이 풀린다 (책상당 1명)", () => {
    const s = useAppStore.getState();
    s.addAgent(mkProfile({ id: "a1" }));
    s.addAgent(mkProfile({ id: "a2" }));
    s.assignDesk(3, "a1");

    s.assignDesk(3, "a2");

    const st = useAppStore.getState();
    expect(st.agents.a2.assignedDeskIndex).toBe(3);
    expect(st.agents.a1.assignedDeskIndex).toBeUndefined();
  });

  it("agentId=null이면 그 책상의 지정을 해제한다", () => {
    const s = useAppStore.getState();
    s.addAgent(mkProfile({ id: "a1" }));
    s.assignDesk(3, "a1");

    s.assignDesk(3, null);

    expect(useAppStore.getState().agents.a1.assignedDeskIndex).toBeUndefined();
  });

  it("다른 책상 지정은 건드리지 않는다", () => {
    const s = useAppStore.getState();
    s.addAgent(mkProfile({ id: "a1" }));
    s.addAgent(mkProfile({ id: "a2" }));
    s.assignDesk(1, "a1");
    s.assignDesk(2, "a2");

    s.assignDesk(1, null);

    const st = useAppStore.getState();
    expect(st.agents.a1.assignedDeskIndex).toBeUndefined();
    expect(st.agents.a2.assignedDeskIndex).toBe(2);
  });

  it("이미 다른 책상을 가진 에이전트를 새 책상에 지정하면 이전 지정을 대체한다", () => {
    const s = useAppStore.getState();
    s.addAgent(mkProfile({ id: "a1" }));
    s.assignDesk(1, "a1");

    s.assignDesk(5, "a1");

    expect(useAppStore.getState().agents.a1.assignedDeskIndex).toBe(5);
  });
});

describe("bumpTerminalEpoch", () => {
  it("0에서 시작해 호출마다 1씩 증가한다", () => {
    const s = useAppStore.getState();
    s.addAgent(mkProfile({ id: "a1" }));
    expect(useAppStore.getState().terminalEpochs.a1).toBeUndefined();

    s.bumpTerminalEpoch("a1");
    expect(useAppStore.getState().terminalEpochs.a1).toBe(1);

    s.bumpTerminalEpoch("a1");
    expect(useAppStore.getState().terminalEpochs.a1).toBe(2);
  });

  it("removeAgent가 해당 에이전트의 terminalEpochs 항목도 제거한다", () => {
    const s = useAppStore.getState();
    s.addAgent(mkProfile({ id: "a1" }));
    s.bumpTerminalEpoch("a1");
    expect(useAppStore.getState().terminalEpochs.a1).toBe(1);

    s.removeAgent("a1");
    expect(useAppStore.getState().terminalEpochs.a1).toBeUndefined();
  });
});

describe("portrait cache", () => {
  it("setPortrait/removePortrait mutate the portraits map", () => {
    const s = useAppStore.getState();
    s.setPortrait("a1", "data:image/png;base64,AAA");
    expect(useAppStore.getState().portraits["a1"]).toBe("data:image/png;base64,AAA");
    s.removePortrait("a1");
    expect(useAppStore.getState().portraits["a1"]).toBeUndefined();
  });

  it("removeAgent also drops its cached portrait", () => {
    const s = useAppStore.getState();
    s.addAgent(mkProfile({ id: "a1" }));
    s.setPortrait("a1", "data:image/png;base64,AAA");
    s.removeAgent("a1");
    expect(useAppStore.getState().portraits["a1"]).toBeUndefined();
  });
});

describe("hydrate", () => {
  it("seeds agents/sessions from persisted state as idle", () => {
    const s = useAppStore.getState();
    s.hydrate({
      version: 1,
      agents: [mkProfile({ id: "a1", createdAt: 42 })],
    });

    const st = useAppStore.getState();
    expect(st.agentOrder).toEqual(["a1"]);
    expect(st.sessions.a1.status).toBe("idle");
    expect(st.sessions.a1.lastActivityAt).toBe(42);
  });

  it("backfills missing archetype to 'human' (legacy profiles stay human)", () => {
    const s = useAppStore.getState();
    s.hydrate({
      version: 1,
      agents: [
        mkProfile({ id: "legacy", createdAt: 1 }), // archetype 없음
        mkProfile({ id: "orc1", createdAt: 2, archetype: "orc" }),
      ],
    });
    const st = useAppStore.getState();
    expect(st.agents.legacy.archetype).toBe("human");
    expect(st.agents.orc1.archetype).toBe("orc"); // 명시값은 보존
  });
});

describe("timeTracking slice", () => {
  it("applyActivityEvent(prompt) opens a working turn", () => {
    const s = useAppStore.getState();
    s.applyActivityEvent({ agentId: "a1", sessionId: "s1", kind: "prompt", at: 1000 });
    const t = useAppStore.getState().timeTracking["a1"];
    expect(t.phase).toBe("working");
    expect(t.turnStartedAt).toBe(1000);
  });

  it("prompt → stop settles one turn with backend timestamps", () => {
    const s = useAppStore.getState();
    s.applyActivityEvent({ agentId: "a1", sessionId: "s1", kind: "prompt", at: 0 });
    s.applyNotificationTiming({
      id: "n1", sessionId: "s1", agentId: "a1", source: "stop",
      message: "done", dedupKey: "k", at: 4000,
    });
    const t = useAppStore.getState().timeTracking["a1"];
    expect(t.phase).toBe("idle");
    expect(t.turns).toBe(1);
    expect(t.totalMs).toBe(4000);
    expect(t.workedMs).toBe(4000);
  });

  it("notification(hook/bell) drives working→waiting; tool resumes", () => {
    const s = useAppStore.getState();
    s.applyActivityEvent({ agentId: "a1", sessionId: "s1", kind: "prompt", at: 0 });
    s.applyNotificationTiming({
      id: "n1", sessionId: "s1", agentId: "a1", source: "hook",
      message: "?", dedupKey: "k", at: 1000,
    });
    expect(useAppStore.getState().timeTracking["a1"].phase).toBe("waiting");
    s.applyActivityEvent({ agentId: "a1", sessionId: "s1", kind: "tool", at: 3000 });
    expect(useAppStore.getState().timeTracking["a1"].phase).toBe("working");
    expect(useAppStore.getState().timeTracking["a1"].waitedInTurnMs).toBe(2000);
  });

  it("applySessionTiming(exited) force-settles an open turn; running is ignored", () => {
    const s = useAppStore.getState();
    s.applyActivityEvent({ agentId: "a1", sessionId: "s1", kind: "prompt", at: 0 });
    s.applySessionTiming("a1", "running", 2000); // ignored
    expect(useAppStore.getState().timeTracking["a1"].phase).toBe("working");
    s.applySessionTiming("a1", "exited", 5000); // force settle
    const t = useAppStore.getState().timeTracking["a1"];
    expect(t.phase).toBe("idle");
    expect(t.turns).toBe(1);
    expect(t.totalMs).toBe(5000);
  });

  it("removeAgent drops the agent's timeTracking entry", () => {
    const s = useAppStore.getState();
    s.addAgent(mkProfile({ id: "a1" }));
    s.applyActivityEvent({ agentId: "a1", sessionId: "s1", kind: "prompt", at: 0 });
    s.removeAgent("a1");
    expect(useAppStore.getState().timeTracking["a1"]).toBeUndefined();
  });
});

describe("app settings slice", () => {
  it("초기값: 전부 OFF, firstRun=false", () => {
    const s = useAppStore.getState();
    expect(s.appSettings.claudeCliEnabled).toBe(false);
    expect(s.appSettings.claudeHooksEnabled).toBe(false);
    expect(s.settingsFirstRun).toBe(false);
  });

  it("hydrateSettings가 설정과 firstRun을 반영한다", () => {
    useAppStore.getState().hydrateSettings(
      { version: 1, claudeCliEnabled: true, claudeHooksEnabled: false },
      true
    );
    const s = useAppStore.getState();
    expect(s.appSettings.claudeCliEnabled).toBe(true);
    expect(s.settingsFirstRun).toBe(true);
  });

  it("updateAppSettings가 스토어를 갱신하고 백엔드에 저장한다", () => {
    useAppStore.getState().updateAppSettings({ claudeHooksEnabled: true });
    expect(useAppStore.getState().appSettings.claudeHooksEnabled).toBe(true);
    expect(setAppSettingsMock).toHaveBeenCalledWith(
      expect.objectContaining({ claudeHooksEnabled: true })
    );
  });

  it("completeFirstRun이 선택을 저장하고 firstRun을 끈다", () => {
    useAppStore.getState().hydrateSettings(
      { version: 1, claudeCliEnabled: false, claudeHooksEnabled: false },
      true
    );
    useAppStore.getState().completeFirstRun({ claudeCliEnabled: true, claudeHooksEnabled: true });
    const s = useAppStore.getState();
    expect(s.settingsFirstRun).toBe(false);
    expect(s.appSettings.claudeCliEnabled).toBe(true);
  });
});
