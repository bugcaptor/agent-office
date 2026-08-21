// @vitest-environment jsdom
//
// src/renderer/settings/__tests__/SettingsDialog.test.tsx
//
// 상시 설정 다이얼로그(ConfirmQuitDialog와 동일 패턴). FirstRunDialog와
// 달리 스토어 값을 직접 바인딩한다 — 토글 클릭이 즉시 updateAppSettings로
// 반영되는지, 닫기 버튼/백드롭이 closeModal을 부르는지 확인한다.
//
// 탭 4개로 나뉜 뒤로 항목은 해당 탭을 먼저 눌러야 화면에 있다 — 각 테스트가
// `openTab()`으로 자기 탭을 연다(기본은 첫 탭 "일반").
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { useAppStore } from "../../store/appStore";
import { SettingsDialog } from "../SettingsDialog";

const initialState = useAppStore.getState();

/** 이름으로 탭을 연다. 탭 버튼은 role="tab"이라 항목 셀렉터와 부딪히지 않는다. */
function openTab(name: string) {
  fireEvent.click(screen.getByRole("tab", { name }));
}

beforeEach(() => {
  useAppStore.setState(initialState, true);
});

afterEach(() => cleanup());

describe("SettingsDialog", () => {
  it("modal이 settings가 아니면 아무것도 렌더하지 않는다", () => {
    const { container } = render(<SettingsDialog />);
    expect(container.firstChild).toBeNull();
  });

  it("settings 모달일 때 렌더되고 공통 설정 변경을 즉시 반영한다", () => {
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
      },
      false,
    );
    useAppStore.getState().openModal({ kind: "settings" });

    render(<SettingsDialog />);
    expect(screen.getByText("설정")).toBeTruthy();

    fireEvent.click(screen.getByRole("radio", { name: "Codex" }));
    fireEvent.click(screen.getByRole("checkbox", { name: /에이전트 관찰/ }));

    expect(useAppStore.getState().appSettings).toEqual({
      version: 1,
      summarizerEnabled: false,
      summaryProvider: "codex",
      summaryModels: {
        claude: { light: "", heavy: "" },
        codex: { light: "", heavy: "" },
        agy: { light: "", heavy: "" },
        gemini: { light: "", heavy: "" },
      },
      diaryEnabled: false,
      observerEnabled: true,
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
    });
  });

  it("외부 터미널 앱 셀렉터가 iTerm2 선택을 즉시 반영한다", () => {
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
      },
      false,
    );
    useAppStore.getState().openModal({ kind: "settings" });

    render(<SettingsDialog />);
    openTab("시스템");
    // 이제 셀렉터가 둘(외부 터미널/셸 출력 에디터)이므로 이름으로 특정한다.
    fireEvent.change(screen.getByRole("combobox", { name: /외부 터미널/ }), {
      target: { value: "iterm" },
    });

    expect(useAppStore.getState().appSettings.externalTerminal).toBe("iterm");
  });

  it("셸 출력 에디터 셀렉터가 VS Code 선택을 즉시 반영한다", () => {
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
      },
      false,
    );
    useAppStore.getState().openModal({ kind: "settings" });

    render(<SettingsDialog />);
    openTab("시스템");
    fireEvent.change(screen.getByRole("combobox", { name: /셸 출력 에디터/ }), {
      target: { value: "vscode" },
    });

    expect(useAppStore.getState().appSettings.externalEditor).toBe("vscode");
  });


  // 터미널 색상만은 AppSettings(Rust 영속)가 아니라 zustand+localStorage에 산다 —
  // updateAppSettings가 아닌 setXtermTheme에 바인딩돼야 한다.
  it("터미널 색상 셀렉터가 xtermTheme을 즉시 반영한다(앱 테마는 그대로)", () => {
    useAppStore.getState().openModal({ kind: "settings" });
    render(<SettingsDialog />);
    openTab("시스템");

    const select = screen.getByRole("combobox", { name: /터미널 색상/ });
    expect((select as HTMLSelectElement).value).toBe("auto");

    fireEvent.change(select, { target: { value: "pipboy" } });
    expect(useAppStore.getState().xtermTheme).toBe("pipboy");
    expect(useAppStore.getState().theme).toBe(initialState.theme);

    fireEvent.change(select, { target: { value: "auto" } });
    expect(useAppStore.getState().xtermTheme).toBe("auto");
  });

  // 사운드 3분할: 타건음/알림음이 서로 독립된 토글이어야 한다(TTS는 별도 섹션).
  it("타건음·알림음 토글이 각각 독립적으로 반영된다", () => {
    useAppStore.getState().openModal({ kind: "settings" });
    render(<SettingsDialog />);
    openTab("소리·음성");

    fireEvent.click(screen.getByLabelText(/타건음/));
    expect(useAppStore.getState().appSettings.typingSoundEnabled).toBe(false);
    expect(useAppStore.getState().appSettings.notifySoundEnabled).toBe(true);

    fireEvent.click(screen.getByLabelText(/알림음/));
    expect(useAppStore.getState().appSettings.notifySoundEnabled).toBe(false);
    expect(useAppStore.getState().appSettings.typingSoundEnabled).toBe(false);
  });

  // 탭 재편(kbm #2dt): 열 때는 항상 첫 탭 "일반"이고, 탭을 바꾸면 그 탭
  // 항목만 남는다. 탭 상태는 기억하지 않으므로 닫았다 열면 다시 "일반".
  it("탭 전환이 해당 탭 항목만 보여주고, 다시 열면 첫 탭으로 돌아온다", () => {
    useAppStore.getState().openModal({ kind: "settings" });
    const { rerender } = render(<SettingsDialog />);

    const tabName = (t: HTMLElement) => t.textContent;
    expect(screen.getAllByRole("tab").map(tabName)).toEqual([
      "일반",
      "소리·음성",
      "시스템",
      "제어",
    ]);

    // 기본 탭 = 일반. 다른 탭 항목은 아직 DOM에 없다.
    expect(screen.getByRole("tab", { name: "일반" }).getAttribute("aria-selected")).toBe("true");
    expect(screen.queryByLabelText(/타건음/)).toBeNull();
    expect(screen.queryByLabelText(/데스크톱 마스코트/)).toBeNull();
    expect(screen.queryByLabelText(/CLI 제어/)).toBeNull();

    openTab("소리·음성");
    expect(screen.getByRole("tab", { name: "소리·음성" }).getAttribute("aria-selected")).toBe(
      "true",
    );
    expect(screen.getByRole("tab", { name: "일반" }).getAttribute("aria-selected")).toBe("false");
    expect(screen.getByLabelText(/타건음/)).toBeTruthy();
    expect(screen.getByLabelText(/알림 대사 읽어주기/)).toBeTruthy();
    // 일반 탭 항목(관찰)은 사라진다.
    expect(screen.queryByLabelText(/에이전트 관찰/)).toBeNull();

    openTab("시스템");
    expect(screen.getByLabelText(/데스크톱 마스코트/)).toBeTruthy();
    expect(screen.getByRole("combobox", { name: /터미널 색상/ })).toBeTruthy();
    expect(screen.queryByLabelText(/타건음/)).toBeNull();

    openTab("제어");
    expect(screen.getByLabelText(/CLI 제어/)).toBeTruthy();
    expect(screen.queryByLabelText(/데스크톱 마스코트/)).toBeNull();

    // 닫으면 본체가 언마운트되므로 탭 상태도 사라진다.
    useAppStore.getState().closeModal();
    rerender(<SettingsDialog />);
    useAppStore.getState().openModal({ kind: "settings" });
    rerender(<SettingsDialog />);
    expect(screen.getByRole("tab", { name: "일반" }).getAttribute("aria-selected")).toBe("true");
    expect(screen.queryByLabelText(/CLI 제어/)).toBeNull();
  });

  // 요약 모델 오버라이드는 "지금 고른 요약기"의 두 칸만 보여준다 — 네 provider의
  // 여덟 칸을 한꺼번에 띄우면 어느 값이 쓰이는지 알 수 없다.
  it("요약 모델 오버라이드는 선택된 요약기의 두 칸만 보이고 그 값이 저장된다", () => {
    useAppStore.getState().hydrateSettings(
      { ...useAppStore.getState().appSettings, summaryProvider: "codex" },
      false,
    );
    useAppStore.getState().openModal({ kind: "settings" });
    render(<SettingsDialog />);

    // placeholder = 비웠을 때 실제로 쓰이는 백엔드 기본 모델.
    const light = screen.getByPlaceholderText("gpt-5.4-mini") as HTMLInputElement;
    const heavy = screen.getByPlaceholderText("gpt-5.4") as HTMLInputElement;
    expect(light.value).toBe("");
    expect(screen.queryByPlaceholderText("haiku")).toBeNull();

    fireEvent.change(light, { target: { value: "gpt-5.4-nano" } });
    fireEvent.change(heavy, { target: { value: "gpt-5.4-pro" } });

    const models = useAppStore.getState().appSettings.summaryModels;
    expect(models.codex).toEqual({ light: "gpt-5.4-nano", heavy: "gpt-5.4-pro" });
    // 다른 provider의 칸은 건드리지 않는다.
    expect(models.claude).toEqual({ light: "", heavy: "" });
  });

  it("닫기 버튼 클릭 시 closeModal을 부른다", () => {
    useAppStore.getState().openModal({ kind: "settings" });

    render(<SettingsDialog />);
    fireEvent.click(screen.getByRole("button", { name: "닫기" }));

    expect(useAppStore.getState().modal).toEqual({ kind: "none" });
  });

  it("백드롭 클릭 시 closeModal을 부른다", () => {
    useAppStore.getState().openModal({ kind: "settings" });

    const { container } = render(<SettingsDialog />);
    const backdrop = container.querySelector(".modal-backdrop");
    expect(backdrop).toBeTruthy();
    fireEvent.mouseDown(backdrop as Element, { button: 0 });

    expect(useAppStore.getState().modal).toEqual({ kind: "none" });
  });
});
