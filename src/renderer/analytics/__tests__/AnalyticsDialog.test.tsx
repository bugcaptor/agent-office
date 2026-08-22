// @vitest-environment jsdom
//
// src/renderer/analytics/__tests__/AnalyticsDialog.test.tsx
//
// 셀프 게이팅(analytics 모달에서만 렌더), 열릴 때 loadSessionEvents 호출 →
// 로딩→집계 표시, 빈/오류+재시도 상태, 기간 전환 재조회. tauriApi는 mock.
import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type {
  AgentProfile,
  SessionEventKind,
  SessionEventRecord,
  SessionEventTokens,
} from "@shared/types";
import { useAppStore } from "../../store/appStore";

const loadSessionEvents = vi.fn();
vi.mock("../../ipc/tauriApi", () => ({
  tauriApi: {
    loadSessionEvents: (...args: unknown[]) => loadSessionEvents(...args),
  },
}));

const { AnalyticsDialog } = await import("../AnalyticsDialog");

const initialState = useAppStore.getState();

let seq = 0;
function ev(
  kind: SessionEventKind,
  at: number,
  agentId = "a1",
  tokens?: SessionEventTokens,
): SessionEventRecord {
  return {
    schemaVersion: 1,
    runId: "r",
    seq: seq++,
    at,
    agentId,
    sessionId: "s1",
    kind,
    ...(tokens ? { tokens } : {}),
  };
}

function profile(id: string, name: string): AgentProfile {
  return { id, name, role: "", seed: `seed-${id}`, createdAt: 0, deskIndex: 0 };
}

beforeEach(() => {
  useAppStore.setState(initialState, true);
  loadSessionEvents.mockReset();
});

afterEach(() => cleanup());

