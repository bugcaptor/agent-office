// @vitest-environment jsdom
//
// src/renderer/settings/__tests__/SettingsDialogControl.test.tsx
//
// CLI 제어(이슈 #55) 설정 섹션 — 2단계 승인 UI. controlStatus 조회 결과에 따라
// 승인/취소 버튼과 연결 안내가 바뀌고, 버튼이 controlApprove/controlRevoke를
// 호출하는지 확인한다. tauriApi를 모킹해 실 IPC 없이 검증한다.
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { ControlStatus } from "@shared/types";

const controlStatus = vi.fn<() => Promise<ControlStatus>>();
const controlApprove = vi.fn<() => Promise<void>>(() => Promise.resolve());
const controlRevoke = vi.fn<() => Promise<void>>(() => Promise.resolve());
const setAppSettings = vi.fn<(s: unknown) => Promise<void>>(() => Promise.resolve());

vi.mock("../../ipc/tauriApi", () => ({
  tauriApi: {
    controlStatus: () => controlStatus(),
    controlApprove: () => controlApprove(),
    controlRevoke: () => controlRevoke(),
    setAppSettings: (s: unknown) => setAppSettings(s),
  },
}));

import { useAppStore } from "../../store/appStore";
import { SettingsDialog } from "../SettingsDialog";

const initialState = useAppStore.getState();

function hydrate(cliEnabled: boolean) {
  useAppStore.getState().hydrateSettings(
    {
      version: 1,
      summarizerEnabled: false,
      summaryProvider: "claude",
      observerEnabled: false,
      soundEnabled: true,
      soundVolume: 0.5,
      externalTerminal: "terminal",
      externalEditor: "system",
      attentionHoldMs: 5000,
      gitStatusEnabled: true,
      cliEnabled,
    },
    false,
  );
  useAppStore.getState().openModal({ kind: "settings" });
}

beforeEach(() => {
  useAppStore.setState(initialState, true);
  controlStatus.mockReset();
  controlApprove.mockClear();
  controlRevoke.mockClear();
  setAppSettings.mockClear();
});

afterEach(() => cleanup());

describe("SettingsDialog · CLI 제어", () => {
  it("cliEnabled 토글이 updateAppSettings로 반영된다", () => {
    controlStatus.mockResolvedValue({
      enabled: false,
      running: false,
      approved: false,
      port: null,
      appDataDir: "/data",
    });
    hydrate(false);
    render(<SettingsDialog />);

    fireEvent.click(screen.getByRole("checkbox", { name: /CLI 제어/ }));
    expect(useAppStore.getState().appSettings.cliEnabled).toBe(true);
  });

  it("활성화+미승인이면 승인 버튼을 보이고 클릭 시 controlApprove를 호출한다", async () => {
    controlStatus.mockResolvedValue({
      enabled: true,
      running: true,
      approved: false,
      port: 51234,
      appDataDir: "/data",
    });
    hydrate(true);
    render(<SettingsDialog />);

    const approveBtn = await screen.findByRole("button", { name: /CLI 제어 승인/ });
    expect(screen.getByText(/실행 중\(포트 51234\)/)).toBeTruthy();
    fireEvent.click(approveBtn);
    await waitFor(() => expect(controlApprove).toHaveBeenCalledTimes(1));
  });

  it("승인됨이면 취소 버튼과 연결 안내(app_data 경로)를 보인다", async () => {
    controlStatus.mockResolvedValue({
      enabled: true,
      running: true,
      approved: true,
      port: 51234,
      appDataDir: "/Users/x/Library/Application Support/com.bugcaptor.agent-office",
    });
    hydrate(true);
    render(<SettingsDialog />);

    const revokeBtn = await screen.findByRole("button", { name: /승인 취소/ });
    expect(
      screen.getByText("/Users/x/Library/Application Support/com.bugcaptor.agent-office"),
    ).toBeTruthy();
    fireEvent.click(revokeBtn);
    await waitFor(() => expect(controlRevoke).toHaveBeenCalledTimes(1));
  });
});
