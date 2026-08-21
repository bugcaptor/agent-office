// @vitest-environment jsdom
//
// src/renderer/settings/__tests__/SettingsDialogOpenrouter.test.tsx
//
// 요약기 provider가 OpenRouter일 때만 뜨는 보조 UI(kbm #2e4):
//  - API 키를 그 자리에서 넣고 저장한다 — 단, 저장소는 TTS와 **하나**여야
//    한다(`ttsSetKeys`의 셋째 칸). 요약 전용 키가 따로 생기면 같은 키를 두 번
//    넣게 되고 어느 쪽이 쓰이는지 알 수 없어진다.
//  - "요약 테스트"는 전용 커맨드가 아니라 실제 `summarizeText` 경로를 탄다 —
//    여기서 성공하면 실제 라벨 요약도 성공한다는 뜻이어야 하기 때문이다.
//  - 모델 추천 목록은 정적 프리셋 + 실시간 카탈로그의 합집합이고, 조회는
//    세션당 1회다.
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { AppSettings, TtsStatus } from "@shared/types";

const ttsKeyStatus = vi.fn<() => Promise<TtsStatus>>();
const ttsSetKeys = vi.fn<(e?: string, a?: string, o?: string) => Promise<TtsStatus>>();
const summarizeText = vi.fn<(...args: unknown[]) => Promise<string>>();
const openrouterListModels = vi.fn<() => Promise<string[]>>();
const setAppSettings = vi.fn<(s: unknown) => Promise<void>>(() => Promise.resolve());

vi.mock("../../ipc/tauriApi", () => ({
  tauriApi: {
    ttsKeyStatus: () => ttsKeyStatus(),
    ttsSetKeys: (e?: string, a?: string, o?: string) => ttsSetKeys(e, a, o),
    summarizeText: (...args: unknown[]) => summarizeText(...args),
    openrouterListModels: () => openrouterListModels(),
    setAppSettings: (s: unknown) => setAppSettings(s),
  },
}));

import { useAppStore } from "../../store/appStore";
import { SettingsDialog } from "../SettingsDialog";
import { OPENROUTER_MODEL_PRESETS, resetOpenrouterModelsCache } from "../openrouterModels";

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

/** 요약 모델 칸이 가리키는 datalist의 항목들. */
function datalistValues(id: string): string[] {
  const el = document.getElementById(id) as HTMLDataListElement | null;
  return el ? Array.from(el.options).map((o) => o.value) : [];
}

beforeEach(() => {
  useAppStore.setState(initialState, true);
  resetOpenrouterModelsCache();
  ttsKeyStatus.mockReset().mockResolvedValue(STATUS);
  ttsSetKeys.mockReset().mockResolvedValue({ ...STATUS, openrouterSet: true });
  summarizeText.mockReset().mockResolvedValue("한 문장 요약입니다.");
  openrouterListModels.mockReset().mockResolvedValue([]);
  setAppSettings.mockClear();
});

afterEach(() => cleanup());

