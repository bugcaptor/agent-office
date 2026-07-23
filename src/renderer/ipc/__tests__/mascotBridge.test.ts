// mascotBridge(이슈 #72) — 스토어 → 마스코트 창 상태 푸시.
//
// 실제 Tauri 이벤트/커맨드는 주입점(MascotBridgeIo)으로 대체하고, 스토어만
// 진짜를 쓴다. 검증 대상: 선정 결과 반영, 중복 방출 억제, 창 표시 전환,
// linger 지연 숨김, 설정 OFF 즉시 숨김, ready 핸드셰이크, 클릭 릴레이.
import { beforeEach, afterEach, describe, expect, it, vi } from "vitest";
import type { AgentProfile } from "../../store/types";

vi.mock("../tauriApi", () => ({
  tauriApi: { setAppSettings: vi.fn().mockResolvedValue(undefined), appendSessionTurn: vi.fn() },
}));
// sessionBridge는 officeBus만 쓰이며 여기선 io로 대체되지만, 모듈 자체가
// tauriApi를 붙잡고 스토어를 구독하므로 가벼운 대역으로 바꾼다.
vi.mock("../sessionBridge", () => ({ officeBus: { emitAgentClicked: vi.fn() } }));

import { useAppStore } from "../../store/appStore";
import { installMascotBridge, MASCOT_HIDE_LINGER_MS, type MascotBridgeIo } from "../mascotBridge";
import { HIDDEN_MASCOT_STATE, type MascotState } from "../../mascot/protocol";

const initialState = useAppStore.getState();

function mkProfile(overrides: Partial<AgentProfile> = {}): AgentProfile {
  return {
    id: "a1",
    name: "테스터",
    role: "backend",
    note: "",
    seed: "seed-a1",
    createdAt: 1_000,
    deskIndex: 0,
    ...overrides,
  };
}

interface Harness {
  io: MascotBridgeIo;
  states: MascotState[];
  visibles: boolean[];
  fireReady: () => void;
  fireOpenTerminal: (agentId: string) => void;
  opened: string[];
  last(): MascotState | undefined;
}

function harness(): Harness {
  const states: MascotState[] = [];
  const visibles: boolean[] = [];
  const opened: string[] = [];
  let readyCb: (() => void) | null = null;
  let clickCb: ((agentId: string) => void) | null = null;
  return {
    states,
    visibles,
    opened,
    last: () => states[states.length - 1],
    fireReady: () => readyCb?.(),
    fireOpenTerminal: (id) => clickCb?.(id),
    io: {
      emitState: (s) => void states.push(s),
      setVisible: (v) => void visibles.push(v),
      onMascotReady: (cb) => {
        readyCb = cb;
        return () => {
          readyCb = null;
        };
      },
      onOpenTerminal: (cb) => {
        clickCb = cb;
        return () => {
          clickCb = null;
        };
      },
      openTerminal: (id) => void opened.push(id),
    },
  };
}

/** 마스코트 켜기 + 에이전트 1명 등록. 브리지 설치 전에 부른다. */
function seed(agents: AgentProfile[] = [mkProfile()]) {
  useAppStore.setState({
    appSettings: { ...useAppStore.getState().appSettings, mascotEnabled: true },
    agents: Object.fromEntries(agents.map((a) => [a.id, a])),
    agentOrder: agents.map((a) => a.id),
  });
}

/** 스토어에 알림 1건을 직접 넣는다(pushNotification의 억제 규칙 우회). */
function setPending(agentId: string | null) {
  useAppStore.setState({
    notifications: agentId
      ? [
          {
            id: "n1",
            agentId,
            type: "info" as const,
            message: "m",
            excerpt: "m",
            createdAt: 1,
          },
        ]
      : [],
  });
}

function setWorking(agentId: string, turnStartedAt: number | null) {
  useAppStore.setState({
    timeTracking: {
      ...useAppStore.getState().timeTracking,
      [agentId]: {
        phase: "working",
        turnStartedAt,
        waitingSince: null,
        waitedInTurnMs: 0,
        totalMs: 0,
        workedMs: 0,
        waitedMs: 0,
        turns: 0,
      },
    },
  });
}

function setIdle(agentId: string) {
  useAppStore.setState({
    timeTracking: {
      ...useAppStore.getState().timeTracking,
      [agentId]: {
        phase: "idle",
        turnStartedAt: null,
        waitingSince: null,
        waitedInTurnMs: 0,
        totalMs: 0,
        workedMs: 0,
        waitedMs: 0,
        turns: 0,
      },
    },
  });
}

beforeEach(() => {
  useAppStore.setState(initialState, true);
  vi.useFakeTimers();
});

afterEach(() => {
  vi.useRealTimers();
});

