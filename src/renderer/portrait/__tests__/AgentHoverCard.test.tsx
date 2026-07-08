// @vitest-environment jsdom
//
// src/renderer/portrait/__tests__/AgentHoverCard.test.tsx
//
// AgentHoverCard의 폴백 프리뷰가 프로필의 archetype을 반영하는지 검증한다
// (버그: generateSpritePreview(seed)만 호출해 항상 human으로 렌더).
//
// `../../office/gen/characterFactory`는 jsdom이 canvas 2d 컨텍스트를 구현하지
// 않으므로 mock한다 (AgentTabStrip/ProfileDialog 테스트와 동일한 이유).
// `../../ipc/sessionBridge`는 tauriApi를 끌어오므로 hover 콜백만 중계하는
// 가짜 officeBus로 대체한다.
import { act, cleanup, render } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { useAppStore } from "../../store/appStore";
import type { AgentProfile } from "../../store/types";

const generateSpritePreview = vi.fn(
  (..._args: unknown[]) => "data:image/png;base64,stub"
);
vi.mock("../../office/gen/characterFactory", () => ({
  generateSpritePreview: (...args: unknown[]) => generateSpritePreview(...args),
}));

type HoverCb = (agentId: string | null, x: number, y: number) => void;
const hoverCbs = new Set<HoverCb>();
vi.mock("../../ipc/sessionBridge", () => ({
  officeBus: {
    onAgentHoverChanged: (cb: HoverCb) => {
      hoverCbs.add(cb);
      return () => hoverCbs.delete(cb);
    },
  },
}));

const { AgentHoverCard } = await import("../AgentHoverCard");

function mkProfile(id: string, overrides: Partial<AgentProfile> = {}): AgentProfile {
  return {
    id,
    name: `Agent ${id}`,
    role: "eng",
    note: "",
    seed: id,
    createdAt: Date.now(),
    deskIndex: 0,
    ...overrides,
  };
}

const initialState = useAppStore.getState();

beforeEach(() => {
  useAppStore.setState(initialState, true);
  generateSpritePreview.mockClear();
  hoverCbs.clear();
  vi.useFakeTimers();
});

afterEach(() => {
  cleanup();
  vi.useRealTimers();
});

/** 호버 이벤트 방출 후 SHOW_DELAY_MS(150ms)를 넘겨 카드를 띄운다. */
function hoverAgent(agentId: string) {
  act(() => {
    hoverCbs.forEach((cb) => cb(agentId, 40, 40));
  });
  act(() => {
    vi.advanceTimersByTime(200);
  });
}

describe("폴백 프리뷰의 archetype 반영", () => {
  it("non-human archetype 프로필의 폴백 프리뷰는 해당 archetype으로 생성된다", () => {
    useAppStore.getState().addAgent(mkProfile("a1", { archetype: "orc" }));

    const { getByAltText } = render(<AgentHoverCard />);
    hoverAgent("a1");

    // 카드가 실제로 떴는지(폴백 이미지 사용) 먼저 확인.
    const img = getByAltText("Agent a1") as HTMLImageElement;
    expect(img.src).toBe("data:image/png;base64,stub");

    // 월드와 동일하게 resolveArchetype(profile.archetype, seed) 결과 전달.
    expect(generateSpritePreview).toHaveBeenCalledWith(
      "a1",
      6,
      undefined,
      undefined,
      "orc"
    );
  });

  it("archetype 미지정 프로필은 human으로 폴백된다", () => {
    useAppStore.getState().addAgent(mkProfile("a1"));

    render(<AgentHoverCard />);
    hoverAgent("a1");

    expect(generateSpritePreview).toHaveBeenCalledWith(
      "a1",
      6,
      undefined,
      undefined,
      "human"
    );
  });
});
