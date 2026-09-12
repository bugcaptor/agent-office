import { beforeEach, describe, expect, it, vi } from "vitest";

const mocks = vi.hoisted(() => {
  const instances: unknown[] = [];
  class Channel<T> {
    onmessage: ((value: T) => void) | null = null;
    constructor() { instances.push(this); }
  }
  return { invoke: vi.fn(() => Promise.resolve(undefined)), instances, Channel };
});
const invoke = mocks.invoke;
const MockChannel = mocks.Channel;

const applyRemoteMessage = vi.fn(() => Promise.resolve());
const store = {
  agentOrder: ["a"],
  agents: { a: { id: "a", name: "A" } },
  vacationMode: false,
};

vi.mock("@tauri-apps/api/core", () => ({ invoke: mocks.invoke, Channel: mocks.Channel }));
vi.mock("@tauri-apps/api/event", () => ({ listen: vi.fn(() => Promise.resolve(vi.fn())) }));
vi.mock("../../terminal/TerminalRegistry", () => ({ terminalRegistry: { applyRemoteMessage } }));
vi.mock("../../store/appStore", () => ({ useAppStore: { getState: () => store } }));

import { createRemoteApi, connectRemote, setRemoteOffset, acknowledgeRemoteOffset } from "../remoteApi";

const snapshot = {
  state: { agents: [], version: 1 as const },
  settings: {} as never,
  sessions: [],
  revision: "r1",
};

describe("remoteApi terminal channel", () => {
  beforeEach(() => {
    invoke.mockClear();
    applyRemoteMessage.mockClear();
    mocks.instances.length = 0;
  });

  it("serializes lowercase restore, consecutive output, and resize before ACK", async () => {
    const api = createRemoteApi(snapshot);
    api.onData("a", vi.fn());
    const channel = mocks.instances[0] as InstanceType<typeof MockChannel>;
    let release!: () => void;
    applyRemoteMessage.mockImplementationOnce(() => new Promise<void>((resolve) => { release = resolve; }));
    channel.onmessage!({ type: "restore", agentId: "a", snapshot: "", baseOffset: 0, cols: 80, rows: 24 });
    channel.onmessage!({ type: "output", agentId: "a", data: "one", offset: 0, bytes: 3 });
    channel.onmessage!({ type: "output", agentId: "a", data: "two", offset: 3, bytes: 3 });
    channel.onmessage!({ type: "resized", agentId: "a", cols: 100, rows: 30 });
    await new Promise((resolve) => setTimeout(resolve, 0));
    expect(applyRemoteMessage).toHaveBeenCalledTimes(1);
    release();
    await new Promise((resolve) => setTimeout(resolve, 0));
    expect((applyRemoteMessage.mock.calls as unknown as [ { type: string } ][]).map(([frame]) => frame.type)).toEqual(["restore", "output", "output", "resized"]);
  });

  it("ignores an ACK from a generation replaced by restore", async () => {
    await connectRemote("http://localhost", "test");
    const oldGeneration = setRemoteOffset("a", 10);
    setRemoteOffset("a", 0);
    acknowledgeRemoteOffset("a", 20, oldGeneration);
    expect(invoke).not.toHaveBeenCalledWith("remote_ack", expect.anything());
  });

  it("ACKs a rendered snapshot base so native backpressure can resume", async () => {
    await connectRemote("http://localhost", "test");
    const offset = 2 * 1024 * 1024;
    const generation = setRemoteOffset("snapshot", offset, "session", "current");
    acknowledgeRemoteOffset("snapshot", offset, generation);
    expect(invoke).toHaveBeenCalledWith("remote_ack", { agentId: "snapshot", offset, restoreId: "current" });
  });

  it("re-subscribes when an output offset has a gap", async () => {
    const api = createRemoteApi(snapshot);
    api.onData("a", vi.fn());
    const channel = mocks.instances[0] as InstanceType<typeof MockChannel>;
    channel.onmessage!({ type: "restore", agentId: "a", snapshot: null, baseOffset: 0, cols: 80, rows: 24 });
    channel.onmessage!({ type: "output", agentId: "a", data: "gap", offset: 9, bytes: 3 });
    await Promise.resolve();
    expect(invoke).toHaveBeenCalledWith("remote_unsubscribe", { agentId: "a" });
  });

  it("does not replay an already queued output after same-session null restore", async () => {
    const api = createRemoteApi(snapshot);
    api.onData("a", vi.fn());
    const channel = mocks.instances[0] as InstanceType<typeof MockChannel>;
    channel.onmessage!({ type: "restore", agentId: "a", sessionId: "s1", snapshot: null, baseOffset: 0, cols: 80, rows: 24 });
    channel.onmessage!({ type: "output", agentId: "a", sessionId: "s1", data: "once", offset: 0, bytes: 4 });
    channel.onmessage!({ type: "restore", agentId: "a", sessionId: "s1", snapshot: null, baseOffset: 0, cols: 80, rows: 24 });
    channel.onmessage!({ type: "output", agentId: "a", sessionId: "s1", data: "once", offset: 0, bytes: 4 });
    await new Promise((resolve) => setTimeout(resolve, 0));
    const outputs = (applyRemoteMessage.mock.calls as unknown as [ { type: string } ][]).map(([frame]) => frame).filter((frame) => frame.type === "output");
    expect(outputs).toHaveLength(1);
  });

  it("saves the latest profile state before starting its session", async () => {
    (invoke as unknown as { mockImplementation(fn: (command: string) => Promise<unknown>): void }).mockImplementation((command: string) => {
      if (command === "remote_rpc") return Promise.resolve({ revision: "r2" });
      return Promise.resolve(undefined);
    });
    const api = createRemoteApi(snapshot);
    await api.createSession("a");
    const calls = (invoke.mock.calls as unknown as [string, { cmd: string }][]).filter(([command]) => command === "remote_rpc");
    expect(calls.map(([, args]) => args.cmd)).toEqual(["office.saveState", "session.start"]);
  });
});


