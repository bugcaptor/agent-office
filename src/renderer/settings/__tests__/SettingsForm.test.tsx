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
  it("두 토글과 설명 문구를 렌더하고 변경을 콜백한다", () => {
    const onChange = vi.fn();
    render(
      <SettingsForm
        value={{ claudeCliEnabled: false, claudeHooksEnabled: false }}
        onChange={onChange}
      />,
    );
    const cli = screen.getByRole("checkbox", { name: /라벨 요약/ }) as HTMLInputElement;
    const hooks = screen.getByRole("checkbox", { name: /알림·시간측정/ }) as HTMLInputElement;
    expect(cli.checked).toBe(false);
    expect(hooks.checked).toBe(false);
    fireEvent.click(hooks);
    expect(onChange).toHaveBeenCalledWith({ claudeHooksEnabled: true });
    expect(screen.getByText(/구독 크레딧/)).toBeTruthy();
    expect(screen.getByText(/127\.0\.0\.1/)).toBeTruthy();
  });

  it("체크된 값을 그대로 반영한다", () => {
    const onChange = vi.fn();
    render(
      <SettingsForm
        value={{ claudeCliEnabled: true, claudeHooksEnabled: true }}
        onChange={onChange}
      />,
    );
    expect((screen.getByRole("checkbox", { name: /라벨 요약/ }) as HTMLInputElement).checked).toBe(true);
    expect((screen.getByRole("checkbox", { name: /알림·시간측정/ }) as HTMLInputElement).checked).toBe(true);
  });
});
