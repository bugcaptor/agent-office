// @vitest-environment jsdom
//
// src/renderer/shortcuts/__tests__/agentShortcuts.test.ts
//
// 단축키 → 열 에이전트 결정과, 리스너가 터미널(xterm)보다 먼저 키를 가로채는지.
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { AgentProfile } from "@shared/types";
import type { Notification } from "../../store/types";

const { emitAgentClicked } = vi.hoisted(() => ({ emitAgentClicked: vi.fn() }));
vi.mock("../../ipc/sessionBridge", () => ({ officeBus: { emitAgentClicked } }));
vi.mock("../../ipc/tauriApi", () => ({ tauriApi: { setAppSettings: vi.fn().mockResolvedValue(undefined) } }));
vi.mock("../../shared/platform", () => ({ IS_MAC: false, IS_WINDOWS: false }));

import { installAgentShortcuts, resolveShortcut, type ShortcutContext } from "../agentShortcuts";
import { useAppStore } from "../../store/appStore";

function profile(id: string, extra: Partial<AgentProfile> = {}): AgentProfile {
  return { id, name: id, role: "", seed: id, createdAt: 0, deskIndex: 0, ...extra };
}

function note(agentId: string, createdAt: number): Notification {
  return { id: `${agentId}-${createdAt}`, agentId, type: "question", message: "", excerpt: "", createdAt };
}

const J = { code: "KeyJ", key: "j", ctrlKey: true, altKey: true, shiftKey: false, metaKey: false };
const K1 = { ...J, code: "Digit1", key: "1" };

function ctx(over: Partial<ShortcutContext> = {}): ShortcutContext {
  return {
    agentShortcuts: { a: "Ctrl+Alt+Digit1" },
    notifyJumpShortcut: "Ctrl+Alt+KeyJ",
    agents: { a: profile("a"), b: profile("b"), c: profile("c") },
    notifications: [],
    activeTerminalAgentId: null,
    ...over,
  };
}

describe("resolveShortcut", () => {
  it("에이전트 키는 그 에이전트를, 이미 앞에 열려 있으면 아무것도 하지 않는다", () => {
    expect(resolveShortcut(K1, ctx(), false)).toEqual({ agentId: "a" });
    expect(resolveShortcut(K1, ctx({ activeTerminalAgentId: "a" }), false)).toBe("consume");
  });

  it("퇴근한 에이전트는 열지 않는다", () => {
    const c = ctx({ agents: { a: profile("a", { clockedOut: true }) } });
    expect(resolveShortcut(K1, c, false)).toBe("consume");
  });

  it("알림 키는 가장 오래된 알림으로 가고, 열 수 없는 대상은 건너뛴다", () => {
    // 최신순 배열: c(3) → b(2) → gone(1)
    const notifications = [note("c", 3), note("b", 2), note("gone", 1)];
    expect(resolveShortcut(J, ctx({ notifications }), false)).toEqual({ agentId: "b" });
  });

  it("남은 알림이 없으면 키만 삼킨다", () => {
    expect(resolveShortcut(J, ctx(), false)).toBe("consume");
  });

  it("지정되지 않은 키는 지나간다", () => {
    expect(resolveShortcut({ ...K1, code: "Digit2", key: "2" }, ctx(), false)).toBeNull();
  });
});

describe("installAgentShortcuts", () => {
  const initial = useAppStore.getState();
  let off: () => void;

  beforeEach(() => {
    useAppStore.setState(initial, true);
    emitAgentClicked.mockClear();
    useAppStore.setState({
      agents: { a: profile("a") },
      appSettings: { ...initial.appSettings, agentShortcuts: { a: "Ctrl+Alt+Digit1" } },
    });
    off = installAgentShortcuts();
  });

  afterEach(() => {
    off();
    document.body.innerHTML = "";
  });

  it("포커스된 입력(터미널)보다 먼저 받아 키를 넘기지 않는다", () => {
    const input = document.createElement("textarea");
    document.body.appendChild(input);
    const reached = vi.fn();
    input.addEventListener("keydown", reached);
    const ev = new KeyboardEvent("keydown", { ...K1, bubbles: true, cancelable: true });
    input.dispatchEvent(ev);
    expect(emitAgentClicked).toHaveBeenCalledWith("a");
    expect(reached).not.toHaveBeenCalled();
    expect(ev.defaultPrevented).toBe(true);
  });

  it("모달이 열려 있으면 동작하지 않는다", () => {
    useAppStore.setState({ modal: { kind: "settings" } });
    window.dispatchEvent(new KeyboardEvent("keydown", { ...K1, cancelable: true }));
    expect(emitAgentClicked).not.toHaveBeenCalled();
  });
});