describe("SettingsDialog · OpenRouter 요약 설정", () => {
  it("다른 요약기에서는 키 입력도 테스트 버튼도 없고 카탈로그를 조회하지 않는다", () => {
    hydrate({ summaryProvider: "claude" });
    render(<SettingsDialog />);

    expect(screen.queryByText("요약 테스트")).toBeNull();
    expect(screen.queryByPlaceholderText("sk-or-…")).toBeNull();
    expect(openrouterListModels).not.toHaveBeenCalled();
    expect(ttsKeyStatus).not.toHaveBeenCalled();
  });

  it("키 상태를 마스킹된 문장으로 보여주고, 저장은 TTS와 같은 저장소를 쓴다", async () => {
    hydrate({ summaryProvider: "openrouter" });
    render(<SettingsDialog />);
    await screen.findByText(/OpenRouter 키 없음/);

    const input = screen.getByPlaceholderText("sk-or-…") as HTMLInputElement;
    expect(input.type).toBe("password");
    // 빈 입력으로는 저장할 수 없다(실수로 기존 키를 건드리지 않게).
    const save = screen.getByText("키 저장") as HTMLButtonElement;
    expect(save.disabled).toBe(true);

    fireEvent.change(input, { target: { value: "sk-or-abc" } });
    fireEvent.click(save);
    // 앞 두 칸(ElevenLabs/Anthropic)은 건드리지 않는다.
    await waitFor(() => expect(ttsSetKeys).toHaveBeenCalledWith(undefined, undefined, "sk-or-abc"));
    // 저장 후 입력은 비고, 상태는 "있음"으로 갱신된다.
    await waitFor(() => expect(input.value).toBe(""));
    await screen.findByText(/OpenRouter 키 있음/);
    await screen.findByText("키를 저장했습니다.");
  });

  it("요약 테스트가 실제 summarizeText 경로를 타고 결과를 보여준다", async () => {
    hydrate({ summaryProvider: "openrouter" });
    render(<SettingsDialog />);

    fireEvent.click(screen.getByText("요약 테스트"));
    await waitFor(() => expect(summarizeText).toHaveBeenCalled());
    const [provider, instruction, text, purpose] = summarizeText.mock.calls[0];
    expect(provider).toBe("openrouter");
    expect(String(instruction).length).toBeGreaterThan(0);
    expect(String(text).length).toBeGreaterThan(0);
    expect(purpose).toBe("label");
    await screen.findByText("요약: 한 문장 요약입니다.");
  });

  it("테스트 실패의 안정 코드를 사람이 읽는 문장으로 바꿔 준다", async () => {
    hydrate({ summaryProvider: "openrouter" });
    summarizeText.mockRejectedValue("openrouter-key-missing");
    render(<SettingsDialog />);

    fireEvent.click(screen.getByText("요약 테스트"));
    await screen.findByText("요약 실패: OpenRouter API 키가 없습니다");

    // 모르는 오류는 원문 그대로 — 상류 오류는 종류가 열려 있다.
    summarizeText.mockRejectedValue("openrouter provider error 429");
    fireEvent.click(screen.getByText("요약 테스트"));
    await screen.findByText("요약 실패: openrouter provider error 429");
  });

  it("모델 추천은 프리셋을 앞에 두고 실시간 카탈로그를 뒤에 붙인다(중복 제거)", async () => {
    // 응답에 프리셋과 겹치는 id가 섞여 있어도 한 번만 나와야 한다.
    openrouterListModels.mockResolvedValue([
      "openai/gpt-5.4-mini",
      "zzz/new-model",
      "aaa/other-model",
    ]);
    hydrate({ summaryProvider: "openrouter" });
    render(<SettingsDialog />);

    await waitFor(() =>
      expect(datalistValues("summary-openrouter-models")).toEqual([
        ...OPENROUTER_MODEL_PRESETS,
        "zzz/new-model",
        "aaa/other-model",
      ]),
    );
  });

  it("카탈로그 조회가 실패해도 조용히 정적 프리셋만 쓴다", async () => {
    openrouterListModels.mockRejectedValue(new Error("offline"));
    hydrate({ summaryProvider: "openrouter" });
    render(<SettingsDialog />);

    await waitFor(() => expect(openrouterListModels).toHaveBeenCalled());
    expect(datalistValues("summary-openrouter-models")).toEqual(OPENROUTER_MODEL_PRESETS);
    // 실패 문구가 설정 화면에 튀어나오지 않는다.
    expect(screen.queryByText(/offline/)).toBeNull();
  });

  // 다이얼로그는 열고 닫기를 반복하는 창이고 카탈로그는 그 사이에 바뀌지
  // 않는다 — 열 때마다 다시 두드리면 그냥 낭비다.
  it("카탈로그는 다이얼로그를 다시 열어도 세션당 한 번만 조회한다", async () => {
    hydrate({ summaryProvider: "openrouter" });
    const { rerender } = render(<SettingsDialog />);
    await waitFor(() => expect(openrouterListModels).toHaveBeenCalledTimes(1));

    useAppStore.getState().closeModal();
    rerender(<SettingsDialog />);
    useAppStore.getState().openModal({ kind: "settings" });
    rerender(<SettingsDialog />);

    await waitFor(() => expect(datalistValues("summary-openrouter-models").length).toBeGreaterThan(0));
    expect(openrouterListModels).toHaveBeenCalledTimes(1);
  });
});
