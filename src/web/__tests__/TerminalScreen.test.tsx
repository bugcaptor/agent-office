// @vitest-environment jsdom
//
// 웹 터미널의 복구 계약: 연결별 attach 상태가 사라져도 마지막 offset으로 다시
// 붙고, 링 델타 restore는 현재 화면을 보존하며, 실제 snapshot만 화면을 교체한다.

import { act, cleanup, render } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import type { HostMsg, RemoteAgent } from "@web/protocol";
import type { ConnState, WebRemoteSocket } from "@web/ws";

const xterm = vi.hoisted(() => {
  class FakeTerminal {
    static instances: FakeTerminal[] = [];
    readonly writes: string[] = [];
    readonly reset = vi.fn();
    readonly open = vi.fn();
    readonly resize = vi.fn();
    readonly dispose = vi.fn();
    readonly onData = vi.fn(() => ({ dispose: vi.fn() }));
    cols: number;
    rows: number;
    options: Record<string, unknown>;

    constructor(options: { cols: number; rows: number }) {
      this.cols = options.cols;
      this.rows = options.rows;
      this.options = { ...options };
      FakeTerminal.instances.push(this);
    }

    write(data: string): void {
      this.writes.push(data);
    }
  }
  return { FakeTerminal };
});

vi.mock("@xterm/xterm", () => ({ Terminal: xterm.FakeTerminal }));

import { TerminalScreen } from "@web/TerminalScreen";

const agent: RemoteAgent = { agentId: "a1", name: "아다", cols: 80, rows: 24 };
const noop = () => {};

function fakeSocket(initial: ConnState = "open") {
  const messageListeners = new Set<(msg: HostMsg) => void>();
  const stateListeners = new Set<(state: ConnState) => void>();
  const socket = {
    send: vi.fn(),
    onMessage(cb: (msg: HostMsg) => void) {
      messageListeners.add(cb);
      return () => messageListeners.delete(cb);
    },
    onState(cb: (state: ConnState) => void) {
      stateListeners.add(cb);
      cb(initial);
      return () => stateListeners.delete(cb);
    },
  } as unknown as WebRemoteSocket;

  const push = (msg: HostMsg) =>
    act(() => {
      for (const cb of messageListeners) cb(msg);
    });
  const state = (next: ConnState) =>
    act(() => {
      for (const cb of stateListeners) cb(next);
    });
  return { socket, push, state, send: socket.send };
}

function mount(socket: WebRemoteSocket) {
  return render(
    <TerminalScreen
      socket={socket}
      agent={agent}
      permission="input"
      onBack={noop}
      onOpenChat={noop}
    />
  );
}

afterEach(() => {
  cleanup();
  xterm.FakeTerminal.instances.length = 0;
});

describe("TerminalScreen recovery", () => {
  it("re-attaches from the last consumed offset when the socket reopens", () => {
    const { socket, push, state, send } = fakeSocket("closed");
    mount(socket);
    expect(send).not.toHaveBeenCalled();

    state("open");
    expect(send).toHaveBeenLastCalledWith({
      type: "attach", agentId: "a1", lastOffset: null, lastSessionId: null,
    });
    push({ type: "output", agentId: "a1", sessionId: "s", seq: 1, offset: 0, bytes: 3, data: "one" });

    state("closed");
    state("open");
    expect(send).toHaveBeenLastCalledWith({
      type: "attach", agentId: "a1", lastOffset: 3, lastSessionId: "s",
    });
  });

  it("keeps the session ID with the offset when reconnecting", () => {
    const { socket, push, state, send } = fakeSocket();
    mount(socket);
    push({ type: "restore", agentId: "a1", snapshot: "old", baseOffset: 7, sessionId: "s1" });

    state("closed");
    state("open");
    expect(send).toHaveBeenLastCalledWith({
      type: "attach", agentId: "a1", lastOffset: 7, lastSessionId: "s1",
    });
  });

  it("re-attaches instead of applying an output from a replacement session", () => {
    const { socket, push, send } = fakeSocket();
    mount(socket);
    push({ type: "restore", agentId: "a1", snapshot: "old", baseOffset: 3, sessionId: "s1" });
    push({ type: "output", agentId: "a1", sessionId: "s2", seq: 1, offset: 0, bytes: 3, data: "new" });

    expect(send).toHaveBeenLastCalledWith({
      type: "attach", agentId: "a1", lastOffset: null, lastSessionId: null,
    });
  });

  it("keeps the existing screen for a snapshot:null delta restore", () => {
    const { socket, push } = fakeSocket();
    mount(socket);
    const term = xterm.FakeTerminal.instances[0];
    push({ type: "output", agentId: "a1", sessionId: "s", seq: 1, offset: 0, bytes: 4, data: "old\n" });
    push({ type: "restore", agentId: "a1", snapshot: null, baseOffset: 4 });

    expect(term.reset).not.toHaveBeenCalled();
    expect(term.writes).toEqual(["old\n"]);
  });

  it("resets and writes when restore carries a real snapshot", () => {
    const { socket, push } = fakeSocket();
    mount(socket);
    const term = xterm.FakeTerminal.instances[0];
    push({ type: "restore", agentId: "a1", snapshot: "SCREEN", baseOffset: 10 });

    expect(term.reset).toHaveBeenCalledTimes(1);
    expect(term.writes).toEqual(["SCREEN"]);
  });

  it("treats an empty string as a real empty snapshot, not a delta restore", () => {
    const { socket, push } = fakeSocket();
    mount(socket);
    const term = xterm.FakeTerminal.instances[0];
    push({ type: "restore", agentId: "a1", snapshot: "", baseOffset: 10 });

    expect(term.reset).toHaveBeenCalledTimes(1);
    expect(term.writes).toEqual([]);
  });
});
