import { beforeEach, describe, expect, it, vi } from "vitest";
import type { RolesFile } from "@shared/types";

const api = vi.hoisted(() => ({
  loadRoles: vi.fn<() => Promise<RolesFile | null>>(),
  saveRoles: vi.fn<(file: RolesFile) => Promise<void>>(),
}));
vi.mock("../../ipc/tauriApi", () => ({ tauriApi: api }));

import { ROLE_PRESETS, normalizeRoles, presetRoles, resetRoleStoreForTest, useRoleStore } from "../roleStore";

beforeEach(() => {
  resetRoleStoreForTest();
  api.loadRoles.mockReset();
  api.saveRoles.mockReset().mockResolvedValue(undefined);
});

describe("roleStore", () => {
  it("seeds translated presets once when the file is missing", async () => {
    api.loadRoles.mockResolvedValue(null);
    await Promise.all([useRoleStore.getState().ensureLoaded(), useRoleStore.getState().ensureLoaded()]);
    expect(api.loadRoles).toHaveBeenCalledTimes(1);
    expect(api.saveRoles).toHaveBeenCalledTimes(1);
    const saved = api.saveRoles.mock.calls[0][0];
    expect(saved.version).toBe(1);
    expect(saved.roles.map((r) => r.id)).toEqual(ROLE_PRESETS.map((p) => `preset-${p.key}`));
    expect(saved.roles[0]).toMatchObject({ name: "사회자", stance: "moderator" });
    expect(saved.roles.every((r) => r.name && r.perspective && r.criteria && !r.name.includes("presets."))).toBe(true);
    expect(useRoleStore.getState()).toMatchObject({ status: "ready", roles: saved.roles });
  });

  it("keeps an empty saved list instead of re-seeding", async () => {
    api.loadRoles.mockResolvedValue({ version: 1, roles: [] });
    await useRoleStore.getState().ensureLoaded();
    expect(api.saveRoles).not.toHaveBeenCalled();
    expect(useRoleStore.getState()).toMatchObject({ status: "ready", roles: [] });
  });

  it("never seeds or saves when the file cannot be read", async () => {
    api.loadRoles.mockRejectedValue("roles-corrupt: x");
    await useRoleStore.getState().ensureLoaded();
    expect(api.saveRoles).not.toHaveBeenCalled();
    expect(useRoleStore.getState()).toMatchObject({ status: "error", roles: [] });
    // 다시 시도하면 다시 읽는다.
    api.loadRoles.mockResolvedValue({ version: 1, roles: presetRoles().slice(0, 1) });
    await useRoleStore.getState().ensureLoaded();
    expect(useRoleStore.getState().status).toBe("ready");
  });

  it("normalizeRoles trims text and rejects blank names", () => {
    const [role] = presetRoles();
    expect(normalizeRoles([{ ...role, name: " A ", criteria: " c " }])?.[0]).toMatchObject({ name: "A", criteria: "c" });
    expect(normalizeRoles([{ ...role, name: "  " }])).toBeNull();
  });
});
