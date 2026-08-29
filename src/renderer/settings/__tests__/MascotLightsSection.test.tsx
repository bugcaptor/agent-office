// @vitest-environment jsdom
//
// src/renderer/settings/__tests__/MascotLightsSection.test.tsx
//
// 마스코트 신호등(docs/mascot-lights-design.md) 설정 UI — 모드 선택·세로 배열
// 체크박스가 저장 payload에 실리는지, 마스코트가 꺼져 있으면 세 컨트롤 모두
// 비활성인지, 프로젝트 모드에서만 폴더 목록 편집기가 보이고 추가(중복 무시)/
// 제거가 스토어에 반영되는지 확인한다. `SettingsDialogTalk.test.tsx`와 같은
// 구성 — tauriApi를 모킹해 실 IPC 없이 검증한다.
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { AppSettings } from "@shared/types";

const pickDirectory = vi.fn<(initialDir?: string) => Promise<string | null>>(() =>
  Promise.resolve(null),
);
const setAppSettings = vi.fn<(s: AppSettings) => Promise<void>>(() => Promise.resolve());

vi.mock("../../ipc/tauriApi", () => ({
  tauriApi: {
    pickDirectory: (initialDir?: string) => pickDirectory(initialDir),
    setAppSettings: (s: AppSettings) => setAppSettings(s),
  },
}));

import { useAppStore } from "../../store/appStore";
import { SettingsDialog } from "../SettingsDialog";

const initialState = useAppStore.getState();

/** 마스코트 신호등 섹션은 "시스템" 탭에 있다. */
function openSystemTab() {
  fireEvent.click(screen.getByRole("tab", { name: "시스템" }));
}

function hydrate(
  lights: Partial<
    Pick<
      AppSettings,
      | "mascotEnabled"
      | "mascotLightsMode"
      | "mascotLightsVertical"
      | "mascotLightsProjects"
      | "mascotLightsFace"
      | "mascotLightsLabel"
    >
  >,
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
      mascotEnabled: true,
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
      ...lights,
    },
    false,
  );
  useAppStore.getState().openModal({ kind: "settings" });
}

beforeEach(() => {
  useAppStore.setState(initialState, true);
  pickDirectory.mockClear();
  pickDirectory.mockResolvedValue(null);
  setAppSettings.mockClear();
});

afterEach(() => cleanup());