describe("remote profile files", () => {
  beforeEach(() => { invoke.mockReset(); });

  it("uses client file dialogs and host media without invoking local image storage", async () => {
    const api = createRemoteApi(snapshot);
    await api.exportCharacterFile("agent.character.json", "bundle");
    await api.importCharacterFile();
    expect(invoke).toHaveBeenCalledWith("export_character_file", { defaultName: "agent.character.json", content: "bundle" });
    expect(invoke).toHaveBeenCalledWith("import_character_file");
    invoke.mockClear();
    await api.loadPortrait("a");
    await api.loadSprite("a");
    await api.loadMinimi("a");
    await api.savePortrait("a", "png");
    await api.saveSprite("a", "png");
    await api.saveMinimi("a", "png");
    await api.deletePortrait("a");
    await api.deleteSprite("a");
    await api.deleteMinimi("a");
    const calls = invoke.mock.calls as unknown as [string, {cmd: string; args: unknown}][];
    expect(calls.every(([command]) => command === "remote_rpc")).toBe(true);
    expect(calls.map(([, request]) => request)).toEqual([
      {cmd: "media.portrait", args: {agentId: "a"}},
      ...["sprite", "minimi"].map((kind) => ({cmd: "office.media.load", args: {agentId: "a", kind}})),
      ...["portrait", "sprite", "minimi"].map((kind) => ({cmd: "office.media.save", args: {agentId: "a", kind, pngBase64: "png"}})),
      ...["portrait", "sprite", "minimi"].map((kind) => ({cmd: "office.media.delete", args: {agentId: "a", kind}})),
    ]);
  });

  it("does not fall back to local media when the server rejects a request", async () => {
    invoke.mockRejectedValueOnce(new Error("forbidden"));
    await expect(createRemoteApi(snapshot).saveSprite("a", "png")).rejects.toThrow("forbidden");
    expect(invoke).toHaveBeenCalledTimes(1);
    expect(invoke).toHaveBeenCalledWith("remote_rpc", {cmd: "office.media.save", args: {agentId: "a", kind: "sprite", pngBase64: "png"}});
  });
});
