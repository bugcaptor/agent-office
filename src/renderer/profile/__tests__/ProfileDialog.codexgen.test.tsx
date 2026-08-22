// @vitest-environment jsdom
//
// src/renderer/profile/__tests__/ProfileDialog.codexgen.test.tsx
//
// "외형" 섹션의 모드 탭(직접 만들기 / Codex로 생성)과 Codex 생성
// 오케스트레이션: 탭이 한 번에 한 쪽만 보여 주는지, 생성 중 disabled,
// 성공 시 해당 크롭 편집기가 initialImage(data URL)로 열리는지,
// 실패 시 코드별 한국어 캡션, 늦은 응답의 편집 세션 토큰 가드.
// 프로세스·캔버스 없음 — tauriApi와 두 편집기는 mock.
import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { useAppStore } from "../../store/appStore";
import type { AgentProfile } from "../../store/types";

const generateSpritePreview = vi.fn((seed: string) => `data:image/png;base64,PREVIEW-${seed}`);
vi.mock("../../office/gen/characterFactory", () => ({
  generateSpritePreview: (seed: string) => generateSpritePreview(seed),
}));

const generateCodexImage = vi.fn();
const codexImageStatus = vi.fn();
vi.mock("../../ipc/tauriApi", () => ({
  tauriApi: {
    generateCodexImage: (...args: unknown[]) => generateCodexImage(...args),
    codexImageStatus: (...args: unknown[]) => codexImageStatus(...args),
    deletePortrait: vi.fn().mockResolvedValue(undefined),
    deleteSprite: vi.fn().mockResolvedValue(undefined),
    listAvailableShells: vi.fn().mockResolvedValue([]),
  },
}));

// 두 편집기는 받은 props를 기록만 한다.
const portraitEditorProps = vi.fn();
vi.mock("../../portrait/PortraitEditor", () => ({
  PortraitEditor: (props: Record<string, unknown>) => {
    portraitEditorProps(props);
    return <div data-testid="portrait-editor" />;
  },
}));
const spriteEditorProps = vi.fn();
vi.mock("../../sprite/SpriteEditor", () => ({
  SpriteEditor: (props: Record<string, unknown>) => {
    spriteEditorProps(props);
    return <div data-testid="sprite-editor" />;
  },
}));

const { ProfileDialog } = await import("../ProfileDialog");
const { codexGenErrorCaption } = await import("../../portrait/CodexGenPanel");

function mkProfile(overrides: Partial<AgentProfile> = {}): AgentProfile {
  return {
    id: "a1",
    name: "Existing",
    role: "eng",
    seed: "existing-seed",
    createdAt: Date.now(),
    deskIndex: 0,
    ...overrides,
  };
}

const initialState = useAppStore.getState();

beforeEach(() => {
  useAppStore.setState(initialState, true);
  generateCodexImage.mockReset();
  codexImageStatus.mockReset();
  codexImageStatus.mockResolvedValue({ available: true, version: "codex-cli 0.149.0" });
  spriteEditorProps.mockClear();
  portraitEditorProps.mockClear();
});
afterEach(() => cleanup());

/** 편집 모달을 연 상태로 렌더 (ProfileDialog.test.tsx의 편집 모드 패턴 그대로). */
function renderEdit() {
  useAppStore.getState().addAgent(mkProfile());
  useAppStore.getState().openModal({ kind: "profile-edit", agentId: "a1" });
  return render(<ProfileDialog />);
}

/** Codex 탭으로 전환하고 탐지가 끝나기를 기다린다. */
async function switchToCodexTab() {
  fireEvent.click(screen.getByText("Codex로 생성"));
  await waitFor(() => expect(screen.getByText(/codex CLI 사용 가능/)).toBeTruthy());
}