describe("MascotLightsSection", () => {
  it("모드 셀렉터 변경이 스토어와 저장 payload에 반영된다", () => {
    hydrate({ mascotEnabled: true, mascotLightsMode: "off" });
    render(<SettingsDialog />);
    openSystemTab();

    fireEvent.change(screen.getByRole("combobox", { name: /상태 신호등/ }), {
      target: { value: "agents" },
    });

    expect(useAppStore.getState().appSettings.mascotLightsMode).toBe("agents");
    expect(setAppSettings).toHaveBeenCalledTimes(1);
    expect(setAppSettings.mock.calls[0][0].mascotLightsMode).toBe("agents");
  });

  it("칸 얼굴 셀렉터 변경이 스토어와 저장 payload에 반영된다", () => {
    hydrate({ mascotEnabled: true, mascotLightsFace: "sprite" });
    render(<SettingsDialog />);
    openSystemTab();

    fireEvent.change(screen.getByRole("combobox", { name: /칸에 띄울 얼굴/ }), {
      target: { value: "portrait" },
    });

    expect(useAppStore.getState().appSettings.mascotLightsFace).toBe("portrait");
    expect(setAppSettings.mock.calls[0][0].mascotLightsFace).toBe("portrait");
  });

  it("칸에 표시할 이름 셀렉터 변경이 스토어와 저장 payload에 반영된다", () => {
    hydrate({ mascotEnabled: true, mascotLightsLabel: "auto" });
    render(<SettingsDialog />);
    openSystemTab();

    fireEvent.change(screen.getByRole("combobox", { name: /칸에 표시할 이름/ }), {
      target: { value: "task" },
    });

    expect(useAppStore.getState().appSettings.mascotLightsLabel).toBe("task");
    expect(setAppSettings.mock.calls[0][0].mascotLightsLabel).toBe("task");
  });

  it("칸에 표시할 이름 셀렉터에서 프로젝트+작업을 고르면 projecttask로 저장된다", () => {
    hydrate({ mascotEnabled: true, mascotLightsLabel: "auto" });
    render(<SettingsDialog />);
    openSystemTab();

    fireEvent.change(screen.getByRole("combobox", { name: /칸에 표시할 이름/ }), {
      target: { value: "projecttask" },
    });

    expect(useAppStore.getState().appSettings.mascotLightsLabel).toBe("projecttask");
    expect(setAppSettings.mock.calls[0][0].mascotLightsLabel).toBe("projecttask");
  });

  it("세로 배열 체크박스가 저장 payload에 반영된다", () => {
    hydrate({ mascotEnabled: true, mascotLightsVertical: false });
    render(<SettingsDialog />);
    openSystemTab();

    fireEvent.click(screen.getByRole("checkbox", { name: /세로로 표시/ }));

    expect(useAppStore.getState().appSettings.mascotLightsVertical).toBe(true);
    expect(setAppSettings.mock.calls[0][0].mascotLightsVertical).toBe(true);
  });

  it("마스코트가 꺼져 있으면 모드·세로·폴더 편집 컨트롤이 모두 비활성이다", () => {
    hydrate({
      mascotEnabled: false,
      mascotLightsMode: "projects",
      mascotLightsProjects: ["/repo/a"],
    });
    render(<SettingsDialog />);
    openSystemTab();

    const modeSelect = screen.getByRole("combobox", { name: /상태 신호등/ }) as HTMLSelectElement;
    const faceSelect = screen.getByRole("combobox", { name: /칸에 띄울 얼굴/ }) as HTMLSelectElement;
    const labelSelect = screen.getByRole("combobox", { name: /칸에 표시할 이름/ }) as HTMLSelectElement;
    const verticalCheckbox = screen.getByRole("checkbox", { name: /세로로 표시/ }) as HTMLInputElement;
    expect(modeSelect.disabled).toBe(true);
    expect(faceSelect.disabled).toBe(true);
    expect(labelSelect.disabled).toBe(true);
    expect(verticalCheckbox.disabled).toBe(true);
    for (const btn of screen.getAllByRole("button", { name: /추가|제거/ })) {
      expect((btn as HTMLButtonElement).disabled).toBe(true);
    }
  });

  it("프로젝트 모드가 아니면 폴더 목록 편집기가 보이지 않는다", () => {
    hydrate({ mascotEnabled: true, mascotLightsMode: "agents" });
    render(<SettingsDialog />);
    openSystemTab();

    expect(screen.queryByText(/프로젝트 폴더/)).toBeNull();
  });

  it("프로젝트 모드에서 목록이 비어 있으면 안내 문구를 보여준다", () => {
    hydrate({ mascotEnabled: true, mascotLightsMode: "projects", mascotLightsProjects: [] });
    render(<SettingsDialog />);
    openSystemTab();

    expect(screen.getByText("아직 등록된 폴더가 없습니다.")).toBeTruthy();
  });

  it("폴더 추가 버튼이 pickDirectory 결과를 목록에 append한다", async () => {
    pickDirectory.mockResolvedValue("/Users/me/dev/agent-office");
    hydrate({ mascotEnabled: true, mascotLightsMode: "projects", mascotLightsProjects: [] });
    render(<SettingsDialog />);
    openSystemTab();

    fireEvent.click(screen.getByRole("button", { name: "폴더 추가…" }));
    await screen.findByText("/Users/me/dev/agent-office");

    expect(useAppStore.getState().appSettings.mascotLightsProjects).toEqual([
      "/Users/me/dev/agent-office",
    ]);
  });

  it("이미 등록된 폴더를 다시 고르면 중복 추가하지 않는다", async () => {
    pickDirectory.mockResolvedValue("/repo/a");
    hydrate({
      mascotEnabled: true,
      mascotLightsMode: "projects",
      mascotLightsProjects: ["/repo/a"],
    });
    render(<SettingsDialog />);
    openSystemTab();

    fireEvent.click(screen.getByRole("button", { name: "폴더 추가…" }));
    await Promise.resolve();
    await Promise.resolve();

    expect(useAppStore.getState().appSettings.mascotLightsProjects).toEqual(["/repo/a"]);
    // T3: 상태가 그대로인 것만 보면 핸들러가 통째로 죽어도(예외로 조기 종료)
    // 통과한다 — 저장 호출 자체가 없었는지도 함께 확인한다.
    expect(setAppSettings).not.toHaveBeenCalled();
  });

  it("제거 버튼이 인덱스로 해당 폴더만 지운다", () => {
    hydrate({
      mascotEnabled: true,
      mascotLightsMode: "projects",
      mascotLightsProjects: ["/repo/a", "/repo/b"],
    });
    render(<SettingsDialog />);
    openSystemTab();

    fireEvent.click(screen.getAllByRole("button", { name: "제거" })[0]);

    expect(useAppStore.getState().appSettings.mascotLightsProjects).toEqual(["/repo/b"]);
  });
});
