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

function setWaiting(agentId: string, waitingSince: number | null) {
  useAppStore.setState({
    timeTracking: {
      ...useAppStore.getState().timeTracking,
      [agentId]: {
        phase: "waiting",
        turnStartedAt: null,
        waitingSince,
        waitedInTurnMs: 0,
        totalMs: 0,
        workedMs: 0,
        waitedMs: 0,
        turns: 0,
      },
    },
  });
}

/** 신호등 설정 3필드만 골라 갱신한다(나머지 appSettings는 그대로). */
function setLightsSettings(patch: {
  mascotLightsMode?: "off" | "agents" | "projects";
  mascotLightsVertical?: boolean;
  mascotLightsProjects?: string[];
}) {
  useAppStore.setState({
    appSettings: { ...useAppStore.getState().appSettings, ...patch },
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
      colors: null,
      spriteUpdatedAt: 7,
      hasPending: false,
      working: true,
      lights: [],
      lightsVertical: false,
      lightsFace: "sprite",
      lightsWide: false,
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

describe("installMascotBridge · 신호등(lights)", () => {
  it("스프라이트 대상이 없어도 lights가 있으면 창이 뜬다(agentId:null)", () => {
    seed(); // a1, mascotEnabled=true
    setLightsSettings({ mascotLightsMode: "agents" });
    const h = harness();
    const off = installMascotBridge(h.io);
    // waiting은 pickMascotTarget(스프라이트)에서는 제외되지만(결정 1),
    // 신호등에서는 attention이다 — 알림을 지운 뒤에도 남는 차이.
    setWaiting("a1", 50);
    expect(h.last()).toMatchObject({
      visible: true,
      agentId: null,
      lights: [
        {
          id: "a1",
          label: "테스터",
          state: "attention",
          clickAgentId: "a1",
          avatar: {
            agentId: "a1",
            seed: "seed-a1",
            archetype: null,
            colors: null,
            spriteUpdatedAt: null,
          },
        },
      ],
    });
    off();
  });

  it("스프라이트 linger가 다 돼도 lights가 남아 있으면 창이 계속 떠 있는다(스프라이트만 접힌다)", () => {
    seed([mkProfile({ id: "a1" }), mkProfile({ id: "b1", seed: "s-b1" })]);
    setLightsSettings({ mascotLightsMode: "agents" });
    const h = harness();
    const off = installMascotBridge(h.io);

    setWorking("a1", 100); // 스프라이트 대상 + a1 신호등 GREEN
    setWaiting("b1", 10); // b1 신호등 YELLOW(스프라이트 pick에는 관여하지 않음)
    expect(h.last()).toMatchObject({ visible: true, agentId: "a1" });

    setIdle("a1"); // 스프라이트 대상 소멸 → linger 시작. b1은 여전히 attention.
    expect(h.last()).toMatchObject({ visible: true, agentId: "a1", working: false });

    vi.advanceTimersByTime(MASCOT_HIDE_LINGER_MS);
    const after = h.last();
    expect(after).toMatchObject({ visible: true, agentId: null });
    expect(after?.lights).toEqual([
      {
        id: "b1",
        label: "테스터",
        tooltip: "테스터",
        state: "attention",
        clickAgentId: "b1",
        avatar: {
          agentId: "b1",
          seed: "s-b1",
          archetype: null,
          colors: null,
          spriteUpdatedAt: null,
          portraitUpdatedAt: null,
        },
      },
    ]);
    // 스프라이트 자체는 사라져야 한다(창은 lights 때문에 떠 있을 뿐).
    expect(h.visibles).toEqual([true]); // visible이 계속 true라 show/hide 전환이 없다
    off();
  });

  it("a1 스프라이트가 linger 중이어도 b1의 램프는 idle 전환 즉시 꺼진다(여운 금지, T2)", () => {
    seed([mkProfile({ id: "a1" }), mkProfile({ id: "b1", seed: "s-b1" })]);
    setLightsSettings({ mascotLightsMode: "agents" });
    const h = harness();
    const off = installMascotBridge(h.io);

    setWorking("a1", 100); // 스프라이트 pick = a1
    // b1은 waiting으로 둔다 — working이면 a1이 idle 되는 순간 pickMascotTarget이
    // 스프라이트를 b1로 넘겨 linger 자체가 안 걸린다(waiting은 스프라이트
    // pick에서 제외되므로 신호등에만 attention으로 반영된다, 결정 1).
    setWaiting("b1", 10);
    expect(h.last()?.lights.map((l) => l.id)).toEqual(["a1", "b1"]);

    setIdle("a1"); // a1 스프라이트 linger 시작 — 창은 계속 a1을 조용히 보여준다.
    expect(h.last()).toMatchObject({ agentId: "a1", working: false });
    expect(h.last()?.lights.map((l) => l.id)).toEqual(["b1"]); // a1은 신호등에서 즉시 제외

    setIdle("b1"); // b1도 idle — linger 없이 곧바로 lights에서 빠져야 한다.
    expect(h.last()?.lights).toEqual([]);
    // linger가 아직 안 끝났으므로 스프라이트는 여전히 a1(조용한 모습)을 보여준다.
    expect(h.last()).toMatchObject({ agentId: "a1", working: false, visible: true });
    off();
  });

  it("lights가 없으면(기능 꺼짐) linger 만료 시 기존대로 완전히 숨는다", () => {
    seed(); // mascotLightsMode 기본값 off
    const h = harness();
    const off = installMascotBridge(h.io);
    setWorking("a1", 100);
    setIdle("a1");
    vi.advanceTimersByTime(MASCOT_HIDE_LINGER_MS);
    expect(h.last()).toEqual(HIDDEN_MASCOT_STATE);
    off();
  });

  it("mascotLightsVertical 변경만으로도 재방출된다", () => {
    seed();
    const h = harness();
    const off = installMascotBridge(h.io);
    const before = h.states.length;
    setLightsSettings({ mascotLightsVertical: true });
    expect(h.states.length).toBe(before + 1);
    expect(h.last()?.lightsVertical).toBe(true);
    off();
  });

  it("mascotLightsFace 변경만으로도 재방출되고, 칸의 avatar가 portraitUpdatedAt을 나른다", () => {
    seed([mkProfile({ id: "a1", portraitUpdatedAt: 42 })]);
    setLightsSettings({ mascotLightsMode: "agents" });
    setWorking("a1", 100);
    const h = harness();
    const off = installMascotBridge(h.io);
    expect(h.last()?.lightsFace).toBe("sprite");
    expect(h.last()?.lights[0]?.avatar).toMatchObject({ portraitUpdatedAt: 42 });

    const before = h.states.length;
    useAppStore.setState({
      appSettings: { ...useAppStore.getState().appSettings, mascotLightsFace: "portrait" },
    });
    expect(h.states.length).toBe(before + 1);
    expect(h.last()?.lightsFace).toBe("portrait");
    off();
  });

  it("mascotLightsMode/mascotLightsProjects 변경이 신호등 칸에 즉시 반영된다", () => {
    seed([mkProfile({ id: "a1", cwd: "/dev/proj" })]);
    setWorking("a1", 10);
    const h = harness();
    const off = installMascotBridge(h.io);
    expect(h.last()?.lights).toEqual([]); // 기본 off

    setLightsSettings({ mascotLightsMode: "projects", mascotLightsProjects: ["/dev/proj"] });
    expect(h.last()?.lights).toEqual([
      {
        id: "/dev/proj",
        label: "proj",
        tooltip: "테스터 · proj",
        state: "working",
        clickAgentId: "a1",
        // 칸의 얼굴 = 대표 에이전트의 스프라이트 좌표(§6 개정).
        avatar: {
          agentId: "a1",
          seed: "seed-a1",
          archetype: null,
          colors: null,
          spriteUpdatedAt: null,
          portraitUpdatedAt: null,
        },
      },
    ]);
    off();
  });

  it("mascotEnabled=false면 lights가 있어도 즉시 완전히 숨긴다", () => {
    seed();
    setLightsSettings({ mascotLightsMode: "agents" });
    const h = harness();
    const off = installMascotBridge(h.io);
    setWaiting("a1", 50);
    expect(h.last()?.lights.length).toBeGreaterThan(0);

    useAppStore.setState({
      appSettings: { ...useAppStore.getState().appSettings, mascotEnabled: false },
    });
    expect(h.last()).toEqual(HIDDEN_MASCOT_STATE);
    off();
  });

  it("taskLabels가 바뀌어도 신호등 결과가 같으면 재방출하지 않는다", () => {
    seed([mkProfile({ id: "a1", cwd: "/dev/proj" })]);
    setLightsSettings({ mascotLightsMode: "projects", mascotLightsProjects: ["/dev/proj"] });
    setWorking("a1", 10);
    const h = harness();
    const off = installMascotBridge(h.io);
    const before = h.states.length;
    // 소속·상태에 영향 없는 taskLabels 갱신(예: 프롬프트 텍스트만 변경).
    useAppStore.setState({
      taskLabels: { ...useAppStore.getState().taskLabels, a1: { sessionId: "s1" } },
    });
    expect(h.states.length).toBe(before); // 결과가 같아 dedupe로 흡수된다
    off();
  });
});
