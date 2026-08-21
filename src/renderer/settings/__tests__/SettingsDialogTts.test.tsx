// @vitest-environment jsdom
//
// src/renderer/settings/__tests__/SettingsDialogTts.test.tsx
//
// 알림 대사 TTS 설정 섹션. 토글 OFF면 상세(키 입력·공급자·시청)가 접혀
// 있어야 하고, ON이면 키 상태가 마스킹된 형태로만 표시돼야 한다 —
// **키 값이 화면에 돌아오는 경로가 없다**는 것이 이 섹션의 핵심 계약이다.
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { AppSettings, TtsStatus } from "@shared/types";

const ttsKeyStatus = vi.fn<() => Promise<TtsStatus>>();
const ttsSetKeys = vi.fn<(e?: string, a?: string, o?: string) => Promise<TtsStatus>>();
const setAppSettings = vi.fn<(s: unknown) => Promise<void>>(() => Promise.resolve());
const controlStatus = vi.fn(() => Promise.reject(new Error("not used")));

vi.mock("../../ipc/tauriApi", () => ({
  tauriApi: {
    ttsKeyStatus: () => ttsKeyStatus(),
    ttsSetKeys: (e?: string, a?: string, o?: string) => ttsSetKeys(e, a, o),
    setAppSettings: (s: unknown) => setAppSettings(s),
    controlStatus: () => controlStatus(),
  },
}));
// 미리듣기는 AudioContext가 필요하니 이 테스트에서는 대체한다.
vi.mock("../../sound/soundManager", () => ({
  previewVoice: () => Promise.resolve("[nervous] 이거 진행해도 될까요?"),
}));

import { useAppStore } from "../../store/appStore";
import { SettingsDialog } from "../SettingsDialog";

const initialState = useAppStore.getState();

const STATUS: TtsStatus = {
  elevenlabsSet: true,
  anthropicSet: false,
  elevenlabsFromEnv: false,
  anthropicFromEnv: false,
  openrouterSet: false,
  openrouterFromEnv: false,
  claudeCliAvailable: true,
  effectiveRewriteVia: "claude-cli",
};

/** TTS 섹션은 "소리·음성" 탭에 있다 — 렌더 직후 그 탭을 열어야 마운트된다. */
function openTab(name: string) {
  fireEvent.click(screen.getByRole("tab", { name }));
}

function hydrate(patch: Partial<AppSettings> = {}) {
  useAppStore.getState().hydrateSettings(
    {
      version: 1,
      summarizerEnabled: false,
      summaryProvider: "claude",
      summaryModels: {
        claude: { light: "", heavy: "" },
        codex: { light: "", heavy: "" },
        agy: { light: "", heavy: "" },
        gemini: { light: "", heavy: "" },
        openrouter: { light: "", heavy: "" },
      },
      diaryEnabled: false,
      observerEnabled: false,
      typingSoundEnabled: true,
      notifySoundEnabled: true,
      soundVolume: 0.5,
      externalTerminal: "terminal",
      externalEditor: "system",
      attentionHoldMs: 5000,
      gitStatusEnabled: true,
      fileIndexBackend: "walker",
      cliEnabled: false,
      keepAwakeEnabled: false,
      sessionLogEnabled: true,
      mascotEnabled: false,
      ttsEnabled: false,
      ttsRewriteModelAnthropic: "claude-haiku-4-5",
      ttsRewriteModelOpenrouter: "openai/gpt-5.4-mini",
      ttsRewriteProvider: "auto",
      ...patch,
    },
    false,
  );
  useAppStore.getState().openModal({ kind: "settings" });
}

beforeEach(() => {
  useAppStore.setState(initialState, true);
  ttsKeyStatus.mockReset().mockResolvedValue(STATUS);
  ttsSetKeys.mockReset().mockResolvedValue({ ...STATUS, anthropicSet: true });
  setAppSettings.mockClear();
});

afterEach(() => cleanup());

