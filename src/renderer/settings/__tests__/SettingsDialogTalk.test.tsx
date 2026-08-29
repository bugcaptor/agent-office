// @vitest-environment jsdom
//
// src/renderer/settings/__tests__/SettingsDialogTalk.test.tsx
//
// 동료 대화(docs/agent-talk-design.md) 설정 섹션 — 옵트인 토글이 저장
// payload(setAppSettings)에 실리는지, 꺼져 있을 때 하위 숫자 입력이 비활성인지,
// 켰을 때 숫자 입력이 범위 안으로 클램프되는지 확인한다. tauriApi를 모킹해 실
// IPC 없이 검증한다(같은 탭의 CLI 제어 섹션이 부르는 controlStatus 포함).
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { AppSettings, ControlStatus } from "@shared/types";

const controlStatus = vi.fn<() => Promise<ControlStatus>>(() =>
  Promise.resolve({
    enabled: false,
    running: false,
    approved: false,
    port: null,
    appDataDir: "/data",
  }),
);
const setAppSettings = vi.fn<(s: AppSettings) => Promise<void>>(() => Promise.resolve());

vi.mock("../../ipc/tauriApi", () => ({
  tauriApi: {
    controlStatus: () => controlStatus(),
    controlApprove: () => Promise.resolve(),
    controlRevoke: () => Promise.resolve(),
    setAppSettings: (s: AppSettings) => setAppSettings(s),
  },
}));

import { useAppStore } from "../../store/appStore";
import { SettingsDialog } from "../SettingsDialog";

const initialState = useAppStore.getState();

/** 동료 대화 섹션은 "제어" 탭에 있다 — 렌더 직후 그 탭을 열어야 마운트된다. */
function openControlTab() {
  fireEvent.click(screen.getByRole("tab", { name: "제어" }));
}

