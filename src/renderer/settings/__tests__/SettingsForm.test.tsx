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
        value={{
          summarizerEnabled: false,
          summaryProvider: "claude",
          summarizerToolCalls: false,
          observerEnabled: false,
        }}
        onChange={onChange}
      />,
    );

    expect((screen.getByRole("checkbox", { name: /라벨 요약/ }) as HTMLInputElement).checked).toBe(false);
    expect((screen.getByRole("radio", { name: "Claude" }) as HTMLInputElement).checked).toBe(true);
    expect((screen.getByRole("radio", { name: "Codex" }) as HTMLInputElement).checked).toBe(false);
    expect((screen.getByRole("checkbox", { name: /에이전트 관찰/ }) as HTMLInputElement).checked).toBe(false);

    fireEvent.click(screen.getByRole("radio", { name: "Codex" }));
    fireEvent.click(screen.getByRole("checkbox", { name: /에이전트 관찰/ }));
    expect(onChange).toHaveBeenCalledWith({ summaryProvider: "codex" });
    expect(onChange).toHaveBeenCalledWith({ observerEnabled: true });
  });

  it("요약이 꺼져 있어도 provider를 미리 선택할 수 있다", () => {
    const onChange = vi.fn();
    render(
      <SettingsForm
        value={{
          summarizerEnabled: false,
          summaryProvider: "codex",
          summarizerToolCalls: false,
          observerEnabled: false,
        }}
        onChange={onChange}
      />,
    );
    expect((screen.getByRole("radio", { name: "Codex" }) as HTMLInputElement).disabled).toBe(false);
  });

  it("실험 툴 체크박스: 요약 OFF거나 Codex면 비활성, Claude+요약 ON이면 토글된다", () => {
    const onChange = vi.fn();
    const { rerender } = render(
      <SettingsForm
        value={{
          summarizerEnabled: false,
          summaryProvider: "claude",
          summarizerToolCalls: false,
          observerEnabled: false,
        }}
        onChange={onChange}
      />,
    );
    const probe = () =>
      screen.getByRole("checkbox", { name: /작업 폴더 훑어보기/ }) as HTMLInputElement;
    // 요약 OFF → 비활성.
    expect(probe().disabled).toBe(true);

    // 요약 ON + Codex → 여전히 비활성(Claude 전용).
    rerender(
      <SettingsForm
        value={{
          summarizerEnabled: true,
          summaryProvider: "codex",
          summarizerToolCalls: false,
          observerEnabled: false,
        }}
        onChange={onChange}
      />,
    );
    expect(probe().disabled).toBe(true);

    // 요약 ON + Claude → 활성, 클릭 시 패치.
    rerender(
      <SettingsForm
        value={{
          summarizerEnabled: true,
          summaryProvider: "claude",
          summarizerToolCalls: false,
          observerEnabled: false,
        }}
        onChange={onChange}
      />,
    );
    expect(probe().disabled).toBe(false);
    fireEvent.click(probe());
    expect(onChange).toHaveBeenCalledWith({ summarizerToolCalls: true });
  });
});