describe("SettingsDialog · 알림 대사 TTS", () => {
  it("기본은 꺼짐이고, 꺼져 있으면 키 입력·시청이 노출되지 않는다", () => {
    hydrate();
    render(<SettingsDialog />);
    openTab("소리·음성");
    const toggle = screen.getByLabelText(/알림 대사 읽어주기/) as HTMLInputElement;
    expect(toggle.checked).toBe(false);
    expect(screen.queryByText(/ElevenLabs API 키/)).toBeNull();
    expect(screen.queryByText("시청 (미리듣기)")).toBeNull();
  });

  it("토글이 updateAppSettings로 반영된다", () => {
    hydrate();
    render(<SettingsDialog />);
    openTab("소리·음성");
    fireEvent.click(screen.getByLabelText(/알림 대사 읽어주기/));
    expect(useAppStore.getState().appSettings.ttsEnabled).toBe(true);
    expect(setAppSettings).toHaveBeenCalled();
  });

  it("켜면 키 상태를 마스킹된 문장으로만 보여준다 (키 값 노출 경로 없음)", async () => {
    hydrate({ ttsEnabled: true });
    render(<SettingsDialog />);
    openTab("소리·음성");
    await waitFor(() => expect(ttsKeyStatus).toHaveBeenCalled());
    await screen.findByText(/ElevenLabs 키 있음/);
    // "자동"이 실제로 무엇을 고를지 안내한다.
    expect(screen.getByText(/리라이트: claude CLI \(구독\)/)).toBeTruthy();
    // 입력 필드는 비어 있고 placeholder만 "저장됨"을 알린다.
    const el = screen.getByPlaceholderText("저장됨 (변경 시 입력)") as HTMLInputElement;
    expect(el.value).toBe("");
    expect(el.type).toBe("password");
  });

  it("공백뿐인 입력이면 저장 버튼이 비활성이고, 입력 후 저장하면 필드를 비운다", async () => {
    hydrate({ ttsEnabled: true });
    render(<SettingsDialog />);
    openTab("소리·음성");
    await waitFor(() => expect(ttsKeyStatus).toHaveBeenCalled());
    const save = screen.getByText("키 저장") as HTMLButtonElement;
    expect(save.disabled).toBe(true);

    const anthropic = screen.getByPlaceholderText("sk-ant-…") as HTMLInputElement;
    fireEvent.change(anthropic, { target: { value: "sk-ant-abc" } });
    expect(save.disabled).toBe(false);
    fireEvent.click(save);
    // 손대지 않은 필드는 undefined로 보내 기존 값을 보존한다.
    await waitFor(() =>
      expect(ttsSetKeys).toHaveBeenCalledWith(undefined, "sk-ant-abc", undefined),
    );
    await waitFor(() => expect(anthropic.value).toBe(""));
    await screen.findByText("키를 저장했습니다.");
  });

  // 라벨이 설명문(small)까지 감싸고 있어 접근성 이름으로 특정하기 어렵다 —
  // "그 값을 고를 수 있는 select"로 집는다. (현재 값으로 집으면 다이얼로그의
  // 다른 select와 부딪힌다 — "터미널 색상"도 기본값이 "auto"다.)
  const selectByOption = (optionValue: string) =>
    screen
      .getAllByRole("combobox")
      .find((el) =>
        Array.from((el as HTMLSelectElement).options).some((o) => o.value === optionValue),
      )!;

  it("리라이트 공급자/모델 입력이 설정에 반영된다", async () => {
    hydrate({ ttsEnabled: true });
    render(<SettingsDialog />);
    openTab("소리·음성");
    await waitFor(() => expect(ttsKeyStatus).toHaveBeenCalled());

    fireEvent.change(selectByOption("claude-cli"), { target: { value: "claude-cli" } });
    expect(useAppStore.getState().appSettings.ttsRewriteProvider).toBe("claude-cli");

    // 모델은 자유 입력이다 — 목록에 없는 모델도 그대로 저장된다.
    const model = screen.getByPlaceholderText("claude-haiku-4-5") as HTMLInputElement;
    fireEvent.change(model, { target: { value: "claude-future-9" } });
    expect(useAppStore.getState().appSettings.ttsRewriteModelAnthropic).toBe("claude-future-9");
  });

  // 공급자를 OpenRouter로 바꾸면 모델 칸이 그쪽 값으로 갈린다 — 두 칸이 함께
  // 보이면 어느 값이 실제로 쓰이는지 알 수 없다.
  it("OpenRouter를 고르면 OpenRouter 모델 칸만 보이고 그 값이 저장된다", async () => {
    hydrate({ ttsEnabled: true });
    render(<SettingsDialog />);
    openTab("소리·음성");
    await waitFor(() => expect(ttsKeyStatus).toHaveBeenCalled());

    fireEvent.change(selectByOption("openrouter"), { target: { value: "openrouter" } });
    expect(useAppStore.getState().appSettings.ttsRewriteProvider).toBe("openrouter");
    expect(screen.queryByPlaceholderText("claude-haiku-4-5")).toBeNull();

    const model = screen.getByPlaceholderText("openai/gpt-5.4-mini") as HTMLInputElement;
    expect(model.value).toBe("openai/gpt-5.4-mini");
    fireEvent.change(model, { target: { value: "google/gemini-3-pro" } });
    const s = useAppStore.getState().appSettings;
    expect(s.ttsRewriteModelOpenrouter).toBe("google/gemini-3-pro");
    expect(s.ttsRewriteModelAnthropic).toBe("claude-haiku-4-5");
  });

  it("OpenRouter 키도 마스킹 상태로만 표시하고 저장 시 셋째 인자로 넘긴다", async () => {
    hydrate({ ttsEnabled: true });
    render(<SettingsDialog />);
    openTab("소리·음성");
    await waitFor(() => expect(ttsKeyStatus).toHaveBeenCalled());
    await screen.findByText(/OpenRouter 키 없음/);

    const or = screen.getByPlaceholderText("sk-or-…") as HTMLInputElement;
    expect(or.type).toBe("password");
    fireEvent.change(or, { target: { value: "sk-or-abc" } });
    fireEvent.click(screen.getByText("키 저장"));
    await waitFor(() =>
      expect(ttsSetKeys).toHaveBeenCalledWith(undefined, undefined, "sk-or-abc"),
    );
    await waitFor(() => expect(or.value).toBe(""));
  });

  it("시청 버튼이 실제 발화된 대사를 보여준다", async () => {
    hydrate({ ttsEnabled: true });
    render(<SettingsDialog />);
    openTab("소리·음성");
    await waitFor(() => expect(ttsKeyStatus).toHaveBeenCalled());
    fireEvent.click(screen.getByText("시청 (미리듣기)"));
    await screen.findByText(/발화: \[nervous\] 이거 진행해도 될까요\?/);
  });

  // 무음 모드인 줄 모르고 "왜 발화가 안 되지"로 헤매는 사고가 실제로 있었다 —
  // 미리듣기는 무음에서도 울리므로 그 옆에서 명시적으로 알린다.
  it("무음 모드면 미리듣기 옆에 실제 알림은 발화되지 않는다고 알린다", async () => {
    hydrate({ ttsEnabled: true });
    render(<SettingsDialog />);
    openTab("소리·음성");
    await waitFor(() => expect(ttsKeyStatus).toHaveBeenCalled());
    expect(screen.queryByText(/무음 모드가 켜져 있어/)).toBeNull();
    useAppStore.getState().toggleMuted();
    await screen.findByText(/무음 모드가 켜져 있어/);
  });

  it("claude CLI 경로는 구독 사용량 소모를 안내한다", async () => {
    hydrate({ ttsEnabled: true });
    render(<SettingsDialog />);
    openTab("소리·음성");
    await waitFor(() => expect(ttsKeyStatus).toHaveBeenCalled());
    expect(screen.getByText(/구독 사용량을 소모합니다/)).toBeTruthy();
  });
});
