// src/renderer/portrait/__tests__/portraitCache.test.ts
//
// portraitCache 순수 헬퍼 + 삭제 브리지(installPortraitCache) TDD.
// tauriApi만 목킹(bootstrap.test.ts와 동일 관례).
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { AgentProfile } from "../../store/types";

const { mockApi } = vi.hoisted(() => ({
  mockApi: {
    loadPortrait: vi.fn(),
    deletePortrait: vi.fn(),
  },
}));
vi.mock("../../ipc/tauriApi", () => ({ tauriApi: mockApi }));

import { useAppStore } from "../../store/appStore";
import {
  agentsNeedingPortraits,
  pngBase64ToDataUrl,
  loadPortraitsFor,
  installPortraitCache,
} from "../portraitCache";

const initial = useAppStore.getState();

function mkProfile(overrides: Partial<AgentProfile> = {}): AgentProfile {
  return {
    id: "a1",
    name: "Ada",
    role: "backend",
    note: "",
    seed: "seed",
    createdAt: 1,
    deskIndex: 0,
    ...overrides,
  };
}

beforeEach(() => {
  useAppStore.setState(initial, true);
  mockApi.loadPortrait.mockReset();
  mockApi.deletePortrait.mockReset();
  mockApi.deletePortrait.mockResolvedValue(undefined);
});
afterEach(() => vi.restoreAllMocks());

describe("agentsNeedingPortraits", () => {
  it("returns only ids whose profile has portraitUpdatedAt", () => {
    const agents = {
      a1: mkProfile({ id: "a1", portraitUpdatedAt: 123 }),
      a2: mkProfile({ id: "a2" }),
      a3: mkProfile({ id: "a3", portraitUpdatedAt: 456 }),
    };
    expect(agentsNeedingPortraits(agents).sort()).toEqual(["a1", "a3"]);
  });
});

describe("pngBase64ToDataUrl", () => {
  it("prefixes with the png data url header", () => {
    expect(pngBase64ToDataUrl("ABC")).toBe("data:image/png;base64,ABC");
  });
});

describe("loadPortraitsFor", () => {
  it("caches a dataUrl for each id that returns base64", async () => {
    mockApi.loadPortrait.mockImplementation(async (id: string) =>
      id === "a1" ? "AAA" : null
    );
    await loadPortraitsFor(["a1", "a2"]);
    const { portraits } = useAppStore.getState();
    expect(portraits["a1"]).toBe("data:image/png;base64,AAA");
    expect(portraits["a2"]).toBeUndefined();
  });

  it("swallows a load failure and continues", async () => {
    mockApi.loadPortrait.mockRejectedValue(new Error("boom"));
    await expect(loadPortraitsFor(["a1"])).resolves.toBeUndefined();
  });
});

describe("installPortraitCache", () => {
  it("deletes the backend portrait when an agent is removed", async () => {
    const s = useAppStore.getState();
    s.addAgent(mkProfile({ id: "a1", portraitUpdatedAt: 1 }));
    const off = installPortraitCache();

    s.removeAgent("a1");
    // subscription fired synchronously; the async delete was dispatched.
    expect(mockApi.deletePortrait).toHaveBeenCalledWith("a1");
    off();
  });
});
