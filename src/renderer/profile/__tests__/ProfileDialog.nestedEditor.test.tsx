// @vitest-environment jsdom
//
// src/renderer/profile/__tests__/ProfileDialog.nestedEditor.test.tsx
//
// Reviewer fix (nested modal click bubbling): PortraitEditor renders nested
// inside ProfileDialog's backdrop. Before the fix, ProfileDialog's backdrop
// used `onClick={closeModal}` — every React synthetic `click` inside the
// nested editor (레트로 필터 checkbox, 취소 button, file input) bubbled up the
// React tree and closed the whole dialog. This file renders the REAL
// PortraitEditor (unlike ProfileDialog.test.tsx, which stubs it out for its
// unrelated orchestration tests) so we can exercise that bubbling path.
// jsdom-safe: PortraitEditor's `redraw()` early-returns without a loaded
// image, so no file needs to actually be selected for these cases.
import { cleanup, fireEvent, render, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { useAppStore } from "../../store/appStore";
import type { AgentProfile } from "../../store/types";

vi.mock("../../office/gen/characterFactory", () => ({
  generateSpritePreview: (seed: string) => `data:image/png;base64,PREVIEW-${seed}`,
}));

const createSession = vi.fn().mockResolvedValue({ sessionId: "s1", state: "starting" });
const deletePortrait = vi.fn().mockResolvedValue(undefined);
const savePortrait = vi.fn().mockResolvedValue(undefined);
vi.mock("../../ipc/tauriApi", () => ({
  tauriApi: {
    createSession: (...args: unknown[]) => createSession(...args),
    deletePortrait: (...args: unknown[]) => deletePortrait(...args),
    savePortrait: (...args: unknown[]) => savePortrait(...args),
    listAvailableShells: vi.fn().mockResolvedValue([]),
  },
}));

const { ProfileDialog } = await import("../ProfileDialog");

function mkProfile(overrides: Partial<AgentProfile> = {}): AgentProfile {
  return {
    id: "a1",
    name: "Existing",
    role: "eng",
    note: "existing note",
    seed: "existing-seed",
    createdAt: Date.now(),
    deskIndex: 0,
    ...overrides,
  };
}

const initialState = useAppStore.getState();

beforeEach(() => {
  useAppStore.setState(initialState, true);
  useAppStore.getState().addAgent(mkProfile());
  useAppStore.getState().openModal({ kind: "profile-edit", agentId: "a1" });
  createSession.mockClear();
  deletePortrait.mockClear();
  savePortrait.mockClear();
});

afterEach(() => cleanup());

describe("nested PortraitEditor clicks do not bubble to ProfileDialog's backdrop", () => {
  it("clicking the 레트로 필터 checkbox inside the editor does not close ProfileDialog", () => {
    const { getByText, getByLabelText, queryByText } = render(<ProfileDialog />);

    fireEvent.click(getByText("이미지 업로드"));
    const checkbox = getByLabelText("레트로 픽셀 필터");
    fireEvent.click(checkbox);

    expect(useAppStore.getState().modal.kind).toBe("profile-edit");
    expect(queryByText("에이전트 편집")).not.toBeNull();
  });

  it("clicking 취소 inside the editor closes only the editor, not ProfileDialog", () => {
    const { getByText, container } = render(<ProfileDialog />);

    fireEvent.click(getByText("이미지 업로드"));
    const editorPanel = container.querySelector(".portrait-editor") as HTMLElement;
    expect(editorPanel).not.toBeNull();

    fireEvent.click(within(editorPanel).getByText("취소"));

    expect(useAppStore.getState().modal.kind).toBe("profile-edit");
    expect(container.querySelector(".portrait-editor")).toBeNull();
  });
});