describe("installMascotBridge", () => {
  it("설정이 꺼져 있으면 활동이 있어도 숨김 상태를 유지한다", () => {
    seed();
    useAppStore.setState({
      appSettings: { ...useAppStore.getState().appSettings, mascotEnabled: false },
    });
    const h = harness();
    const off = installMascotBridge(h.io);
    setWorking("a1", 10);
    expect(h.states).toEqual([]); // 초기 상태(HIDDEN)와 같아 방출 자체가 없다
    expect(h.visibles).toEqual([]);
    off();
  });

  it("working 캐릭터가 생기면 프로필을 실어 방출하고 창을 띄운다", () => {
    seed([mkProfile({ id: "a1", seed: "s1", archetype: "cat", spriteUpdatedAt: 7 })]);
    const h = harness();
    const off = installMascotBridge(h.io);
    setWorking("a1", 100);
    expect(h.last()).toEqual({
      visible: true,
      agentId: "a1",
      name: "테스터",
      seed: "s1",
      archetype: "cat",
      spriteUpdatedAt: 7,
      hasPending: false,
      working: true,
    });
    expect(h.visibles).toEqual([true]);
    off();
  });

  it("같은 상태로 수렴하는 변화는 다시 방출하지 않는다", () => {
    seed();
    const h = harness();
    const off = installMascotBridge(h.io);
    setWorking("a1", 100);
    const n = h.states.length;
    // 관계없는 슬라이스 변경 → 재계산은 돌지만 결과가 같다.
    useAppStore.setState({ agents: { ...useAppStore.getState().agents } });
    expect(h.states.length).toBe(n);
    off();
  });

  it("알림이 붙으면 hasPending이 켜지고, 사라지면 꺼진다", () => {
    seed();
    const h = harness();
    const off = installMascotBridge(h.io);
    setPending("a1");
    expect(h.last()).toMatchObject({ visible: true, agentId: "a1", hasPending: true });
    setPending(null);
    expect(h.last()).toMatchObject({ hasPending: false });
    off();
  });

  it("활동이 끊겨도 linger 동안은 그 캐릭터를 유지하다가 숨긴다", () => {
    seed();
    const h = harness();
    const off = installMascotBridge(h.io);
    setWorking("a1", 100);
    setIdle("a1");
    // 아직 보이는 상태 — 조용해지기만 한다.
    expect(h.last()).toMatchObject({ visible: true, agentId: "a1", working: false });
    expect(h.visibles).toEqual([true]);

    vi.advanceTimersByTime(MASCOT_HIDE_LINGER_MS - 1);
    expect(h.last()?.visible).toBe(true);

    vi.advanceTimersByTime(1);
    expect(h.last()).toEqual(HIDDEN_MASCOT_STATE);
    expect(h.visibles).toEqual([true, false]);
    off();
  });

  it("linger 중 활동이 돌아오면 숨기지 않는다", () => {
    seed();
    const h = harness();
    const off = installMascotBridge(h.io);
    setWorking("a1", 100);
    setIdle("a1");
    vi.advanceTimersByTime(MASCOT_HIDE_LINGER_MS - 100);
    setWorking("a1", 200);
    vi.advanceTimersByTime(MASCOT_HIDE_LINGER_MS);
    expect(h.last()).toMatchObject({ visible: true, agentId: "a1", working: true });
    expect(h.visibles).toEqual([true]);
    off();
  });

  it("설정을 끄면 linger 없이 즉시 숨긴다", () => {
    seed();
    const h = harness();
    const off = installMascotBridge(h.io);
    setWorking("a1", 100);
    useAppStore.setState({
      appSettings: { ...useAppStore.getState().appSettings, mascotEnabled: false },
    });
    expect(h.last()).toEqual(HIDDEN_MASCOT_STATE);
    expect(h.visibles).toEqual([true, false]);
    off();
  });

  it("ready 핸드셰이크는 dedupe를 무시하고 현재 상태를 재방출한다", () => {
    seed();
    const h = harness();
    const off = installMascotBridge(h.io);
    setWorking("a1", 100);
    const before = h.states.length;
    h.fireReady();
    expect(h.states.length).toBe(before + 1);
    expect(h.last()).toMatchObject({ visible: true, agentId: "a1" });
    expect(h.visibles[h.visibles.length - 1]).toBe(true);
    off();
  });

  it("마스코트 클릭은 officeBus 경로(openTerminal)로 넘긴다", () => {
    seed();
    const h = harness();
    const off = installMascotBridge(h.io);
    h.fireOpenTerminal("a1");
    expect(h.opened).toEqual(["a1"]);
    off();
  });

  it("해제하면 더 이상 방출하지 않는다", () => {
    seed();
    const h = harness();
    const off = installMascotBridge(h.io);
    off();
    setWorking("a1", 100);
    expect(h.states).toEqual([]);
  });
});
