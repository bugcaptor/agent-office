// @vitest-environment jsdom
//
// src/renderer/settings/__tests__/ModelPicker.test.tsx
//
// 모델 고르기 콤보박스(kbm #2fc). 지키려는 계약:
//  - 목록은 **펼칠 때** 조회한다(설정창을 열기만 하면 네트워크·CLI를 안 건드림).
//  - 목록은 힌트일 뿐 강제가 아니다 — 사용자가 친 값은 절대 교정되지 않는다.
//  - 조회는 provider당 세션 1회. 새로고침만 그 캐시를 버린다.
//  - provider가 바뀌면 이전 서비스의 목록이 남아 보이면 안 된다.
import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const listProviderModels = vi.fn<(p: string) => Promise<string[]>>();

vi.mock("../../ipc/tauriApi", () => ({
  tauriApi: { listProviderModels: (p: string) => listProviderModels(p) },
}));

import { ModelPicker, filterModels } from "../ModelPicker";
import { MODEL_PRESETS, resetModelCatalogCache } from "../modelCatalog";
import { SUMMARY_DEFAULT_MODELS } from "../SummarySection";

function options(): string[] {
  return screen.queryAllByRole("option").map((o) => o.textContent ?? "");
}

function open() {
  fireEvent.click(screen.getByRole("button", { name: "모델 목록" }));
}

beforeEach(() => {
  resetModelCatalogCache();
  listProviderModels.mockReset().mockResolvedValue([]);
});

afterEach(() => cleanup());

describe("filterModels", () => {
  it("공백으로 끊은 조각이 모두 들어 있어야 후보다(순서 무관·대소문자 무시)", () => {
    const models = ["anthropic/claude-haiku-4.5", "openai/gpt-5.4-mini", "google/gemini-2.5-pro"];
    expect(filterModels(models, "")).toEqual(models);
    expect(filterModels(models, "HAIKU")).toEqual(["anthropic/claude-haiku-4.5"]);
    expect(filterModels(models, "4.5 claude")).toEqual(["anthropic/claude-haiku-4.5"]);
    expect(filterModels(models, "없는모델")).toEqual([]);
  });
});

