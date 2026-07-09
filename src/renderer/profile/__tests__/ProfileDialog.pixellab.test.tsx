// @vitest-environment jsdom
//
// src/renderer/profile/__tests__/ProfileDialog.pixellab.test.tsx
//
// "PixelLab로 생성" 버튼 오케스트레이션: 생성 중 disabled,
// 성공 시 SpriteEditor가 initialImage(data URL)로 열림 + 비용 캡션,
// 실패 시 코드별 한국어 캡션. 네트워크·캔버스 없음 — tauriApi와
// SpriteEditor는 mock.
import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { useAppStore } from "../../store/appStore";
import type { AgentProfile } from "../../store/types";

const generateSpritePreview = vi.fn((seed: string) => `data:image/png;base64,PREVIEW-${seed}`);
vi.mock("../../office/gen/characterFactory", () => ({
  generateSpritePreview: (seed: string) => generateSpritePreview(seed),
}));

const generateSpriteImage = vi.fn();
vi.mock("../../ipc/tauriApi", () => ({
  tauriApi: {
    generateSpriteImage: (...args: unknown[]) => generateSpriteImage(...args),
    deletePortrait: vi.fn().mockResolvedValue(undefined),
    deleteSprite: vi.fn().mockResolvedValue(undefined),
    listAvailableShells: vi.fn().mockResolvedValue([]),
  },
}));

vi.mock("../../portrait/PortraitEditor", () => ({
  PortraitEditor: () => null,
}));

// SpriteEditor는 받은 props를 기록만 한다.
const spriteEditorProps = vi.fn();
vi.mock("../../sprite/SpriteEditor", () => ({
  SpriteEditor: (props: Record<string, unknown>) => {
    spriteEditorProps(props);
    return <div data-testid="sprite-editor" />;
  },
}));

const { ProfileDialog, pixellabErrorCaption } = await import("../ProfileDialog");

function mkProfile(overrides: Partial<AgentProfile> = {}): AgentProfile {
  return {
    id: "a1",
    name: "Existing",
    role: "eng",
    note: "",
    seed: "existing-seed",
    createdAt: Date.now(),
    deskIndex: 0,
    ...overrides,
  };
}

const initialState = useAppStore.getState();

beforeEach(() => {
  useAppStore.setState(initialState, true);
  generateSpriteImage.mockReset();
  spriteEditorProps.mockClear();
});
afterEach(() => cleanup());

/** 편집 모달을 연 상태로 렌더 (ProfileDialog.test.tsx의 편집 모드 패턴 그대로). */
function renderEdit() {
  useAppStore.getState().addAgent(mkProfile());
  useAppStore.getState().openModal({ kind: "profile-edit", agentId: "a1" });
  return render(<ProfileDialog />);
}

