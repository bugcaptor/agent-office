import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

// tauriApi를 콜백 캡처형으로 모킹.
const captured: {
  activity?: (e: any) => void;
  notif?: (e: any) => void;
  state?: (e: any) => void;
} = {};

vi.mock("../tauriApi", () => ({
  tauriApi: {
    onSessionState: (cb: any) => ((captured.state = cb), () => {}),
    onNotification: (cb: any) => ((captured.notif = cb), () => {}),
    onNotificationCleared: () => () => {},
    onActivity: (cb: any) => ((captured.activity = cb), () => {}),
    setBadgeCount: vi.fn(),
    appendSessionTurn: vi.fn(),
  },
}));

import { installSessionBridge } from "../sessionBridge";
import { useAppStore } from "../../store/appStore";

const initial = useAppStore.getState();
let teardown: () => void;

beforeEach(() => {
  useAppStore.setState(initial, true);
  teardown = installSessionBridge();
});
afterEach(() => teardown());

describe("sessionBridge time-tracking wiring", () => {
  it("activity events feed the turn reducer", () => {
    captured.activity!({ agentId: "a1", sessionId: "s1", kind: "prompt", at: 1000 });
    expect(useAppStore.getState().timeTracking["a1"].phase).toBe("working");
  });

  it("notification events (stop) settle a turn even for the active terminal agent", () => {
    // 활성 터미널이면 pushNotification은 억제되지만 시간 집계는 계속돼야 한다.
    useAppStore.setState({ activeTerminalAgentId: "a1" });
    captured.activity!({ agentId: "a1", sessionId: "s1", kind: "prompt", at: 0 });
    captured.notif!({
      id: "n1", sessionId: "s1", agentId: "a1", source: "stop",
      message: "done", dedupKey: "k", at: 3000,
    });
    const t = useAppStore.getState().timeTracking["a1"];
    expect(t.turns).toBe(1);
    expect(t.totalMs).toBe(3000);
  });

  it("session exited force-settles the open turn", () => {
    captured.activity!({ agentId: "a1", sessionId: "s1", kind: "prompt", at: 0 });
    captured.state!({ agentId: "a1", sessionId: "s1", state: "exited", at: 7000 });
    const t = useAppStore.getState().timeTracking["a1"];
    expect(t.phase).toBe("idle");
    expect(t.totalMs).toBe(7000);
  });
});
