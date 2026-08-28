// @vitest-environment jsdom
//
// src/renderer/usage/__tests__/UsageFloat.test.tsx
//
// filled 뷰 모드 사용량 플로팅(이슈 #69). 표시 조건(뷰 모드 + 터미널 열림 +
// 표시할 provider 존재)과 클릭 시 상세 모달을 여는지만 확인한다 — 문구·색
// 규칙은 usageView 순수 함수 테스트와 UsageDialog/UsageWidget이 이미 덮는다.
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterAll, afterEach, beforeEach, describe, expect, it } from "vitest";
import { SOURCE_LANGUAGE, initI18nForTest } from "@renderer/i18n";
import type { UsageSnapshot } from "@shared/types";
import { useAppStore } from "../../store/appStore";
import { UsageFloat } from "../UsageFloat";

const initialState = useAppStore.getState();
const NOW = Date.now();

function snapshot(): UsageSnapshot {
  return {
    claude: {
      provider: "claude",
      fetchedAtMs: NOW,
      planLabel: "Max",
      windows: [
        {
          kind: "session",
          label: null,
          usedPercent: 61.4,
          resetsAtMs: NOW + 3 * 60 * 60_000,
          windowMinutes: null,
          isActive: true,
        },
      ],
    },
    codex: null,
    claudeLive: {
      outcome: "ok",
      tokenSource: null,
      detail: null,
      lastAttemptMs: NOW,
      lastSuccessMs: NOW,
      via: null,
    },
    codexLive: {
      outcome: "never_attempted",
      detail: null,
      lastAttemptMs: null,
      lastSuccessMs: null,
    },
    antigravity: null,
    antigravityLive: {
      outcome: "never_attempted",
      detail: null,
      lastAttemptMs: null,
      lastSuccessMs: null,
    },
    gemini: null,
    geminiLive: {
      outcome: "never_attempted",
      detail: null,
      lastAttemptMs: null,
      lastSuccessMs: null,
    },
  };
}

beforeEach(async () => {
  await initI18nForTest(SOURCE_LANGUAGE);
  useAppStore.setState(initialState, true);
});
afterEach(() => cleanup());
afterAll(async () => {
  await initI18nForTest(SOURCE_LANGUAGE);
});

describe("UsageFloat", () => {
  it("filled 모드 + 터미널 열림 + usage 있음 → 퍼센트가 렌더된다", () => {
    useAppStore.setState({
      usage: snapshot(),
      terminalViewMode: "filled",
      activeTerminalAgentId: "agent-1",
    });
    render(<UsageFloat />);
    expect(screen.getByText("61%")).toBeTruthy();
  });

  it("windowed 모드면 렌더하지 않는다", () => {
    useAppStore.setState({
      usage: snapshot(),
      terminalViewMode: "windowed",
      activeTerminalAgentId: "agent-1",
    });
    const { container } = render(<UsageFloat />);
    expect(container.firstChild).toBeNull();
  });

  it("터미널이 닫혀 있으면(filled 모드여도) 렌더하지 않는다", () => {
    useAppStore.setState({
      usage: snapshot(),
      terminalViewMode: "filled",
      activeTerminalAgentId: null,
    });
    const { container } = render(<UsageFloat />);
    expect(container.firstChild).toBeNull();
  });

  it("설정(usageFloatEnabled)이 false면 filled+열림+usage 있음이어도 렌더하지 않는다", () => {
    useAppStore.setState({
      appSettings: { ...useAppStore.getState().appSettings, usageFloatEnabled: false },
      usage: snapshot(),
      terminalViewMode: "filled",
      activeTerminalAgentId: "agent-1",
    });
    const { container } = render(<UsageFloat />);
    expect(container.firstChild).toBeNull();
  });

  it("클릭하면 openModal이 usage 상세로 호출된다", () => {
    useAppStore.setState({
      usage: snapshot(),
      terminalViewMode: "filled",
      activeTerminalAgentId: "agent-1",
    });
    render(<UsageFloat />);
    fireEvent.click(screen.getByRole("button"));
    expect(useAppStore.getState().modal).toEqual({ kind: "usage" });
  });
});
