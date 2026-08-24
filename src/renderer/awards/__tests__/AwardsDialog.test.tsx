// @vitest-environment jsdom
//
// src/renderer/awards/__tests__/AwardsDialog.test.tsx
//
// TDD for `AwardsDialog`. `useAppStore`/`useAwardsStore`는 실제 zustand
// 스토어를 그대로 쓰고(ProfileDialog.test.tsx와 같은 관례), 스토어의 비동기
// 액션(load/ensureFinalized/provisionalWinner/generateSpeechFor)만 매 테스트
// `vi.fn()`으로 갈아 끼워 네트워크성 왕복 없이 결정적으로 검증한다.
// `tauriApi.loadAwardPortrait`만 모듈 경계에서 목으로 대체한다(portraitCache의
// pngBase64ToDataUrl과 함께 초상 스냅샷 로드 경로가 실제로 배선돼 있는지는
// 여기서 다루지 않는다 — hasPortrait:false 픽스처만 쓴다).
//
// 이 저장소엔 jest-dom 매처가 설치돼 있지 않다 — toBeNull/toBeTruthy와
// `.disabled` 프로퍼티 직접 확인이 관례다(ProfileDialog.test.tsx 참고).
import { act, cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type {
  AgentProfile,
  AwardRecord,
  AwardStanding,
  AwardStats,
  AwardWinner,
} from "@shared/types";

const loadAwardPortrait = vi.fn().mockResolvedValue(null);
vi.mock("../../ipc/tauriApi", () => ({
  tauriApi: { loadAwardPortrait: (...args: unknown[]) => loadAwardPortrait(...args) },
}));

import { useAppStore } from "../../store/appStore";
import { useAwardsStore } from "../awardsStore";
import { AwardsDialog } from "../AwardsDialog";

function mkProfile(overrides: Partial<AgentProfile> = {}): AgentProfile {
  return {
    id: "a1",
    name: "Ada Lovelace",
    role: "backend",
    seed: "seed-a1",
    createdAt: 0,
    deskIndex: 0,
    ...overrides,
  };
}

function mkStats(overrides: Partial<AwardStats> = {}): AwardStats {
  return {
    workedMs: 43 * 3_600_000,
    turns: 12,
    toolEvents: 34,
    activeDays: 20,
    tokensIn: 120_000,
    tokensOut: 45_000,
    costUsd: 3.21,
    ...overrides,
  };
}

function mkWinner(overrides: Partial<AwardWinner> = {}): AwardWinner {
  return {
    agentId: "a1",
    name: "Ada Lovelace",
    role: "backend",
    hasPortrait: false,
    stats: mkStats(),
    ...overrides,
  };
}

function mkStanding(overrides: Partial<AwardStanding> = {}): AwardStanding {
  return {
    agentId: "a1",
    name: "Ada Lovelace",
    workedMs: 43 * 3_600_000,
    turns: 12,
    activeDays: 20,
    ...overrides,
  };
}

function mkRecord(month: string, overrides: Partial<AwardRecord> = {}): AwardRecord {
  const winner = overrides.winner === undefined ? mkWinner() : overrides.winner;
  return {
    month,
    decidedAt: 0,
    rulesVersion: 1,
    winner,
    leaderboard: winner
      ? [
          mkStanding(),
          mkStanding({ agentId: "a2", name: "Grace Hopper", workedMs: 10_000_000, turns: 5, activeDays: 8 }),
        ]
      : [],
    speeches: [],
    ...overrides,
  };
}

/** 스토어를 결정적인 fixture로 채운다. load/ensureFinalized/provisionalWinner/
 * generateSpeechFor는 기본적으로 아무 것도 하지 않는 스텁 — 각 테스트가 필요한
 * 것만 patch로 덮어쓴다. */
function seedAwardsStore(patch: Partial<ReturnType<typeof useAwardsStore.getState>>) {
  useAwardsStore.setState({
    loaded: true,
    finalizing: false,
    generating: {},
    error: undefined,
    load: vi.fn().mockResolvedValue(undefined),
    ensureFinalized: vi.fn().mockResolvedValue(undefined),
    provisionalWinner: vi.fn().mockResolvedValue(null),
    generateSpeechFor: vi.fn().mockResolvedValue(undefined),
    ...patch,
  });
}

function openDialog() {
  act(() => {
    useAppStore.getState().openModal({ kind: "awards" });
  });
  return render(<AwardsDialog />);
}

const initialAppState = useAppStore.getState();
const initialAwardsState = useAwardsStore.getState();

beforeEach(() => {
  useAppStore.setState(initialAppState, true);
  useAwardsStore.setState(initialAwardsState, true);
  loadAwardPortrait.mockClear();
  loadAwardPortrait.mockResolvedValue(null);
});

afterEach(() => cleanup());

describe("visibility", () => {
  it("modal.kind가 awards가 아니면 아무것도 렌더하지 않는다", () => {
    seedAwardsStore({ awards: [mkRecord("2026-07")] });
    const { container } = render(<AwardsDialog />);
    expect(container.querySelector(".modal-backdrop")).toBeNull();
  });
});

describe("수상자가 있는 달", () => {
  it("이름·통계·순위표를 보여준다", () => {
    useAppStore.setState({ agents: { a1: mkProfile() } });
    seedAwardsStore({ awards: [mkRecord("2026-07")] });

    const { container } = openDialog();

    const left = container.querySelector(".awards-left")!;
    expect(left.textContent).toContain("Ada Lovelace");
    expect(left.textContent).toContain("backend");
    // 작업시간은 사람이 읽는 "43시간 0분" 단위.
    const right = container.querySelector(".awards-right")!;
    expect(right.textContent).toContain("43시간 0분");
    expect(right.textContent).toContain("12"); // 턴
    expect(right.textContent).toContain("34"); // 도구 호출
    expect(right.textContent).toContain("20일"); // 활동일

    const table = container.querySelector(".awards-table")!;
    expect(table.textContent).toContain("Ada Lovelace");
    expect(table.textContent).toContain("Grace Hopper");
  });

  it("통산 수상 횟수를 보여준다(awardCountFor)", () => {
    useAppStore.setState({ agents: { a1: mkProfile() } });
    seedAwardsStore({
      awards: [mkRecord("2026-05"), mkRecord("2026-06"), mkRecord("2026-07")],
    });

    const { container } = openDialog();

    expect(container.querySelector(".awards-award-count")?.textContent).toBe("3회 수상");
  });
});

describe("winner: null (빈 상태)", () => {
  it("이 달은 수상자가 없습니다 + 임계 안내를 보여준다", () => {
    seedAwardsStore({ awards: [mkRecord("2026-07", { winner: null, leaderboard: [] })] });

    openDialog();

    expect(screen.getByText("이 달은 수상자가 없습니다.")).toBeTruthy();
    expect(screen.getByText(/최소 활동/)).toBeTruthy();
  });
});

describe("기록이 아예 없음", () => {
  it("아직 시상 기록이 없습니다를 보여준다", () => {
    seedAwardsStore({ awards: [] });

    openDialog();

    expect(screen.getByText("아직 시상 기록이 없습니다.")).toBeTruthy();
  });
});

describe("월 네비게이션", () => {
  it("◀/▶로 확정된 달 사이를 이동하고 양끝에서 비활성화된다", () => {
    seedAwardsStore({
      awards: [
        mkRecord("2026-06", { winner: mkWinner({ name: "Bo" }) }),
        mkRecord("2026-07", { winner: mkWinner({ name: "Ada Lovelace" }) }),
      ],
    });

    const { container } = openDialog();

    // 기본 선택 = 최신 확정 월.
    expect(container.querySelector(".awards-left")!.textContent).toContain("Ada Lovelace");
    expect(
      (screen.getByRole("button", { name: "다음 달" }) as HTMLButtonElement).disabled
    ).toBe(true);

    fireEvent.click(screen.getByRole("button", { name: "이전 달" }));
    expect(container.querySelector(".awards-left")!.textContent).toContain("Bo");
    expect(
      (screen.getByRole("button", { name: "이전 달" }) as HTMLButtonElement).disabled
    ).toBe(true);

    fireEvent.click(screen.getByRole("button", { name: "다음 달" }));
    expect(container.querySelector(".awards-left")!.textContent).toContain("Ada Lovelace");
    expect(
      (screen.getByRole("button", { name: "다음 달" }) as HTMLButtonElement).disabled
    ).toBe(true);
  });

  it("드롭다운으로 특정 달을 바로 고를 수 있다", () => {
    seedAwardsStore({
      awards: [
        mkRecord("2026-06", { winner: mkWinner({ name: "Bo" }) }),
        mkRecord("2026-07", { winner: mkWinner({ name: "Ada Lovelace" }) }),
      ],
    });

    const { container } = openDialog();

    fireEvent.change(screen.getByLabelText("확정된 월 목록"), { target: { value: "2026-06" } });
    expect(container.querySelector(".awards-left")!.textContent).toContain("Bo");
  });
});

describe("수상 소감", () => {
  it("소감이 없으면 버튼을 보여주고 클릭 시 generateSpeechFor를 호출한다", () => {
    useAppStore.setState((s) => ({ appSettings: { ...s.appSettings, summarizerEnabled: true } }));
    useAppStore.setState({ agents: { a1: mkProfile() } });
    const generateSpeechFor = vi.fn().mockResolvedValue(undefined);
    seedAwardsStore({ awards: [mkRecord("2026-07", { speeches: [] })], generateSpeechFor });

    openDialog();

    const btn = screen.getByRole("button", { name: /수상 소감 듣기/ }) as HTMLButtonElement;
    expect(btn.disabled).toBe(false);
    fireEvent.click(btn);
    expect(generateSpeechFor).toHaveBeenCalledWith("2026-07");
  });

  it("소감이 있으면 카드 + 다시 듣기 버튼을 보여준다", () => {
    useAppStore.setState((s) => ({ appSettings: { ...s.appSettings, summarizerEnabled: true } }));
    useAppStore.setState({ agents: { a1: mkProfile() } });
    const generateSpeechFor = vi.fn().mockResolvedValue(undefined);
    seedAwardsStore({
      awards: [
        mkRecord("2026-07", {
          speeches: [{ at: 1000, provider: "claude", text: "감사합니다, 다음 달도 열심히 하겠습니다." }],
        }),
      ],
      generateSpeechFor,
    });

    openDialog();

    expect(screen.getByText("감사합니다, 다음 달도 열심히 하겠습니다.")).toBeTruthy();
    const regen = screen.getByRole("button", { name: "다시 듣기" }) as HTMLButtonElement;
    expect(regen.disabled).toBe(false);
    fireEvent.click(regen);
    expect(generateSpeechFor).toHaveBeenCalledWith("2026-07");
  });

  it("이전 소감이 있으면 접어서 보여준다", () => {
    useAppStore.setState({ agents: { a1: mkProfile() } });
    seedAwardsStore({
      awards: [
        mkRecord("2026-07", {
          speeches: [
            { at: 1000, provider: "claude", text: "첫 번째 소감" },
            { at: 2000, provider: "claude", text: "두 번째(대표) 소감" },
          ],
        }),
      ],
    });

    openDialog();

    expect(screen.getByText("두 번째(대표) 소감")).toBeTruthy();
    expect(screen.getByText("이전 소감 1개 보기")).toBeTruthy();
    expect(screen.getByText("첫 번째 소감")).toBeTruthy();
  });

  it("생성 중이면 버튼이 비활성화되고 진행 표시를 보여준다", () => {
    useAppStore.setState((s) => ({ appSettings: { ...s.appSettings, summarizerEnabled: true } }));
    useAppStore.setState({ agents: { a1: mkProfile() } });
    seedAwardsStore({
      awards: [mkRecord("2026-07", { speeches: [] })],
      generating: { "2026-07": true },
    });

    openDialog();

    const btn = screen.getByRole("button", { name: /생성하는 중/ }) as HTMLButtonElement;
    expect(btn.disabled).toBe(true);
  });

  it("요약기가 꺼져 있으면 버튼이 비활성화되고 사유 툴팁을 보여준다", () => {
    // appSettings.summarizerEnabled 기본값(false) 그대로.
    useAppStore.setState({ agents: { a1: mkProfile() } });
    seedAwardsStore({ awards: [mkRecord("2026-07", { speeches: [] })] });

    openDialog();

    const btn = screen.getByRole("button", { name: /수상 소감 듣기/ }) as HTMLButtonElement;
    expect(btn.disabled).toBe(true);
    expect(btn.title).toBe("설정에서 요약 기능을 켜면 소감을 들을 수 있습니다.");
  });

  it("수상자 프로필이 삭제됐으면 버튼이 비활성화되고 사유 툴팁을 보여준다", () => {
    useAppStore.setState((s) => ({ appSettings: { ...s.appSettings, summarizerEnabled: true } }));
    useAppStore.setState({ agents: {} }); // a1 없음 = 삭제됨
    seedAwardsStore({ awards: [mkRecord("2026-07", { speeches: [] })] });

    openDialog();

    const btn = screen.getByRole("button", { name: /수상 소감 듣기/ }) as HTMLButtonElement;
    expect(btn.disabled).toBe(true);
    expect(btn.title).toBe("수상자 캐릭터가 남아 있지 않아 소감을 들을 수 없습니다.");
  });
});

describe("진행 중인 달 배너", () => {
  it("최신 확정 월을 볼 때 잠정 선두를 보여준다", async () => {
    const provisionalWinner = vi.fn().mockResolvedValue({
      month: "2026-08",
      at: Date.now(),
      winner: { agentId: "a2", name: "Grace Hopper", deleted: false, color: "#000" },
      leaderboard: [],
    });
    seedAwardsStore({ awards: [mkRecord("2026-07")], provisionalWinner });

    openDialog();

    await waitFor(() => expect(screen.getByText(/이번 달 잠정 선두: Grace Hopper/)).toBeTruthy());
  });

  it("잠정 후보가 없으면 배너를 숨긴다", async () => {
    const provisionalWinner = vi.fn().mockResolvedValue(null);
    seedAwardsStore({ awards: [mkRecord("2026-07")], provisionalWinner });

    const { container } = openDialog();

    await waitFor(() => expect(provisionalWinner).toHaveBeenCalled());
    expect(container.querySelector(".awards-provisional-banner")).toBeNull();
  });
});
