// @vitest-environment jsdom
//
// src/renderer/settings/__tests__/SettingsDialogAnthropic.test.tsx
//
// 요약기 provider가 Claude일 때만 뜨는 Anthropic API 키 칸:
//  - 정리 G-C23에서 TTS 설정과 함께 키 입력 UI가 사라졌지만, 저장된 키는
//    Claude 모델 카탈로그(Anthropic `/v1/models`)가 계속 쓴다 — 그 키를 넣고
//    지우는 자리다.
//  - 저장소는 공용 API 키 저장소(`setApiKeys`의 첫째 칸)이고, OpenRouter
//    칸은 건드리지 않는다.
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { AppSettings, ApiKeyStatus } from "@shared/types";

const apiKeyStatus = vi.fn<() => Promise<ApiKeyStatus>>();
const setApiKeys = vi.fn<(a?: string, o?: string) => Promise<ApiKeyStatus>>();
const summarizeText = vi.fn<(...args: unknown[]) => Promise<string>>();
const listProviderModels = vi.fn<(p: string) => Promise<string[]>>();
const setAppSettings = vi.fn<(s: unknown) => Promise<void>>(() => Promise.resolve());

vi.mock("../../ipc/tauriApi", () => ({
  tauriApi: {
    apiKeyStatus: () => apiKeyStatus(),
    setApiKeys: (a?: string, o?: string) => setApiKeys(a, o),
    summarizeText: (...args: unknown[]) => summarizeText(...args),
    listProviderModels: (p: string) => listProviderModels(p),
    setAppSettings: (s: unknown) => setAppSettings(s),
  },
}));

import { useAppStore } from "../../store/appStore";
import { SettingsDialog } from "../SettingsDialog";
import { resetModelCatalogCache } from "../modelCatalog";

const initialState = useAppStore.getState();

const STATUS: ApiKeyStatus = {
  anthropicSet: false,
  anthropicFromEnv: false,
  openrouterSet: false,
  openrouterFromEnv: false,
  opencodeSet: false,
  opencodeFromEnv: false,
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
  apiKeyStatus.mockReset().mockResolvedValue(STATUS);
  setApiKeys.mockReset().mockResolvedValue({ ...STATUS, anthropicSet: true });
  summarizeText.mockReset().mockResolvedValue("한 문장 요약입니다.");
  listProviderModels.mockReset().mockResolvedValue([]);
  setAppSettings.mockClear();
});

afterEach(() => cleanup());

describe("SettingsDialog · Anthropic API 키", () => {
  it("Claude가 아닌 요약기에서는 Anthropic 키 칸이 없다", async () => {
    hydrate({ summaryProvider: "openrouter" });
    render(<SettingsDialog />);
    // OpenRouter 칸은 떠도 Anthropic 칸은 뜨지 않는다.
    await screen.findByText(/OpenRouter 키 없음/);
    expect(screen.queryByPlaceholderText("sk-ant-…")).toBeNull();
    expect(screen.queryByText(/Anthropic 키/)).toBeNull();
  });

  it("키 상태를 마스킹된 문장으로 보여주고, 저장은 Anthropic 칸만 채운다", async () => {
    hydrate({ summaryProvider: "claude" });
    render(<SettingsDialog />);
    await screen.findByText(/Anthropic 키 없음/);
    // Claude에는 OpenRouter 칸이 없다.
    expect(screen.queryByPlaceholderText("sk-or-…")).toBeNull();

    const input = screen.getByPlaceholderText("sk-ant-…") as HTMLInputElement;
    expect(input.type).toBe("password");
    // 빈 입력으로는 저장할 수 없다(실수로 기존 키를 건드리지 않게).
    const save = screen.getByText("키 저장") as HTMLButtonElement;
    expect(save.disabled).toBe(true);

    fireEvent.change(input, { target: { value: "sk-ant-abc" } });
    fireEvent.click(save);
    // 뒤 칸(OpenRouter)은 건드리지 않는다.
    await waitFor(() => expect(setApiKeys).toHaveBeenCalledWith("sk-ant-abc", undefined));
    await waitFor(() => expect(input.value).toBe(""));
    await screen.findByText(/Anthropic 키 있음/);
    await screen.findByText("키를 저장했습니다.");
  });

  it("저장된 키에만 삭제 버튼이 뜨고, 누르면 Anthropic 칸만 빈 문자열로 보낸다", async () => {
    hydrate({ summaryProvider: "claude" });
    apiKeyStatus.mockResolvedValue({ ...STATUS, anthropicSet: true, anthropicFromEnv: false });
    setApiKeys.mockResolvedValue({ ...STATUS, anthropicSet: false });
    render(<SettingsDialog />);
    await screen.findByText(/Anthropic 키 있음/);
    expect((screen.getByPlaceholderText("저장됨 (변경 시 입력)") as HTMLInputElement).type).toBe(
      "password",
    );

    fireEvent.click(screen.getByText("키 삭제"));
    await waitFor(() => expect(setApiKeys).toHaveBeenCalledWith("", undefined));
    await screen.findByText("키를 삭제했습니다.");
    await screen.findByText(/Anthropic 키 없음/);
  });

  it("환경변수 출처인 키는 삭제 버튼이 뜨지 않는다", async () => {
    hydrate({ summaryProvider: "claude" });
    apiKeyStatus.mockResolvedValue({ ...STATUS, anthropicSet: true, anthropicFromEnv: true });
    render(<SettingsDialog />);
    await screen.findByText(/Anthropic 키 있음\(환경변수\)/);

    expect(screen.queryByText("키 삭제")).toBeNull();
  });

  it("저장 실패는 오류 문장으로 보여준다", async () => {
    hydrate({ summaryProvider: "claude" });
    setApiKeys.mockRejectedValue("disk full");
    render(<SettingsDialog />);
    await screen.findByText(/Anthropic 키 없음/);

    fireEvent.change(screen.getByPlaceholderText("sk-ant-…"), { target: { value: "sk-ant-x" } });
    fireEvent.click(screen.getByText("키 저장"));
    await screen.findByText("키 저장 실패: disk full");
  });
});
