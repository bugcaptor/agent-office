// @vitest-environment jsdom
//
// src/renderer/settings/__tests__/SettingsDialogSummaryCommand.test.tsx
//
// 요약기 provider별 **실행 명령** 오버라이드와, provider를 가리지 않는 응답
// 테스트(kbm #2nv):
//  - 실행 명령은 CLI provider에만 있다. 비우면 기본 이름을 부르고, 채우면 그
//    이름 대신 그것을 부른다(별개 계정으로 붙는 `claude-t` 같은 래퍼).
//  - "요약 테스트"는 이제 OpenRouter 전용이 아니다 — 실행 명령이나 모델을
//    고쳐 놓고 그게 실제로 도는지 확인할 자리는 어느 provider에나 필요하다.
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { AppSettings, TtsStatus } from "@shared/types";

const ttsKeyStatus = vi.fn<() => Promise<TtsStatus>>();
const ttsSetKeys = vi.fn<(e?: string, a?: string, o?: string) => Promise<TtsStatus>>();
const summarizeText = vi.fn<(...args: unknown[]) => Promise<string>>();
const listProviderModels = vi.fn<(p: string) => Promise<string[]>>();
const setAppSettings = vi.fn<(s: unknown) => Promise<void>>(() => Promise.resolve());

vi.mock("../../ipc/tauriApi", () => ({
  tauriApi: {
    ttsKeyStatus: () => ttsKeyStatus(),
    ttsSetKeys: (e?: string, a?: string, o?: string) => ttsSetKeys(e, a, o),
    summarizeText: (...args: unknown[]) => summarizeText(...args),
    listProviderModels: (p: string) => listProviderModels(p),
    setAppSettings: (s: unknown) => setAppSettings(s),
  },
}));

import { useAppStore } from "../../store/appStore";
import { SettingsDialog } from "../SettingsDialog";
import { resetModelCatalogCache } from "../modelCatalog";

const initialState = useAppStore.getState();

const STATUS: TtsStatus = {
  elevenlabsSet: false,
  anthropicSet: false,
  elevenlabsFromEnv: false,
  anthropicFromEnv: false,
  openrouterSet: false,
  openrouterFromEnv: false,
  claudeCliAvailable: true,
  effectiveRewriteVia: "claude-cli",
};

function hydrate(patch: Partial<AppSettings> = {}) {
  useAppStore
    .getState()
    .hydrateSettings({ ...useAppStore.getState().appSettings, ...patch }, false);
  useAppStore.getState().openModal({ kind: "settings" });
}

beforeEach(() => {
  useAppStore.setState(initialState, true);
  resetModelCatalogCache();
  ttsKeyStatus.mockReset().mockResolvedValue(STATUS);
  ttsSetKeys.mockReset().mockResolvedValue(STATUS);
  summarizeText.mockReset().mockResolvedValue("한 문장 요약입니다.");
  listProviderModels.mockReset().mockResolvedValue([]);
  setAppSettings.mockClear();
});

afterEach(() => cleanup());

describe("SettingsDialog · 요약기 실행 명령", () => {
  it("빈 칸은 기본 이름을 안내하고, 고친 값은 그 provider 칸에만 저장된다", () => {
    hydrate({ summaryProvider: "claude" });
    render(<SettingsDialog />);

    const command = screen.getByPlaceholderText("claude") as HTMLInputElement;
    expect(command.value).toBe("");

    fireEvent.change(command, { target: { value: "claude-t" } });
    const models = useAppStore.getState().appSettings.summaryModels;
    expect(models.claude).toEqual({ light: "", heavy: "", command: "claude-t" });
    // 모델 칸은 건드리지 않고, 다른 provider도 그대로다.
    expect(models.codex).toEqual({ light: "", heavy: "", command: "" });
  });

  it("provider를 바꾸면 그 provider의 기본 명령을 안내한다", () => {
    hydrate({ summaryProvider: "codex" });
    render(<SettingsDialog />);

    expect(screen.getByPlaceholderText("codex")).toBeTruthy();
    expect(screen.queryByPlaceholderText("claude")).toBeNull();
  });

  // OpenRouter는 서브프로세스가 아니라 HTTP라 부를 명령 자체가 없다.
  it("OpenRouter에는 실행 명령 칸이 없다", () => {
    hydrate({ summaryProvider: "openrouter" });
    render(<SettingsDialog />);

    expect(screen.queryByPlaceholderText("openrouter")).toBeNull();
    expect(screen.queryByPlaceholderText("claude")).toBeNull();
  });
});

describe("SettingsDialog · 요약 응답 테스트", () => {
  it("지금 고른 provider로 실제 summarizeText 경로를 탄다", async () => {
    hydrate({ summaryProvider: "claude" });
    render(<SettingsDialog />);

    fireEvent.click(screen.getByText("요약 테스트"));
    await waitFor(() => expect(summarizeText).toHaveBeenCalled());
    const [provider, instruction, text, purpose] = summarizeText.mock.calls[0];
    expect(provider).toBe("claude");
    expect(String(instruction).length).toBeGreaterThan(0);
    expect(String(text).length).toBeGreaterThan(0);
    expect(purpose).toBe("label");
    await screen.findByText("요약: 한 문장 요약입니다.");
  });

  // 실행 명령을 잘못 적었을 때 가장 흔한 실패다 — 원문(`claude-not-found`)만
  // 보여주면 어디를 고쳐야 하는지 알 수 없다.
  it("실행 명령을 못 찾은 실패를 사람이 읽는 문장으로 바꿔 준다", async () => {
    hydrate({ summaryProvider: "claude" });
    summarizeText.mockRejectedValue("claude-not-found");
    render(<SettingsDialog />);

    fireEvent.click(screen.getByText("요약 테스트"));
    await screen.findByText(/요약 실패: 실행 명령을 찾을 수 없습니다/);
  });

  it("요약 기능이 꺼져 있으면 그렇다고 말해 준다", async () => {
    hydrate({ summaryProvider: "codex" });
    summarizeText.mockRejectedValue("summarizer-disabled");
    render(<SettingsDialog />);

    fireEvent.click(screen.getByText("요약 테스트"));
    await screen.findByText(/요약 실패: 요약 기능이 꺼져 있습니다/);
  });
});
