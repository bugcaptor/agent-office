// @vitest-environment jsdom
// src/renderer/labels/__tests__/TaskLabelLayer.test.tsx
//
// 라벨 레이어: 표시 조건(세션 starting|running), 1/2줄 내용
// (요약 우선, 원문 폴백), phase 클래스, bus 앵커 → transform 직접 갱신.
import { beforeEach, describe, expect, it } from "vitest";
import { act, render } from "@testing-library/react";
import { createMockOfficeBus } from "../../office/bus";
import { useAppStore } from "../../store/appStore";
import { initialTurnState } from "../../timeline/turnReducer";
import { TaskLabelLayer } from "../TaskLabelLayer";
import type { AgentProfile } from "@shared/types";

function agent(id: string, cwd?: string): AgentProfile {
  return { id, name: id, role: "", note: "", seed: "s", createdAt: 0, deskIndex: 0, cwd };
}

function seedStore(opts: {
  status?: "idle" | "starting" | "running" | "exited" | "disposed";
  label?: Partial<import("../../store/types").AgentTaskLabel>;
  phase?: "idle" | "working" | "waiting";
}) {
  useAppStore.setState({
    agents: { a1: agent("a1", "/Users/me/dev/agent-office") },
    agentOrder: ["a1"],
    sessions: {
      a1: { agentId: "a1", status: opts.status ?? "running", cols: 80, rows: 24, lastActivityAt: 0 },
    },
    taskLabels: opts.label ? { a1: { sessionId: "s1", ...opts.label } } : {},
    timeTracking: { a1: { ...initialTurnState(), phase: opts.phase ?? "working" } },
  });
}

beforeEach(() => {
  useAppStore.setState({ agents: {}, agentOrder: [], sessions: {}, taskLabels: {}, timeTracking: {} });
});

describe("TaskLabelLayer", () => {
  it("요약이 있으면 '프로젝트명 · 목표'와 현재 명령 요약을 보여준다", () => {
    seedStore({ label: { firstPromptText: "버그 고쳐줘", latestPromptText: "버그 고쳐줘", goal: "버그 수정", currentSummary: "버그 고치는 중" } });
    const { container } = render(<TaskLabelLayer bus={createMockOfficeBus()} />);
    const el = container.querySelector(".task-label")!;
    expect(el.querySelector(".task-label-line1")!.textContent).toBe("agent-office · 버그 수정");
    expect(el.querySelector(".task-label-line2")!.textContent).toBe("버그 고치는 중");
    expect(el.className).toContain("phase-working");
  });

  it("2줄은 실황(assistant > tool) > 지시 요약 > 프롬프트 순으로 우선한다", () => {
    // 모두 있을 때: assistant 내레이션이 이긴다.
    seedStore({
      label: {
        firstPromptText: "버그 고쳐줘",
        latestPromptText: "버그 고쳐줘",
        currentSummary: "버그 고치는 중",
        latestToolText: "Bash: npm test",
        latestAssistantText: "원인을 좁히는 중",
      },
    });
    const { container } = render(<TaskLabelLayer bus={createMockOfficeBus()} />);
    expect(container.querySelector(".task-label-line2")!.textContent).toBe("원인을 좁히는 중");

    // assistant 없으면 도구 요약.
    act(() => {
      seedStore({
        label: {
          latestPromptText: "버그 고쳐줘",
          currentSummary: "버그 고치는 중",
          latestToolText: "Bash: npm test",
        },
      });
    });
    expect(container.querySelector(".task-label-line2")!.textContent).toBe("Bash: npm test");

    // 실황이 없으면 지시 요약.
    act(() => {
      seedStore({ label: { latestPromptText: "버그 고쳐줘", currentSummary: "버그 고치는 중" } });
    });
    expect(container.querySelector(".task-label-line2")!.textContent).toBe("버그 고치는 중");
  });

  it("요약이 없으면 원문 첫 줄 절단으로 폴백한다", () => {
    seedStore({ label: { firstPromptText: "버그를 고쳐줘\n상세", latestPromptText: "테스트 추가해줘" } });
    const { container } = render(<TaskLabelLayer bus={createMockOfficeBus()} />);
    expect(container.querySelector(".task-label-line1")!.textContent).toBe("agent-office · 버그를 고쳐줘");
    expect(container.querySelector(".task-label-line2")!.textContent).toBe("테스트 추가해줘");
  });

  it("프롬프트가 아직 없으면 프로젝트명만 표시한다", () => {
    seedStore({ label: undefined });
    const { container } = render(<TaskLabelLayer bus={createMockOfficeBus()} />);
    expect(container.querySelector(".task-label-line1")!.textContent).toBe("agent-office");
    expect(container.querySelector(".task-label-line2")).toBeNull();
  });

  it("세션이 running/starting이 아니면 라벨을 만들지 않는다", () => {
    seedStore({ status: "idle", label: { firstPromptText: "x", latestPromptText: "x" } });
    const { container } = render(<TaskLabelLayer bus={createMockOfficeBus()} />);
    expect(container.querySelector(".task-label")).toBeNull();
  });

  it("waiting phase가 클래스에 반영된다", () => {
    seedStore({ phase: "waiting", label: { firstPromptText: "x", latestPromptText: "x" } });
    const { container } = render(<TaskLabelLayer bus={createMockOfficeBus()} />);
    expect(container.querySelector(".task-label")!.className).toContain("phase-waiting");
  });

  it("bus 앵커 이벤트가 transform/visibility를 직접 갱신한다 (앵커 없는 라벨은 숨김)", () => {
    seedStore({ label: { firstPromptText: "x", latestPromptText: "x" } });
    const bus = createMockOfficeBus();
    const { container } = render(<TaskLabelLayer bus={bus} />);
    const el = container.querySelector(".task-label") as HTMLElement;
    expect(el.style.visibility).toBe(""); // CSS 기본 hidden(스타일시트) — 인라인은 아직 없음

    act(() => {
      bus.emitLabelAnchorsChanged(new Map([["a1", { x: 100.4, y: 60.6 }]]));
    });
    expect(el.style.visibility).toBe("visible");
    expect(el.style.transform).toBe("translate(100px, 61px) translate(-50%, -100%)");

    act(() => {
      bus.emitLabelAnchorsChanged(new Map());
    });
    expect(el.style.visibility).toBe("hidden");
  });
});