describe("외형 모드 탭", () => {
  it("기본은 '직접 만들기'이고 Codex 생성 버튼은 보이지 않는다", () => {
    renderEdit();
    expect(screen.getByText("초상 프롬프트 복사")).toBeTruthy();
    expect(screen.getByText("스프라이트 프롬프트 복사")).toBeTruthy();
    expect(screen.queryByText("초상 생성")).toBeNull();
    expect(screen.queryByText("스프라이트 생성")).toBeNull();
  });

  it("'Codex로 생성' 탭으로 바꾸면 직접 만들기 항목이 사라지고 생성 버튼이 나온다", async () => {
    renderEdit();
    await switchToCodexTab();
    expect(screen.queryByText("초상 프롬프트 복사")).toBeNull();
    expect(screen.queryByText("스프라이트 프롬프트 복사")).toBeNull();
    expect(screen.getByText("초상 생성")).toBeTruthy();
    expect(screen.getByText("스프라이트 생성")).toBeTruthy();
  });

  it("스프라이트 재생성 같은 공통 항목은 두 모드 모두에서 보인다", async () => {
    renderEdit();
    expect(screen.getByText("스프라이트 재생성")).toBeTruthy();
    await switchToCodexTab();
    expect(screen.getByText("스프라이트 재생성")).toBeTruthy();
  });

  it("codex가 없으면 생성 버튼이 비활성이고 설치 안내가 뜬다", async () => {
    codexImageStatus.mockResolvedValue({ available: false });
    renderEdit();
    fireEvent.click(screen.getByText("Codex로 생성"));
    await waitFor(() => expect(screen.getByText(/codex CLI를 찾을 수 없습니다/)).toBeTruthy());
    expect((screen.getByText("초상 생성") as HTMLButtonElement).disabled).toBe(true);
    expect((screen.getByText("스프라이트 생성") as HTMLButtonElement).disabled).toBe(true);
  });

  it("생성 모드(profile-create)에서는 생성 버튼이 비활성 + 저장 안내", async () => {
    useAppStore.getState().openModal({ kind: "profile-create" });
    render(<ProfileDialog />);
    await switchToCodexTab();
    expect(screen.getByText("저장한 뒤 편집에서 생성할 수 있습니다.")).toBeTruthy();
    expect((screen.getByText("초상 생성") as HTMLButtonElement).disabled).toBe(true);
  });
});

