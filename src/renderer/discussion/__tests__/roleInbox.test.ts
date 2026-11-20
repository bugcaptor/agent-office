import { beforeEach, describe, expect, it, vi } from "vitest";
import type { DiscussionRole, RolesFile } from "@shared/types";

const api = vi.hoisted(() => ({
  loadRoles: vi.fn<() => Promise<RolesFile | null>>(),
  saveRoles: vi.fn<(file: RolesFile) => Promise<void>>(),
  scanRoleInbox: vi.fn<() => Promise<RolesFile | null>>(),
}));
vi.mock("../../ipc/tauriApi", () => ({ tauriApi: api }));

import { receiveRoleInbox } from "../discussionInbox";
import { resetRoleStoreForTest, useRoleStore } from "../roleStore";

const role = (id: string, name: string): DiscussionRole => ({ id, name, perspective: "", stance: "critical", criteria: "" });

beforeEach(() => {
  resetRoleStoreForTest();
  api.loadRoles.mockReset();
  api.saveRoles.mockReset().mockResolvedValue(undefined);
  api.scanRoleInbox.mockReset();
});

describe("receiveRoleInbox", () => {
  it("seeds presets first so the backend can apply, then adopts the applied list", async () => {
    api.loadRoles.mockResolvedValue(null);
    api.scanRoleInbox.mockImplementation(async () => {
      // 시드 저장이 끝난 뒤에 수신함을 읽어야 백엔드가 적용한다.
      expect(api.saveRoles).toHaveBeenCalledTimes(1);
      return { version: 1, roles: [role("r1", "보안 검토자")] };
    });
    await receiveRoleInbox();
    expect(useRoleStore.getState()).toMatchObject({ status: "ready", roles: [role("r1", "보안 검토자")] });
  });

  it("keeps the cached list when nothing changed and never scans over an unreadable file", async () => {
    api.loadRoles.mockResolvedValue({ version: 1, roles: [role("r1", "비판자")] });
    api.scanRoleInbox.mockResolvedValue(null);
    await receiveRoleInbox();
    expect(useRoleStore.getState().roles).toEqual([role("r1", "비판자")]);

    resetRoleStoreForTest();
    api.scanRoleInbox.mockClear();
    api.loadRoles.mockClear();
    api.loadRoles.mockRejectedValue("roles-corrupt: x");
    await receiveRoleInbox();
    await receiveRoleInbox();
    expect(api.scanRoleInbox).not.toHaveBeenCalled();
    expect(api.loadRoles).toHaveBeenCalledTimes(1);
  });

  it("waits while the role list dialog is editing its copy", async () => {
    api.loadRoles.mockResolvedValue({ version: 1, roles: [] });
    api.scanRoleInbox.mockResolvedValue({ version: 1, roles: [role("r1", "새 역할")] });
    await useRoleStore.getState().ensureLoaded();
    useRoleStore.setState({ editing: true });
    await receiveRoleInbox();
    expect(api.scanRoleInbox).not.toHaveBeenCalled();
    useRoleStore.setState({ editing: false });
    await receiveRoleInbox();
    expect(useRoleStore.getState().roles).toEqual([role("r1", "새 역할")]);
  });
});