describe("AnalyticsDialog", () => {
  it("modal이 analytics가 아니면 아무것도 렌더하지 않는다", () => {
    const { container } = render(<AnalyticsDialog />);
    expect(container.firstChild).toBeNull();
    expect(loadSessionEvents).not.toHaveBeenCalled();
  });

  it("열릴 때 loadSessionEvents를 호출하고 집계 표를 보여준다", async () => {
    const now = Date.now();
    loadSessionEvents.mockResolvedValue([
      ev("prompt", now - 600_000),
      ev("stop", now - 300_000),
    ]);
    useAppStore.setState({ agents: { a1: profile("a1", "Ada") } });
    useAppStore.getState().openModal({ kind: "analytics" });

    render(<AnalyticsDialog />);

    expect(screen.getByText("세션 활동 분석")).toBeTruthy();
    await waitFor(() => expect(loadSessionEvents).toHaveBeenCalledTimes(1));
    // 요약 표에 프로필 이름과 작업시간(5분)이 나온다.
    await waitFor(() => expect(screen.getByText("Ada")).toBeTruthy());
    // "5분"은 차트 y축 눈금에도 나오므로 표 셀로 좁혀 확인한다.
    expect(screen.getByRole("cell", { name: "5분" })).toBeTruthy();
    // 차트 SVG도 그려진다.
    expect(document.querySelector("svg.analytics-chart")).toBeTruthy();
  });

  it("stop에 실린 토큰과 추정 비용을 셀에 보여준다", async () => {
    const now = Date.now();
    loadSessionEvents.mockResolvedValue([
      ev("prompt", now - 600_000),
      ev("stop", now - 300_000, "a1", {
        input: 12_000,
        output: 3_000,
        model: "claude-opus-5",
      }),
    ]);
    useAppStore.setState({ agents: { a1: profile("a1", "Ada") } });
    useAppStore.getState().openModal({ kind: "analytics" });

    render(<AnalyticsDialog />);

    // 토큰 = 입력+출력 = 15,000 → "15.0K", 비용 = (12000*5 + 3000*25)/1e6 = $0.135
    await waitFor(() => expect(screen.getByRole("cell", { name: "15.0K" })).toBeTruthy());
    expect(screen.getByRole("cell", { name: "$0.135" })).toBeTruthy();
    expect(screen.getByRole("cell", { name: "15.0K" }).getAttribute("title")).toBe(
      "입력 12.0K · 출력 3.0K",
    );
    expect(screen.getByText(/공개 API 요율 환산 추정치/)).toBeTruthy();
  });

  it("토큰이 없는(과거) 기록은 토큰·비용 셀에 —를 보여준다", async () => {
    const now = Date.now();
    loadSessionEvents.mockResolvedValue([
      ev("prompt", now - 600_000),
      ev("stop", now - 300_000),
    ]);
    useAppStore.setState({ agents: { a1: profile("a1", "Ada") } });
    useAppStore.getState().openModal({ kind: "analytics" });

    render(<AnalyticsDialog />);

    await waitFor(() => expect(screen.getByText("Ada")).toBeTruthy());
    expect(screen.getAllByRole("cell", { name: "—" })).toHaveLength(2);
  });

  it("기간 내 활동이 없으면 빈 상태 문구를 보여준다", async () => {
    loadSessionEvents.mockResolvedValue([]);
    useAppStore.getState().openModal({ kind: "analytics" });

    render(<AnalyticsDialog />);

    await waitFor(() =>
      expect(screen.getByText("이 기간에 기록된 세션 활동이 없습니다.")).toBeTruthy(),
    );
  });

  it("로드 실패 시 오류 문구와 재시도 버튼을 보여주고, 재시도가 재조회한다", async () => {
    loadSessionEvents.mockRejectedValueOnce(new Error("boom"));
    loadSessionEvents.mockResolvedValueOnce([]);
    useAppStore.getState().openModal({ kind: "analytics" });

    render(<AnalyticsDialog />);

    const retry = await screen.findByRole("button", { name: "재시도" });
    expect(screen.getByText("세션 이벤트를 불러오지 못했습니다.")).toBeTruthy();

    fireEvent.click(retry);
    await waitFor(() => expect(loadSessionEvents).toHaveBeenCalledTimes(2));
    await waitFor(() =>
      expect(screen.getByText("이 기간에 기록된 세션 활동이 없습니다.")).toBeTruthy(),
    );
  });

  it("기간 전환 시 loadSessionEvents를 다시 호출한다", async () => {
    loadSessionEvents.mockResolvedValue([]);
    useAppStore.getState().openModal({ kind: "analytics" });

    render(<AnalyticsDialog />);
    await waitFor(() => expect(loadSessionEvents).toHaveBeenCalledTimes(1));

    fireEvent.click(screen.getByRole("button", { name: "30일" }));
    await waitFor(() => expect(loadSessionEvents).toHaveBeenCalledTimes(2));

    // 30일 범위가 7일보다 더 넓은 fromAt으로 조회된다.
    const first = loadSessionEvents.mock.calls[0] as [number, number];
    const second = loadSessionEvents.mock.calls[1] as [number, number];
    expect(second[0]).toBeLessThan(first[0]);
  });

  it("이전 기간의 늦은 응답이 최신 결과를 덮지 않는다", async () => {
    const now = Date.now();
    let resolveFirst!: (v: SessionEventRecord[]) => void;
    let resolveSecond!: (v: SessionEventRecord[]) => void;
    const first = new Promise<SessionEventRecord[]>((r) => {
      resolveFirst = r;
    });
    const second = new Promise<SessionEventRecord[]>((r) => {
      resolveSecond = r;
    });
    loadSessionEvents.mockReturnValueOnce(first).mockReturnValueOnce(second);

    useAppStore.setState({ agents: { a1: profile("a1", "Ada"), a2: profile("a2", "Bob") } });
    useAppStore.getState().openModal({ kind: "analytics" });
    render(<AnalyticsDialog />);

    // 7일 조회 시작 → 응답 전에 30일로 전환해 두 번째 조회 시작.
    await waitFor(() => expect(loadSessionEvents).toHaveBeenCalledTimes(1));
    fireEvent.click(screen.getByRole("button", { name: "30일" }));
    await waitFor(() => expect(loadSessionEvents).toHaveBeenCalledTimes(2));

    // 최신(2번째) 응답이 먼저 도착 → Bob 표시.
    await act(async () => {
      resolveSecond([ev("prompt", now - 600_000, "a2"), ev("stop", now - 300_000, "a2")]);
      await second;
    });
    await waitFor(() => expect(screen.getByText("Bob")).toBeTruthy());

    // 이전(1번째) 응답이 늦게 도착 → 세대 가드로 무시(Ada 안 뜨고 Bob 유지).
    await act(async () => {
      resolveFirst([ev("prompt", now - 600_000, "a1"), ev("stop", now - 300_000, "a1")]);
      await first;
    });
    expect(screen.queryByText("Ada")).toBeNull();
    expect(screen.getByText("Bob")).toBeTruthy();
  });

  it("닫기 버튼이 closeModal을 부른다", async () => {
    loadSessionEvents.mockResolvedValue([]);
    useAppStore.getState().openModal({ kind: "analytics" });

    render(<AnalyticsDialog />);
    fireEvent.click(screen.getByRole("button", { name: "닫기" }));

    expect(useAppStore.getState().modal).toEqual({ kind: "none" });
  });
});
