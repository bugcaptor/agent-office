// @vitest-environment jsdom
//
// src/renderer/profile/__tests__/ProfileDialog.test.tsx
//
// TDD for `ProfileDialog`.
//
// `../../office/gen/characterFactory` is mocked because `generateSpritePreview`
// composes a real sprite sheet on a `document.createElement("canvas")`
// context that jsdom does not implement (see B's `characterFactory.ts` —
// its own tests back the real function with `@napi-rs/canvas` instead).
// This is a pure orchestration test: does the dialog call the live-preview
// function with the right seed at the right times, and does saving wire up
// store + `tauriApi` in the documented order?
import { act, cleanup, fireEvent, render, screen, waitFor, within } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { useAppStore } from "../../store/appStore";
import type { AgentProfile } from "../../store/types";
import { NAME_WORDS, ROLE_WORDS, PERSONALITY_WORDS } from "../wordlists";

const generateSpritePreview = vi.fn((seed: string) => `data:image/png;base64,PREVIEW-${seed}`);
vi.mock("../../office/gen/characterFactory", () => ({
  generateSpritePreview: (seed: string) => generateSpritePreview(seed),
}));

const createSession = vi.fn().mockResolvedValue({ sessionId: "s1", state: "starting" });
const deletePortrait = vi.fn().mockResolvedValue(undefined);
const deleteSprite = vi.fn().mockResolvedValue(undefined);
vi.mock("../../ipc/tauriApi", () => ({
  tauriApi: {
    createSession: (...args: unknown[]) => createSession(...args),
    deletePortrait: (...args: unknown[]) => deletePortrait(...args),
    deleteSprite: (...args: unknown[]) => deleteSprite(...args),
  },
}));

vi.mock("../../portrait/PortraitEditor", () => ({
  PortraitEditor: () => null,
}));

