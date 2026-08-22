// @vitest-environment jsdom
//
// src/renderer/settings/__tests__/SettingsForm.test.tsx
//
// SettingsForm은 상태를 소유하지 않는 순수 제어 컴포넌트 — value/onChange만
// 검증한다. FirstRunDialog/SettingsDialog가 각자 다른 배선(로컬 state vs
// 스토어 직결)으로 이 폼을 감싸는 조합은 각자의 테스트에서 검증한다.
import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { SettingsForm } from "../SettingsForm";

afterEach(() => cleanup());

describe("SettingsForm", () => {
  it("요약 토글, provider 선택, 공통 observer 토글을 렌더한다", () => {
    const onChange = vi.fn();
    render(
      <SettingsForm
        value={{ summarizerEnabled: false, summaryProvider: "claude", diaryEnabled: false, observerEnabled: false }}
        onChange={onChange}
      />,
    );

    expect((screen.getByRole("checkbox", { name: /라벨 요약/ }) as HTMLInputElement).checked).toBe(false);
    expect((screen.getByRole("radio", { name: "Claude" }) as HTMLInputElement).checked).toBe(true);
    expect((screen.getByRole("radio", { name: "Codex" }) as HTMLInputElement).checked).toBe(false);
    expect((screen.getByRole("radio", { name: "Antigravity (agy)" }) as HTMLInputElement).checked).toBe(false);
    expect((screen.getByRole("radio", { name: "Gemini" }) as HTMLInputElement).checked).toBe(false);
    expect((screen.getByRole("radio", { name: "opencode" }) as HTMLInputElement).checked).toBe(false);
    expect((screen.getByRole("radio", { name: /OpenRouter/ }) as HTMLInputElement).checked).toBe(false);
    expect((screen.getByRole("checkbox", { name: /에이전트 관찰/ }) as HTMLInputElement).checked).toBe(false);

    fireEvent.click(screen.getByRole("radio", { name: "Codex" }));
    fireEvent.click(screen.getByRole("checkbox", { name: /에이전트 관찰/ }));
    expect(onChange).toHaveBeenCalledWith({ summaryProvider: "codex" });
    expect(onChange).toHaveBeenCalledWith({ observerEnabled: true });

    fireEvent.click(screen.getByRole("radio", { name: "Antigravity (agy)" }));
    fireEvent.click(screen.getByRole("radio", { name: "Gemini" }));
    expect(onChange).toHaveBeenCalledWith({ summaryProvider: "agy" });
    expect(onChange).toHaveBeenCalledWith({ summaryProvider: "gemini" });

    fireEvent.click(screen.getByRole("radio", { name: "opencode" }));
    expect(onChange).toHaveBeenCalledWith({ summaryProvider: "opencode" });

    // CLI가 아닌 HTTP 경로도 같은 라디오 그룹에서 고른다.
    fireEvent.click(screen.getByRole("radio", { name: /OpenRouter/ }));
    expect(onChange).toHaveBeenCalledWith({ summaryProvider: "openrouter" });
  });

  it("요약이 꺼져 있어도 provider를 미리 선택할 수 있다", () => {
    const onChange = vi.fn();
    render(
      <SettingsForm
        value={{ summarizerEnabled: false, summaryProvider: "codex", diaryEnabled: false, observerEnabled: false }}
        onChange={onChange}
      />,
    );
    expect((screen.getByRole("radio", { name: "Codex" }) as HTMLInputElement).disabled).toBe(false);
  });
});