describe("Codex 생성 버튼", () => {
  it("스프라이트 생성은 codex 프롬프트로 IPC를 호출하고 생성 중 두 버튼 모두 disabled", async () => {
    let resolve!: (v: unknown) => void;
    generateCodexImage.mockReturnValue(new Promise((r) => (resolve = r)));
    renderEdit();
    await switchToCodexTab();
    fireEvent.click(screen.getByText("스프라이트 생성"));
    expect(generateCodexImage).toHaveBeenCalledTimes(1);
    const prompt = generateCodexImage.mock.calls[0][0] as string;
    expect(prompt).toContain("Existing");
    expect(prompt).toContain("1024x1024"); // codex 전용 규격 줄
    expect(prompt).toContain("transparent background");
    await waitFor(() =>
      expect((screen.getByText("스프라이트 생성 중…") as HTMLButtonElement).disabled).toBe(true),
    );
    expect((screen.getByText("초상 생성") as HTMLButtonElement).disabled).toBe(true);
    await act(async () => resolve({ pngBase64: "GEN" }));
  });

  it("성공하면 SpriteEditor가 data URL initialImage로 열린다", async () => {
    generateCodexImage.mockResolvedValue({ pngBase64: "GEN" });
    renderEdit();
    await switchToCodexTab();
    fireEvent.click(screen.getByText("스프라이트 생성"));
    await waitFor(() => expect(screen.getByTestId("sprite-editor")).toBeTruthy());
    expect(spriteEditorProps).toHaveBeenCalledWith(
      expect.objectContaining({ initialImage: "data:image/png;base64,GEN" }),
    );
    expect(screen.getByText("생성 완료 — 편집기에서 확인하고 저장하세요.")).toBeTruthy();
  });

  it("초상 생성은 초상 프롬프트를 쓰고 PortraitEditor를 프리로드해 연다", async () => {
    generateCodexImage.mockResolvedValue({ pngBase64: "PGEN" });
    renderEdit();
    await switchToCodexTab();
    fireEvent.click(screen.getByText("초상 생성"));
    await waitFor(() => expect(screen.getByTestId("portrait-editor")).toBeTruthy());
    const prompt = generateCodexImage.mock.calls[0][0] as string;
    expect(prompt).toContain("1024x1536");
    expect(portraitEditorProps).toHaveBeenCalledWith(
      expect.objectContaining({ initialImage: "data:image/png;base64,PGEN" }),
    );
    expect(screen.queryByTestId("sprite-editor")).toBeNull();
  });

  it("실패하면 코드별 캡션을 표시하고 버튼이 복구된다", async () => {
    generateCodexImage.mockRejectedValue("codex-not-found: codex CLI not found");
    renderEdit();
    await switchToCodexTab();
    fireEvent.click(screen.getByText("스프라이트 생성"));
    await waitFor(() =>
      expect(
        screen.getByText("codex CLI를 찾을 수 없습니다. 설치한 뒤 다시 시도하세요."),
      ).toBeTruthy(),
    );
    expect((screen.getByText("스프라이트 생성") as HTMLButtonElement).disabled).toBe(false);
    expect(screen.queryByTestId("sprite-editor")).toBeNull();
  });

  // ProfileDialog는 App에서 상시 마운트(닫힘 = return null)라 unmount 가드가
  // 아니라 편집 세션 토큰으로 늦은 응답을 무효화해야 한다(이전 생성 버튼 리뷰 Finding 1).
  it("다이얼로그를 닫은 뒤 도착한 늦은 응답은 무시되고, 다른 에이전트로 재오픈해도 새지 않는다", async () => {
    let resolve!: (v: unknown) => void;
    generateCodexImage.mockReturnValue(new Promise((r) => (resolve = r)));
    renderEdit();
    await switchToCodexTab();
    fireEvent.click(screen.getByText("스프라이트 생성"));
    act(() => {
      useAppStore.getState().closeModal();
    });
    await act(async () => resolve({ pngBase64: "STALE" }));
    expect(screen.queryByTestId("sprite-editor")).toBeNull();

    // 다른 에이전트의 편집 모달로 재오픈: 스테일 이미지/캡션/busy가 넘어오면 안 된다.
    act(() => {
      useAppStore.getState().addAgent(mkProfile({ id: "a2", name: "Other", seed: "other-seed" }));
      useAppStore.getState().openModal({ kind: "profile-edit", agentId: "a2" });
    });
    expect(screen.queryByTestId("sprite-editor")).toBeNull();
    expect(spriteEditorProps).not.toHaveBeenCalled();
    await switchToCodexTab();
    expect(screen.queryByText("생성 완료 — 편집기에서 확인하고 저장하세요.")).toBeNull();
    expect((screen.getByText("스프라이트 생성") as HTMLButtonElement).disabled).toBe(false);
  });

  // 같은 뿌리(리뷰 Finding 2): 캡션이 편집 세션을 넘어 잔존하면 안 된다.
  it("성공 캡션은 닫고 다시 열면 사라진다", async () => {
    generateCodexImage.mockResolvedValue({ pngBase64: "GEN" });
    renderEdit();
    await switchToCodexTab();
    fireEvent.click(screen.getByText("스프라이트 생성"));
    await waitFor(() =>
      expect(screen.getByText("생성 완료 — 편집기에서 확인하고 저장하세요.")).toBeTruthy(),
    );
    act(() => {
      useAppStore.getState().closeModal();
    });
    act(() => {
      useAppStore.getState().openModal({ kind: "profile-edit", agentId: "a1" });
    });
    await switchToCodexTab();
    expect(screen.queryByText("생성 완료 — 편집기에서 확인하고 저장하세요.")).toBeNull();
  });
});

describe("codexGenErrorCaption", () => {
  it.each([
    ["codex-not-found: codex CLI not found", "codex CLI를 찾을 수 없습니다. 설치한 뒤 다시 시도하세요."],
    ["timeout: codex image generation timed out", "생성이 시간 안에 끝나지 않았습니다. 다시 시도하세요."],
  ])("%s → %s", (input, expected) => {
    expect(codexGenErrorCaption(input)).toBe(expected);
  });

  it("no_output은 codex가 남긴 사유를 덧붙인다", () => {
    expect(codexGenErrorCaption("no_output: 이미지를 만들지 못했습니다")).toBe(
      "codex가 이미지를 저장하지 않았습니다. 다시 시도하세요: 이미지를 만들지 못했습니다",
    );
  });

  it("모르는 코드는 원문을 포함한 일반 문구", () => {
    expect(codexGenErrorCaption("failed: codex exited 1: boom")).toBe(
      "생성에 실패했습니다: failed: codex exited 1: boom",
    );
  });
});