function hydrate(
  talk: Partial<Pick<AppSettings, "talkEnabled" | "talkMaxTurns" | "talkIdleQuietMs">>,
  rest: Partial<AppSettings> = {},
) {
  useAppStore.getState().hydrateSettings(
    {
      version: 1,
      language: "system",
      summarizerEnabled: false,
      summaryProvider: "claude",
      summaryModels: {
        claude: { light: "", heavy: "", command: "" },
        codex: { light: "", heavy: "", command: "" },
        agy: { light: "", heavy: "", command: "" },
        gemini: { light: "", heavy: "", command: "" },
        opencode: { light: "", heavy: "", command: "" },
        openrouter: { light: "", heavy: "", command: "" },
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
      workdirShowIgnored: false,
      fileIndexBackend: "walker",
      cliEnabled: false,
      keepAwakeEnabled: false,
      sessionLogEnabled: true,
      mascotEnabled: false,
      mascotLightsMode: "off",
      mascotLightsVertical: false,
      mascotLightsProjects: [],
      mascotLightsFace: "sprite",
      mascotLightsLabel: "auto",
      usageFloatEnabled: true,
      sessionCostEnabled: true,
      ttsEnabled: false,
      ttsRewriteModelAnthropic: "claude-haiku-4-5",
      ttsRewriteModelOpenrouter: "openai/gpt-5.4-mini",
      ttsRewriteProvider: "auto",
      webRemoteBind: "tailnet",
      webRemotePort: 47800,
      webRemoteEnabled: false,
      talkEnabled: false,
      talkMaxTurns: 6,
      talkIdleQuietMs: 3000,
      ...talk,
      ...rest,
    },
    false,
  );
  useAppStore.getState().openModal({ kind: "settings" });
}

beforeEach(() => {
  useAppStore.setState(initialState, true);
  controlStatus.mockClear();
  setAppSettings.mockClear();
});

afterEach(() => cleanup());

describe("SettingsDialog · 동료 대화", () => {
  it("토글이 스토어와 저장 payload에 반영된다", () => {
    hydrate({ talkEnabled: false });
    render(<SettingsDialog />);
    openControlTab();

    fireEvent.click(screen.getByRole("checkbox", { name: /동료 대화/ }));

    expect(useAppStore.getState().appSettings.talkEnabled).toBe(true);
    expect(setAppSettings).toHaveBeenCalledTimes(1);
    expect(setAppSettings.mock.calls[0][0].talkEnabled).toBe(true);
  });

  it("꺼져 있으면 왕복 상한·유휴 대기 입력이 비활성이다", () => {
    hydrate({ talkEnabled: false });
    render(<SettingsDialog />);
    openControlTab();

    const turns = screen.getByRole("spinbutton", { name: /왕복 상한/ }) as HTMLInputElement;
    const quiet = screen.getByRole("spinbutton", { name: /유휴 대기/ }) as HTMLInputElement;
    expect(turns.disabled).toBe(true);
    expect(quiet.disabled).toBe(true);
    expect(turns.value).toBe("6");
    expect(quiet.value).toBe("3000");
  });

  it("켜면 숫자 입력이 활성화되고 값이 저장 payload에 실린다", () => {
    hydrate({ talkEnabled: true });
    render(<SettingsDialog />);
    openControlTab();

    const turns = screen.getByRole("spinbutton", { name: /왕복 상한/ }) as HTMLInputElement;
    expect(turns.disabled).toBe(false);
    fireEvent.change(turns, { target: { value: "12" } });
    expect(useAppStore.getState().appSettings.talkMaxTurns).toBe(12);

    const quiet = screen.getByRole("spinbutton", { name: /유휴 대기/ }) as HTMLInputElement;
    expect(quiet.disabled).toBe(false);
    fireEvent.change(quiet, { target: { value: "1500" } });
    expect(useAppStore.getState().appSettings.talkIdleQuietMs).toBe(1500);

    const last = setAppSettings.mock.calls[setAppSettings.mock.calls.length - 1][0];
    expect(last.talkMaxTurns).toBe(12);
    expect(last.talkIdleQuietMs).toBe(1500);
  });

  it("범위를 벗어난 입력은 상·하한으로 클램프된다", () => {
    hydrate({ talkEnabled: true });
    render(<SettingsDialog />);
    openControlTab();

    fireEvent.change(screen.getByRole("spinbutton", { name: /왕복 상한/ }), {
      target: { value: "999" },
    });
    expect(useAppStore.getState().appSettings.talkMaxTurns).toBe(50);

    fireEvent.change(screen.getByRole("spinbutton", { name: /유휴 대기/ }), {
      target: { value: "-100" },
    });
    expect(useAppStore.getState().appSettings.talkIdleQuietMs).toBe(0);
  });

  it("CLI 제어가 미승인이면 대화가 전부 실패한다고 경고한다", async () => {
    hydrate({ talkEnabled: true });
    render(<SettingsDialog />);
    openControlTab();

    // cliEnabled=false(hydrate 기본) + 미승인 → 경고가 뜬다.
    expect((await screen.findByRole("alert")).textContent).toContain("CLI 제어");
  });

  it("CLI 제어가 켜지고 승인됐으면 경고를 감춘다", async () => {
    // CLI 제어 섹션도 같은 탭에서 controlStatus를 부르므로 Once가 아니라 상시 값으로 둔다.
    controlStatus.mockResolvedValue({
      enabled: true,
      running: true,
      approved: true,
      port: 1234,
      appDataDir: "/data",
    });
    hydrate({ talkEnabled: true }, { cliEnabled: true });
    render(<SettingsDialog />);
    openControlTab();

    await waitFor(() => expect(controlStatus).toHaveBeenCalled());
    expect(screen.queryByRole("alert")).toBeNull();
  });

  it("스킬 발동 안내를 보여 준다", () => {
    hydrate({ talkEnabled: false });
    render(<SettingsDialog />);
    openControlTab();

    expect(screen.getAllByText("/agent-office:talk").length).toBeGreaterThan(0);
  });
});
