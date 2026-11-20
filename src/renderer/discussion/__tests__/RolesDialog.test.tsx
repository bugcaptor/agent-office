// @vitest-environment jsdom
import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { DiscussionRole, RolesFile } from "@shared/types";

const api = vi.hoisted(() => ({
  loadRoles: vi.fn<() => Promise<RolesFile | null>>(),
  saveRoles: vi.fn<(file: RolesFile) => Promise<void>>(),
  workAgentModels: vi.fn().mockResolvedValue([]),
}));
vi.mock("../../ipc/tauriApi", () => ({ tauriApi: api }));

import { useAppStore } from "../../store/appStore";
import { resetRoleStoreForTest, useRoleStore } from "../roleStore";
import { RolesDialog } from "../RolesDialog";

const file: RolesFile = {
  version: 1,
  roles: [
    { id: "r1", name: "비판자", perspective: "약점", stance: "critical", criteria: "" },
    { id: "r2", name: "사회자", perspective: "", stance: "moderator", criteria: "" },
  ],
};

beforeEach(() => {
  resetRoleStoreForTest();
  api.loadRoles.mockReset().mockResolvedValue(structuredClone(file));
  api.saveRoles.mockReset().mockResolvedValue(undefined);
  useAppStore.setState({
    agents: { a1: { id: "a1", name: "Ada", role: "", seed: "s", createdAt: 0, deskIndex: 0, defaultRoleId: "r1" } },
  });
  act(() => useAppStore.getState().openModal({ kind: "roles" }));
});
afterEach(() => { cleanup(); act(() => useAppStore.getState().closeModal()); });

const saved = (): DiscussionRole[] => api.saveRoles.mock.calls[api.saveRoles.mock.calls.length - 1][0].roles;

describe("RolesDialog", () => {
  it("edits, duplicates, deletes and saves the whole list", async () => {
    render(<RolesDialog />);
    await screen.findByDisplayValue("비판자");
    expect(screen.getByText("기본 역할로 쓰는 팀원: Ada")).toBeTruthy();
    fireEvent.change(screen.getByDisplayValue("비판자"), { target: { value: " 깐깐한 비판자 " } });
    fireEvent.click(screen.getByText("복제"));
    expect(screen.getByDisplayValue("깐깐한 비판자 사본")).toBeTruthy();
    fireEvent.click(screen.getByText("사회자", { selector: ".roles-list-name" }));
    fireEvent.click(screen.getByText("삭제"));
    fireEvent.click(screen.getByText("저장"));
    await waitFor(() => expect(api.saveRoles).toHaveBeenCalled());
    const roles = saved();
    expect(roles.map((r) => r.name)).toEqual(["깐깐한 비판자", "깐깐한 비판자 사본"]);
    expect(roles[0].id).toBe("r1");
    expect(roles[1].id).not.toBe("r1");
    expect(roles[1]).toMatchObject({ stance: "critical", perspective: "약점" });
    expect(useAppStore.getState().modal.kind).toBe("none");
    expect(useRoleStore.getState().roles).toEqual(roles);
  });

  it("adds presets (a second copy gets a new id) and blocks blank names", async () => {
    render(<RolesDialog />);
    await screen.findByDisplayValue("비판자");
    const preset = screen.getByLabelText("프리셋 추가…");
    fireEvent.change(preset, { target: { value: "moderator" } });
    fireEvent.change(preset, { target: { value: "moderator" } });
    fireEvent.click(screen.getByText("추가"));
    const name = screen.getByDisplayValue("새 역할");
    fireEvent.change(name, { target: { value: " " } });
    const save = screen.getByText("저장") as HTMLButtonElement;
    expect(save.disabled).toBe(true);
    expect(screen.getByText("이름을 적어야 저장할 수 있습니다.")).toBeTruthy();
    fireEvent.change(name, { target: { value: "새 관점" } });
    fireEvent.click(save);
    await waitFor(() => expect(api.saveRoles).toHaveBeenCalled());
    const ids = saved().map((r) => r.id);
    expect(ids).toContain("preset-moderator");
    expect(new Set(ids).size).toBe(ids.length);
    expect(saved().filter((r) => r.name === "사회자")).toHaveLength(3);
  });

  it("keeps the dialog open with an error when saving fails", async () => {
    api.saveRoles.mockRejectedValue("invalid-roles");
    render(<RolesDialog />);
    await screen.findByDisplayValue("비판자");
    fireEvent.click(screen.getByText("저장"));
    expect(await screen.findByText("저장하지 못했습니다.")).toBeTruthy();
    expect(useAppStore.getState().modal.kind).toBe("roles");
  });
});