describe("PixelLab 생성 버튼", () => {
  it("편집 모드에서 렌더되고, 클릭하면 설명 프롬프트로 IPC를 호출하며 생성 중 disabled가 된다", async () => {
    let resolve!: (v: unknown) => void;
    generateSpriteImage.mockReturnValue(new Promise((r) => (resolve = r)));
    renderEdit();
    const btn = screen.getByText("PixelLab로 생성") as HTMLButtonElement;
    fireEvent.click(btn);
    expect(generateSpriteImage).toHaveBeenCalledTimes(1);
    const desc = generateSpriteImage.mock.calls[0][0] as string;
    expect(desc).toContain("Existing"); // 이름이 프롬프트에 반영
    expect(desc).not.toContain("16x16"); // PixelLab 전용 빌더 사용
    await waitFor(() =>
      expect((screen.getByText("생성 중…") as HTMLButtonElement).disabled).toBe(true),
    );
    await act(async () => resolve({ pngBase64: "GEN", costUsd: 0.02 }));
  });

  it("성공하면 SpriteEditor가 data URL initialImage로 열리고 비용 캡션이 보인다", async () => {
    generateSpriteImage.mockResolvedValue({ pngBase64: "GEN", costUsd: 0.02 });
    renderEdit();
    fireEvent.click(screen.getByText("PixelLab로 생성"));
    await waitFor(() => expect(screen.getByTestId("sprite-editor")).toBeTruthy());
    expect(spriteEditorProps).toHaveBeenCalledWith(
      expect.objectContaining({ initialImage: "data:image/png;base64,GEN" }),
    );
    expect(screen.getByText("생성 완료 · $0.02")).toBeTruthy();
  });

  it("costUsd가 없으면 '생성 완료'만 표시한다", async () => {
    generateSpriteImage.mockResolvedValue({ pngBase64: "GEN" });
    renderEdit();
    fireEvent.click(screen.getByText("PixelLab로 생성"));
    await waitFor(() => expect(screen.getByText("생성 완료")).toBeTruthy());
  });

  it("실패하면 코드별 캡션을 표시하고 버튼이 복구된다", async () => {
    generateSpriteImage.mockRejectedValue("missing_api_key: PIXELLAB_API_KEY is not set");
    renderEdit();
    fireEvent.click(screen.getByText("PixelLab로 생성"));
    await waitFor(() =>
      expect(
        screen.getByText("PIXELLAB_API_KEY 환경변수를 설정한 뒤 앱을 재시작하세요."),
      ).toBeTruthy(),
    );
    expect((screen.getByText("PixelLab로 생성") as HTMLButtonElement).disabled).toBe(false);
    expect(screen.queryByTestId("sprite-editor")).toBeNull();
  });

  // 리뷰 Finding 1: ProfileDialog는 App에서 상시 마운트(닫힘 = return null)라
  // unmount 가드가 아니라 편집 세션 토큰으로 늦은 응답을 무효화해야 한다.
  it("다이얼로그를 닫은 뒤 도착한 늦은 응답은 무시되고, 다른 에이전트로 재오픈해도 새지 않는다", async () => {
    let resolve!: (v: unknown) => void;
    generateSpriteImage.mockReturnValue(new Promise((r) => (resolve = r)));
    renderEdit();
    fireEvent.click(screen.getByText("PixelLab로 생성"));
    act(() => {
      useAppStore.getState().closeModal();
    });
    await act(async () => resolve({ pngBase64: "STALE", costUsd: 0.02 }));
    expect(screen.queryByTestId("sprite-editor")).toBeNull();
    expect(screen.queryByText("생성 완료 · $0.02")).toBeNull();
    // 다른 에이전트의 편집 모달로 재오픈: 스테일 이미지/캡션/busy가 넘어오면 안 된다.
    act(() => {
      useAppStore.getState().addAgent(mkProfile({ id: "a2", name: "Other", seed: "other-seed" }));
      useAppStore.getState().openModal({ kind: "profile-edit", agentId: "a2" });
    });
    expect(screen.queryByTestId("sprite-editor")).toBeNull();
    expect(spriteEditorProps).not.toHaveBeenCalled();
    expect(screen.queryByText("생성 완료 · $0.02")).toBeNull();
    expect((screen.getByText("PixelLab로 생성") as HTMLButtonElement).disabled).toBe(false);
  });

  // 리뷰 Finding 2 (같은 뿌리): 캡션이 편집 세션을 넘어 잔존하면 안 된다.
  it("성공 캡션은 닫고 다시 열면 사라진다", async () => {
    generateSpriteImage.mockResolvedValue({ pngBase64: "GEN", costUsd: 0.02 });
    renderEdit();
    fireEvent.click(screen.getByText("PixelLab로 생성"));
    await waitFor(() => expect(screen.getByText("생성 완료 · $0.02")).toBeTruthy());
    act(() => {
      useAppStore.getState().closeModal();
    });
    act(() => {
      useAppStore.getState().openModal({ kind: "profile-edit", agentId: "a1" });
    });
    expect(screen.queryByText("생성 완료 · $0.02")).toBeNull();
  });

  it("생성 모드(profile-create)에서는 버튼이 없다", () => {
    useAppStore.getState().openModal({ kind: "profile-create" });
    render(<ProfileDialog />);
    expect(screen.queryByText("PixelLab로 생성")).toBeNull();
  });
});

describe("pixellabErrorCaption", () => {
  it.each([
    ["missing_api_key: x", "PIXELLAB_API_KEY 환경변수를 설정한 뒤 앱을 재시작하세요."],
    ["invalid_api_key: x", "PixelLab API 키가 유효하지 않습니다."],
    ["insufficient_credits: x", "PixelLab 크레딧이 부족합니다."],
    ["rate_limited: x", "요청이 몰려 있습니다. 잠시 후 다시 시도하세요."],
  ])("%s → %s", (input, expected) => {
    expect(pixellabErrorCaption(input)).toBe(expected);
  });

  it("모르는 코드는 원문을 포함한 일반 문구", () => {
    expect(pixellabErrorCaption("network: HTTP 500")).toBe(
      "생성에 실패했습니다: network: HTTP 500",
    );
  });
});