describe("ModelPicker", () => {
  it("펼치기 전에는 조회하지 않는다", () => {
    render(<ModelPicker provider="openrouter" value="" onChange={() => {}} ariaLabel="모델" />);
    expect(listProviderModels).not.toHaveBeenCalled();
    expect(options()).toEqual([]);
  });

  it("펼치면 그 서비스의 목록을 조회해 프리셋 뒤에 붙인다", async () => {
    listProviderModels.mockResolvedValue(["zzz/live-model"]);
    render(<ModelPicker provider="openrouter" value="" onChange={() => {}} ariaLabel="모델" />);

    open();
    await waitFor(() => expect(listProviderModels).toHaveBeenCalledWith("openrouter"));
    await waitFor(() =>
      expect(options()).toEqual([...MODEL_PRESETS.openrouter, "zzz/live-model"]),
    );
  });

  it("타이핑하면 입력값 그대로 올라가고 후보는 그 값으로 좁혀진다", async () => {
    listProviderModels.mockResolvedValue(["zzz/live-model"]);
    const onChange = vi.fn();
    const { rerender } = render(
      <ModelPicker provider="openrouter" value="" onChange={onChange} ariaLabel="모델" />,
    );

    const input = screen.getByRole("combobox");
    fireEvent.change(input, { target: { value: "live" } });
    // 값은 어디까지나 부모가 소유한다 — 픽커가 목록 값으로 바꿔치지 않는다.
    expect(onChange).toHaveBeenCalledWith("live");

    rerender(
      <ModelPicker provider="openrouter" value="live" onChange={onChange} ariaLabel="모델" />,
    );
    await waitFor(() => expect(options()).toEqual(["zzz/live-model"]));
  });

  it("↓와 Enter로 고르면 그 모델 id가 올라간다", async () => {
    const onChange = vi.fn();
    render(<ModelPicker provider="claude" value="" onChange={onChange} ariaLabel="모델" />);

    const input = screen.getByRole("combobox") as HTMLInputElement;
    input.focus(); // 실제로 키를 치는 상황과 같게 — 포커스가 곧 펼치기다
    fireEvent.keyDown(input, { key: "ArrowDown" }); // 하이라이트를 첫 후보로
    await waitFor(() => expect(options().length).toBeGreaterThan(1));
    fireEvent.keyDown(input, { key: "ArrowDown" }); // 두 번째 후보로
    fireEvent.keyDown(input, { key: "Enter" });

    expect(onChange).toHaveBeenCalledWith(MODEL_PRESETS.claude[1]);
    // 고르면 목록은 접힌다.
    expect(options()).toEqual([]);
  });

  it("Esc는 목록만 접는다(설정 다이얼로그로 새어 나가지 않게)", async () => {
    const onEscape = vi.fn();
    render(
      <div onKeyDown={onEscape}>
        <ModelPicker provider="claude" value="" onChange={() => {}} ariaLabel="모델" />
      </div>,
    );

    open();
    await waitFor(() => expect(options().length).toBeGreaterThan(0));
    fireEvent.keyDown(screen.getByRole("combobox"), { key: "Escape" });
    expect(options()).toEqual([]);
    expect(onEscape).not.toHaveBeenCalled();
  });

  it("라이브 소스가 없는 서비스는 프리셋만 조용히 보여준다", async () => {
    render(<ModelPicker provider="codex" value="" onChange={() => {}} ariaLabel="모델" />);
    open();
    await waitFor(() => expect(options()).toEqual(MODEL_PRESETS.codex));
    // 빈 응답은 실패가 아니다 — 실패 안내가 뜨면 안 된다.
    expect(screen.queryByText(/불러오지 못했습니다/)).toBeNull();
  });

  it("조회 실패는 안내 한 줄로 갈음하고 프리셋으로 강등한다", async () => {
    listProviderModels.mockRejectedValue(new Error("offline"));
    render(<ModelPicker provider="opencode" value="" onChange={() => {}} ariaLabel="모델" />);

    open();
    await waitFor(() => expect(screen.getByText(/직접 적어도 됩니다/)).toBeTruthy());
    expect(options()).toEqual(MODEL_PRESETS.opencode);
    expect(screen.queryByText(/offline/)).toBeNull();
  });

  it("새로고침만 provider 캐시를 버리고 다시 조회한다", async () => {
    render(<ModelPicker provider="openrouter" value="" onChange={() => {}} ariaLabel="모델" />);

    open();
    await waitFor(() => expect(listProviderModels).toHaveBeenCalledTimes(1));
    // 접었다 펴는 것만으로는 다시 두드리지 않는다.
    open();
    open();
    await waitFor(() => expect(options().length).toBeGreaterThan(0));
    expect(listProviderModels).toHaveBeenCalledTimes(1);

    listProviderModels.mockResolvedValue(["zzz/fresh-model"]);
    fireEvent.click(screen.getByText("새로고침"));
    await waitFor(() => expect(listProviderModels).toHaveBeenCalledTimes(2));
    await waitFor(() => expect(options()).toContain("zzz/fresh-model"));
  });

  it("provider가 바뀌면 이전 서비스의 목록을 그대로 보여주지 않는다", async () => {
    listProviderModels.mockImplementation(async (p) =>
      p === "openrouter" ? ["or/only"] : ["oc/only"],
    );
    const { rerender } = render(
      <ModelPicker provider="openrouter" value="" onChange={() => {}} ariaLabel="모델" />,
    );

    open();
    await waitFor(() => expect(options()).toContain("or/only"));

    rerender(<ModelPicker provider="opencode" value="" onChange={() => {}} ariaLabel="모델" />);
    await waitFor(() => expect(options()).toContain("oc/only"));
    expect(options()).not.toContain("or/only");
  });
});

// 프리셋은 라이브 조회가 없거나 실패할 때 유일하게 남는 목록이다. 요약기가
// 실제로 쓰는 기본 모델이 거기 없으면 "지금 쓰이는 모델"을 목록에서 고를 수
// 없다 — 모델 세대가 넘어갈 때마다 실제로 어긋나 온 곳이라 테스트로 고정한다.
describe("MODEL_PRESETS", () => {
  it("요약기 기본 모델을 모두 담는다", () => {
    for (const [provider, defaults] of Object.entries(SUMMARY_DEFAULT_MODELS)) {
      const presets = MODEL_PRESETS[provider as keyof typeof MODEL_PRESETS];
      expect(presets, provider).toContain(defaults.light);
      expect(presets, provider).toContain(defaults.heavy);
    }
  });

  it("provider마다 중복 없는 비어 있지 않은 목록이다", () => {
    for (const [provider, presets] of Object.entries(MODEL_PRESETS)) {
      expect(presets.length, provider).toBeGreaterThan(0);
      expect(new Set(presets).size, provider).toBe(presets.length);
    }
  });
});