vi.mock("../../sprite/SpriteEditor", () => ({
  SpriteEditor: () => null,
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
  generateSpritePreview.mockClear();
  createSession.mockClear();
  deletePortrait.mockClear();
  deleteSprite.mockClear();
});

afterEach(() => cleanup());

describe("visibility", () => {
  it("renders nothing when the modal is closed", () => {
    const { container } = render(<ProfileDialog />);
    expect(container.querySelector(".modal-backdrop")).toBeNull();
  });
});

describe("random initial values (profile-create)", () => {
  beforeEach(() => {
    useAppStore.getState().openModal({ kind: "profile-create" });
  });

  it("fills name/role/note from a random draft and renders a live sprite preview", () => {
    const { getByLabelText, getByAltText } = render(<ProfileDialog />);

    const name = getByLabelText("이름") as HTMLInputElement;
    const role = getByLabelText("역할") as HTMLInputElement;
    const note = getByLabelText("메모") as HTMLTextAreaElement;

    expect(NAME_WORDS).toContain(name.value);
    expect(ROLE_WORDS).toContain(role.value);
    expect(PERSONALITY_WORDS.some((p) => note.value === `${p} 성격`)).toBe(true);

    // Live preview: called once on mount with the initial draft's seed.
    expect(generateSpritePreview).toHaveBeenCalledTimes(1);
    const seed = generateSpritePreview.mock.calls[0][0] as string;
    const img = getByAltText("sprite") as HTMLImageElement;
    expect(img.src).toBe(`data:image/png;base64,PREVIEW-${seed}`);
  });

  it("clicking 스프라이트 재생성 changes only the seed and refreshes the preview", () => {
    const { getByLabelText, getByText, getByAltText } = render(<ProfileDialog />);
    const name = getByLabelText("이름") as HTMLInputElement;
    const role = getByLabelText("역할") as HTMLInputElement;
    const note = getByLabelText("메모") as HTMLTextAreaElement;
    const beforeName = name.value;
    const beforeRole = role.value;
    const beforeNote = note.value;
    const firstSeed = generateSpritePreview.mock.calls[0][0] as string;

    fireEvent.click(getByText("스프라이트 재생성"));

    expect(name.value).toBe(beforeName);
    expect(role.value).toBe(beforeRole);
    expect(note.value).toBe(beforeNote);
    expect(generateSpritePreview).toHaveBeenCalledTimes(2);
    const secondSeed = generateSpritePreview.mock.calls[1][0] as string;
    expect(secondSeed).not.toBe(firstSeed);
    const img = getByAltText("sprite") as HTMLImageElement;
    expect(img.src).toBe(`data:image/png;base64,PREVIEW-${secondSeed}`);
  });

  it("clicking 전체 랜덤 regenerates name/role/note/seed together", () => {
    const randomSpy = vi.spyOn(Math, "random");
    // Initial mount's generateDraft() consumes 3 calls (personality, name, role).
    randomSpy
      .mockReturnValueOnce(0)
      .mockReturnValueOnce(0)
      .mockReturnValueOnce(0)
      // regenAll()'s generateDraft() consumes the next 3.
      .mockReturnValueOnce(1 / 20 + 0.001)
      .mockReturnValueOnce(1 / 20 + 0.001)
      .mockReturnValueOnce(1 / 20 + 0.001);

    const { getByLabelText, getByText } = render(<ProfileDialog />);
    const name = getByLabelText("이름") as HTMLInputElement;
    const role = getByLabelText("역할") as HTMLInputElement;
    const note = getByLabelText("메모") as HTMLTextAreaElement;

    expect(name.value).toBe(NAME_WORDS[0]);
    expect(role.value).toBe(ROLE_WORDS[0]);

    fireEvent.click(getByText("전체 랜덤"));

    expect(name.value).toBe(NAME_WORDS[1]);
    expect(role.value).toBe(ROLE_WORDS[1]);
    expect(note.value).toBe(`${PERSONALITY_WORDS[1]} 성격`);
    // A fresh nanoid seed too -> a third preview render.
    expect(generateSpritePreview).toHaveBeenCalledTimes(2);
  });

  it("saving adds the agent (status starting) and starts its session, then closes the dialog", async () => {
    const { getByLabelText, getByText } = render(<ProfileDialog />);
    fireEvent.change(getByLabelText("이름"), { target: { value: "새 에이전트" } });
    fireEvent.change(getByLabelText("역할"), { target: { value: "테스터" } });

    await act(async () => {
      fireEvent.click(getByText("저장"));
      await Promise.resolve();
      await Promise.resolve();
    });

    await waitFor(() => expect(useAppStore.getState().modal.kind).toBe("none"));

    const state = useAppStore.getState();
    expect(state.agentOrder).toHaveLength(1);
    const id = state.agentOrder[0];
    expect(state.agents[id].name).toBe("새 에이전트");
    expect(state.agents[id].role).toBe("테스터");
    expect(state.agents[id].deskIndex).toBe(0);
    expect(state.sessions[id].status).toBe("starting");
    expect(createSession).toHaveBeenCalledWith(id, undefined);
  });

  it("passes the trimmed 시작 폴더 value as createSession's cwd opt (Task 3)", async () => {
    const { getByLabelText, getByText } = render(<ProfileDialog />);
    fireEvent.change(getByLabelText("이름"), { target: { value: "새 에이전트" } });
    fireEvent.change(getByLabelText("시작 폴더"), { target: { value: "  /a/b  " } });

    await act(async () => {
      fireEvent.click(getByText("저장"));
      await Promise.resolve();
      await Promise.resolve();
    });

    await waitFor(() => expect(useAppStore.getState().modal.kind).toBe("none"));

    const state = useAppStore.getState();
    const id = state.agentOrder[0];
    expect(state.agents[id].cwd).toBe("/a/b");
    expect(createSession).toHaveBeenCalledWith(id, { cwd: "/a/b" });
  });

  it("calls createSession without a cwd opt when 시작 폴더 is left blank (Task 3)", async () => {
    const { getByLabelText, getByText } = render(<ProfileDialog />);
    fireEvent.change(getByLabelText("이름"), { target: { value: "새 에이전트" } });

    await act(async () => {
      fireEvent.click(getByText("저장"));
      await Promise.resolve();
      await Promise.resolve();
    });

    await waitFor(() => expect(useAppStore.getState().modal.kind).toBe("none"));

    const state = useAppStore.getState();
    const id = state.agentOrder[0];
    expect(state.agents[id].cwd).toBeUndefined();
    expect(createSession).toHaveBeenCalledWith(id, undefined);
  });

  it("still closes the dialog and marks the session exited when createSession fails (Fix 3)", async () => {
    const warn = vi.spyOn(console, "warn").mockImplementation(() => {});
    createSession.mockRejectedValueOnce(new Error("spawn failed"));

    const { getByLabelText, getByText } = render(<ProfileDialog />);
    fireEvent.change(getByLabelText("이름"), { target: { value: "새 에이전트" } });

    await act(async () => {
      fireEvent.click(getByText("저장"));
      await Promise.resolve();
      await Promise.resolve();
    });

    // The dialog still closes (the profile is saved regardless of PTY start).
    await waitFor(() => expect(useAppStore.getState().modal.kind).toBe("none"));

    const state = useAppStore.getState();
    const id = state.agentOrder[0];
    expect(state.agents[id]).toBeDefined(); // profile persisted
    expect(state.sessions[id].status).toBe("exited"); // session marked exited for retry
    expect(warn).toHaveBeenCalled();

    warn.mockRestore();
  });

  it("초상 프롬프트 복사 버튼이 clipboard에 프롬프트를 쓴다", async () => {
    const writeText = vi.fn().mockResolvedValue(undefined);
    Object.assign(navigator, { clipboard: { writeText } });
    // profile-create 모달을 연다.
    act(() => {
      useAppStore.getState().openModal({ kind: "profile-create" });
    });
    render(<ProfileDialog />);
    fireEvent.click(screen.getByText("초상 프롬프트 복사"));
    await waitFor(() => expect(writeText).toHaveBeenCalledTimes(1));
    const arg = writeText.mock.calls[0][0] as string;
    expect(arg).toContain("240x320");
  });

  it("생성 모드에서는 픽셀아트 업로드/제거 버튼이 없다", () => {
    render(<ProfileDialog />);
    expect(screen.queryByText("픽셀아트 업로드")).toBeNull();
    expect(screen.queryByText("커스텀 제거")).toBeNull();
  });

  it("renders the archetype select with 자동(시드) + 8 options and saves the chosen archetype", async () => {
    render(<ProfileDialog />);
    const select = await screen.findByLabelText("아키타입");
    expect(select).toBeTruthy();
    expect(within(select).getAllByRole("option")).toHaveLength(9);

    fireEvent.change(select, { target: { value: "orc" } });
    await act(async () => {
      fireEvent.click(screen.getByRole("button", { name: "저장" }));
      await Promise.resolve();
      await Promise.resolve();
    });

    await waitFor(() => expect(useAppStore.getState().modal.kind).toBe("none"));

    const agents = useAppStore.getState().agents;
    const created = Object.values(agents)[0];
    expect(created.archetype).toBe("orc");
  });
});

describe("editing mode (profile-edit)", () => {
  beforeEach(() => {
    useAppStore.getState().addAgent(mkProfile());
    useAppStore.getState().openModal({ kind: "profile-edit", agentId: "a1" });
  });

  it("does not show the 전체 랜덤 button in edit mode", () => {
    const { queryByText } = render(<ProfileDialog />);
    expect(queryByText("전체 랜덤")).toBeNull();
  });

  it("loads the existing profile's values instead of a random draft", () => {
    const { getByLabelText } = render(<ProfileDialog />);
    expect((getByLabelText("이름") as HTMLInputElement).value).toBe("Existing");
    expect((getByLabelText("역할") as HTMLInputElement).value).toBe("eng");
    expect((getByLabelText("메모") as HTMLTextAreaElement).value).toBe("existing note");
    expect((getByLabelText("시작 폴더") as HTMLInputElement).value).toBe("");
    expect(generateSpritePreview).toHaveBeenCalledWith("existing-seed");
  });

  it("loads the existing profile's cwd into 시작 폴더 when set (Task 3)", () => {
    useAppStore.getState().updateAgent("a1", { cwd: "/existing/dir" });
    const { getByLabelText } = render(<ProfileDialog />);
    expect((getByLabelText("시작 폴더") as HTMLInputElement).value).toBe("/existing/dir");
  });

  it("saving in edit mode updates cwd without starting a new session (Task 3)", async () => {
    const { getByLabelText, getByText } = render(<ProfileDialog />);
    fireEvent.change(getByLabelText("시작 폴더"), { target: { value: "  /new/dir  " } });

    await act(async () => {
      fireEvent.click(getByText("저장"));
      await Promise.resolve();
    });

    await waitFor(() => expect(useAppStore.getState().modal.kind).toBe("none"));

    expect(useAppStore.getState().agents["a1"].cwd).toBe("/new/dir");
    expect(createSession).not.toHaveBeenCalled();
  });

  // Final-review fix: PortraitEditor's onSave and the 제거 button both call
  // `updateAgent` mid-dialog (portraitUpdatedAt set/cleared), producing a new
  // `editingAgent` object identity while the dialog stays open. The draft-load
  // effect used to depend on `[editingAgent]` (identity), re-firing on every
  // such update and reverting any typed-but-unsaved fields back to store
  // values. It must depend on identity (agent id) only, not on every
  // in-place agent object change.
  it("keeps unsaved draft edits when a mid-dialog updateAgent changes editingAgent's object identity", () => {
    const { getByLabelText } = render(<ProfileDialog />);
    const name = getByLabelText("이름") as HTMLInputElement;

    fireEvent.change(name, { target: { value: "Typed But Unsaved" } });
    expect(name.value).toBe("Typed But Unsaved");

    // Reproduces the identical object-identity change PortraitEditor's
    // onSave / 제거 button trigger, without needing the real editor.
    act(() => {
      useAppStore.getState().updateAgent("a1", { portraitUpdatedAt: Date.now() });
    });

    expect(name.value).toBe("Typed But Unsaved");
  });

  it("saving updates the agent in place without creating a new session, then closes the dialog", async () => {
    const { getByLabelText, getByText } = render(<ProfileDialog />);
    fireEvent.change(getByLabelText("이름"), { target: { value: "Renamed" } });

    await act(async () => {
      fireEvent.click(getByText("저장"));
      await Promise.resolve();
    });

    await waitFor(() => expect(useAppStore.getState().modal.kind).toBe("none"));

    const state = useAppStore.getState();
    expect(state.agentOrder).toEqual(["a1"]);
    expect(state.agents["a1"].name).toBe("Renamed");
    expect(createSession).not.toHaveBeenCalled();
  });
});

describe("픽셀아트 섹션 (edit mode)", () => {
  beforeEach(() => {
    useAppStore.getState().addAgent(mkProfile());
    useAppStore.getState().openModal({ kind: "profile-edit", agentId: "a1" });
  });

  it("픽셀아트 프롬프트 복사는 16x16 프롬프트를 클립보드에 쓴다", async () => {
    const writeText = vi.fn().mockResolvedValue(undefined);
    Object.assign(navigator, { clipboard: { writeText } });
    render(<ProfileDialog />);
    fireEvent.click(screen.getByText("픽셀아트 프롬프트 복사"));
    await waitFor(() => expect(writeText).toHaveBeenCalled());
    expect(writeText.mock.calls[0][0]).toContain("16x16 pixel art");
  });

  it("의뢰 문구 입력이 draft에 반영되고 저장 시 spriteRequest로 저장된다", async () => {
    const { getByLabelText, getByText } = render(<ProfileDialog />);
    fireEvent.change(getByLabelText(/픽셀아트 의뢰 문구/), {
      target: { value: "red cloak wizard" },
    });

    await act(async () => {
      fireEvent.click(getByText("저장"));
      await Promise.resolve();
    });

    await waitFor(() => expect(useAppStore.getState().modal.kind).toBe("none"));
    expect(useAppStore.getState().agents["a1"].spriteRequest).toBe("red cloak wizard");
  });

  it("초상이 없고 커스텀 스프라이트가 있으면 초상 미리보기가 커스텀 스프라이트를 보여준다", () => {
    // 호버 카드와 동일한 폴백 체인: 초상 > 커스텀 스프라이트 프리뷰 > 프로시저럴.
    useAppStore.getState().setSpritePreview("a1", "data:image/png;base64,CUSTOM");

    render(<ProfileDialog />);

    const img = screen.getByAltText("portrait") as HTMLImageElement;
    expect(img.src).toBe("data:image/png;base64,CUSTOM");
  });

  it("커스텀 제거는 deleteSprite + 프리뷰/마커 정리를 수행한다", async () => {
    useAppStore.getState().setSpritePreview("a1", "data:image/png;base64,X");
    useAppStore.getState().updateAgent("a1", { spriteUpdatedAt: 123 });

    render(<ProfileDialog />);
    fireEvent.click(screen.getByText("커스텀 제거"));

    await waitFor(() => expect(deleteSprite).toHaveBeenCalledWith("a1"));
    expect(useAppStore.getState().spritePreviews["a1"]).toBeUndefined();
    expect(useAppStore.getState().agents["a1"].spriteUpdatedAt).toBeUndefined();
  });
});

describe("cancel", () => {
  it("clicking 취소 closes the dialog without saving", () => {
    useAppStore.getState().openModal({ kind: "profile-create" });
    const { getByText } = render(<ProfileDialog />);

    fireEvent.click(getByText("취소"));

    expect(useAppStore.getState().modal.kind).toBe("none");
    expect(useAppStore.getState().agentOrder).toHaveLength(0);
    expect(createSession).not.toHaveBeenCalled();
  });
});

// Reviewer fix: nested PortraitEditor click bubbling closed the whole dialog
// (backdrop used onClick={closeModal}, so every synthetic click inside the
// nested editor bubbled up to it). Mirrors TerminalOverlay's
// mousedown+target-guard pattern (commit 7986f3d).
describe("backdrop mousedown close (target-guard fix)", () => {
  it("mousedown directly on the backdrop closes the dialog", () => {
    useAppStore.getState().openModal({ kind: "profile-create" });
    const { container } = render(<ProfileDialog />);

    const backdrop = container.querySelector(".modal-backdrop") as HTMLElement;
    fireEvent.mouseDown(backdrop);

    expect(useAppStore.getState().modal.kind).toBe("none");
  });

  it("mousedown inside the panel does not close the dialog", () => {
    useAppStore.getState().openModal({ kind: "profile-create" });
    const { container } = render(<ProfileDialog />);

    const panel = container.querySelector(".pixel-panel") as HTMLElement;
    fireEvent.mouseDown(panel);

    expect(useAppStore.getState().modal.kind).toBe("profile-create");
  });
});

describe("modal kind 가드 (Task A2 회귀)", () => {
  it("confirm-delete 모달에서는 ProfileDialog가 아무것도 렌더하지 않는다", () => {
    const s = useAppStore.getState();
    s.addAgent(mkProfile({ id: "a1" }));
    s.openModal({ kind: "confirm-delete", agentId: "a1" });

    const { container } = render(<ProfileDialog />);

    expect(container.firstChild).toBeNull();
  });
});
