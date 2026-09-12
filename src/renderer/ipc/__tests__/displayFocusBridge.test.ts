import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { AgentProfile } from "@shared/types";

const { listen, emit } = vi.hoisted(() => {
  let handler: ((event: { payload: { agentId?: unknown; intent?: unknown } }) => void) | undefined;
  return {
    listen: vi.fn((_event: string, callback: typeof handler) => {
      handler = callback;
      return Promise.resolve(vi.fn());
    }),
    emit: (payload: { agentId?: unknown; intent?: unknown }) => handler?.({ payload }),
  };
});

vi.mock("@tauri-apps/api/event", () => ({ listen }));

import { useAppStore } from "../../store/appStore";
import { installDisplayFocusBridge } from "../displayFocusBridge";

const initialState = useAppStore.getState();
const flush = () => new Promise((resolve) => setTimeout(resolve, 0));
const agent = (overrides: Partial<AgentProfile> = {}): AgentProfile => ({
  id: "a1", name: "Ada", role: "backend", seed: "seed", createdAt: 1, deskIndex: 0,
  cwd: "/work/demo", ...overrides,
});
let dispose = () => {};

beforeEach(async () => {
  useAppStore.setState(initialState, true);
  vi.clearAllMocks();
  dispose = installDisplayFocusBridge();
  await flush();
});
afterEach(() => dispose());

describe("installDisplayFocusBridge", () => {
  it("keeps the legacy profile editor request and ignores an unknown agent", () => {
    useAppStore.getState().addAgent(agent(), { startSession: false });
    emit({ agentId: "a1" });
    expect(useAppStore.getState().modal).toEqual({ kind: "profile-edit", agentId: "a1" });
    emit({ agentId: "missing" });
    expect(useAppStore.getState().modal).toEqual({ kind: "profile-edit", agentId: "a1" });
  });

  it("opens a Codex IDE connection dialog prefilled from the requested agent", () => {
    useAppStore.getState().addAgent(agent(), { startSession: false });
    emit({ agentId: "a1", intent: "connectCodex" });
    expect(useAppStore.getState().modal).toEqual({
      kind: "ide-session", initialAgentId: "a1", initialCwd: "/work/demo", initialProvider: "codex",
    });
  });
});
