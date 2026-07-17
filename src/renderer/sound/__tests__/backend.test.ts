// createWebAudioBackend의 "상시 복구" 검증 — WKWebView는 장시간 유휴·OS
// 오디오 인터럽트·출력장치 변경 시 AudioContext를 suspended/interrupted로
// 돌린다. WebKit은 사용자 제스처 없이 부른 resume()을 신뢰성 있게 이행하지
// 않으므로, 재생 시점에 resume을 시도하되 그래도 살아나지 않으면 컨텍스트를
// 통째로 재생성(rebuild)해 복구한다. 여기서는 페이크 AudioContext를 주입해:
//  - 재생/상태전이/장치변경/입력마다 resume이 다시 시도되는지
//  - resume이 상태를 못 살리거나(stay) 영원히 pending(hang)이면 재생성되는지
//  - 재생성이 볼륨을 보존하고, keep-alive를 재시작하며, 디바운스되는지
//  - dispose 후엔 아무것도 안 하는지
// 를 확인한다.
import { describe, expect, it, vi } from "vitest";
import { createWebAudioBackend } from "../backend";

function param() {
  return {
    value: 0,
    setValueAtTime: vi.fn(),
    setTargetAtTime: vi.fn(),
    exponentialRampToValueAtTime: vi.fn(),
  };
}

function node() {
  return {
    connect: (dest: unknown) => dest, // Web Audio connect는 대상 노드를 반환(체이닝)
    gain: param(),
    frequency: param(),
    Q: param(),
    threshold: param(),
    knee: param(),
    ratio: param(),
    attack: param(),
    release: param(),
    playbackRate: param(),
    buffer: null as unknown,
    type: "",
    start: vi.fn(),
    stop: vi.fn(),
  };
}

// WebKit은 표준에 없는 "interrupted" 상태를 실제로 낸다(TS DOM lib 미포함).
type FakeState = AudioContextState | "interrupted";
// resume()의 세 거동: 살린다 / resolve하되 상태 유지 / 영원히 pending.
type ResumeMode = "run" | "stay" | "hang";

class FakeAudioContext {
  state: FakeState;
  resumeMode: ResumeMode;
  currentTime = 0;
  sampleRate = 44100;
  destination = node();
  resumeCalls = 0;
  gains: ReturnType<typeof node>[] = []; // 생성 순서대로 — gains[0]이 bus
  startedLoops = 0; // keep-alive용 loop 소스 start 횟수
  private listeners = new Map<string, Set<EventListener>>();

  constructor(opts: { state?: FakeState; resumeMode?: ResumeMode } = {}) {
    this.state = opts.state ?? "running";
    this.resumeMode = opts.resumeMode ?? "run";
  }

  createDynamicsCompressor() {
    return node();
  }
  createGain() {
    const g = node();
    this.gains.push(g);
    return g;
  }
  createBiquadFilter() {
    return node();
  }
  createOscillator() {
    return node();
  }
  createBufferSource() {
    const self = this;
    return {
      ...node(),
      loop: false,
      start(this: { loop: boolean }) {
        if (this.loop) self.startedLoops++;
      },
    };
  }
  createBuffer() {
    return { getChannelData: () => new Float32Array(8) };
  }
  decodeAudioData() {
    return Promise.reject(new Error("no decode in test"));
  }
  resume() {
    this.resumeCalls++;
    if (this.resumeMode === "run") {
      this.state = "running";
      this.emit("statechange");
      return Promise.resolve();
    }
    if (this.resumeMode === "stay") return Promise.resolve(); // resolve하지만 상태 유지
    return new Promise<void>(() => {}); // hang — 영원히 pending
  }
  close() {
    this.state = "closed";
    return Promise.resolve();
  }
  addEventListener(type: string, cb: EventListener) {
    (this.listeners.get(type) ?? this.listeners.set(type, new Set()).get(type)!).add(cb);
  }
  removeEventListener(type: string, cb: EventListener) {
    this.listeners.get(type)?.delete(cb);
  }
  emit(type: string) {
    for (const cb of this.listeners.get(type) ?? []) cb(new Event(type));
  }
  /** 테스트용: OS 인터럽트/출력장치 변경으로 컨텍스트가 죽는 상황 모사. */
  drop(to: FakeState = "interrupted") {
    this.state = to;
    this.emit("statechange");
  }
}

class FakeTarget {
  private listeners = new Map<string, Set<EventListener>>();
  addEventListener(type: string, cb: EventListener) {
    (this.listeners.get(type) ?? this.listeners.set(type, new Set()).get(type)!).add(cb);
  }
  removeEventListener(type: string, cb: EventListener) {
    this.listeners.get(type)?.delete(cb);
  }
  emit(type: string) {
    for (const cb of this.listeners.get(type) ?? []) cb(new Event(type));
  }
}

/** 마이크로태스크를 충분히 흘려 async revive/rebuild 체인을 정착시킨다. */
async function flush() {
  for (let i = 0; i < 20; i++) await Promise.resolve();
}

interface Cfg {
  /** 인덱스별 컨텍스트 옵션. 부족하면 마지막 항목을 반복 적용. */
  perContext?: { state?: FakeState; resumeMode?: ResumeMode }[];
  now?: () => number;
  delay?: (ms: number) => Promise<void>;
}

function setup(cfg: Cfg = {}) {
  const contexts: FakeAudioContext[] = [];
  const per = cfg.perContext ?? [];
  const createContext = () => {
    const opts = per[contexts.length] ?? per[per.length - 1] ?? {};
    const c = new FakeAudioContext(opts);
    contexts.push(c);
    return c;
  };
  const resumeTarget = new FakeTarget();
  const mediaDevices = new FakeTarget();
  const backend = createWebAudioBackend({
    createContext: createContext as unknown as () => AudioContext,
    resumeTarget: resumeTarget as unknown as EventTarget,
    mediaDevices: mediaDevices as unknown as EventTarget,
    now: cfg.now,
    delay: cfg.delay ?? (() => Promise.resolve()), // 기본: 타임아웃 즉시 만료
  })!;
  return { contexts, resumeTarget, mediaDevices, backend, get ctx() {
    return contexts[0];
  } };
}

describe("createWebAudioBackend 상시 복구", () => {
  it("컨텍스트가 죽은 뒤 재생 시도 시 resume을 부른다", () => {
    const { ctx, backend } = setup();
    ctx.state = "interrupted"; // 이벤트 없이 조용히 죽은 상태
    const before = ctx.resumeCalls;
    backend.playClicks("a1", 3);
    expect(ctx.resumeCalls).toBeGreaterThan(before);
  });

  it("running 상태에서는 재생이 resume을 부르지 않는다(무익한 호출 방지)", () => {
    const { ctx, backend } = setup();
    ctx.resumeCalls = 0;
    ctx.state = "running";
    backend.playClicks("a1", 3);
    backend.playDing();
    expect(ctx.resumeCalls).toBe(0);
  });

  it("상태 전이(interrupted/suspended)를 감지해 스스로 재개한다", () => {
    const { ctx } = setup();
    ctx.resumeCalls = 0;
    ctx.drop("interrupted");
    expect(ctx.resumeCalls).toBeGreaterThan(0);
  });

  it("출력장치 변경(devicechange) 시 재개를 시도한다", () => {
    const { ctx, mediaDevices } = setup();
    ctx.state = "interrupted";
    ctx.resumeCalls = 0;
    mediaDevices.emit("devicechange");
    expect(ctx.resumeCalls).toBeGreaterThan(0);
  });

  it("사용자 입력(pointerdown/keydown)으로도 재개를 시도한다", () => {
    const { ctx, resumeTarget } = setup();
    ctx.state = "suspended";
    ctx.resumeCalls = 0;
    resumeTarget.emit("pointerdown");
    expect(ctx.resumeCalls).toBeGreaterThan(0);
  });

  it("dispose 후에는 이벤트로 재개하지 않는다", () => {
    const { ctx, mediaDevices, resumeTarget, backend } = setup();
    backend.dispose();
    ctx.state = "interrupted";
    ctx.resumeCalls = 0;
    mediaDevices.emit("devicechange");
    resumeTarget.emit("pointerdown");
    expect(ctx.resumeCalls).toBe(0);
  });
});

describe("createWebAudioBackend 컨텍스트 재생성(rebuild)", () => {
  it("resume이 상태를 못 살리면(stay) 컨텍스트를 재생성한다", async () => {
    const { contexts, backend } = setup({
      perContext: [
        { state: "interrupted", resumeMode: "stay" },
        { state: "running", resumeMode: "run" },
      ],
      now: () => 0,
    });
    backend.playClicks("a1", 3);
    await flush();
    expect(contexts.length).toBe(2);
    expect(contexts[1].state).toBe("running");
  });

  it("resume이 영원히 pending(hang)이면 타임아웃 후 재생성한다", async () => {
    const { contexts, backend } = setup({
      perContext: [
        { state: "interrupted", resumeMode: "hang" },
        { state: "running", resumeMode: "run" },
      ],
      now: () => 0,
    });
    backend.playClicks("a1", 3);
    await flush();
    expect(contexts.length).toBe(2);
  });

  it("running 상태에서는 재생성하지 않는다(불필요한 재조립 방지)", async () => {
    const { contexts, backend } = setup();
    backend.playClicks("a1", 3);
    backend.playDing();
    await flush();
    expect(contexts.length).toBe(1);
  });

  it("재생성 시 사용자 볼륨을 보존한다", async () => {
    const { contexts, backend } = setup({
      perContext: [{ state: "interrupted", resumeMode: "stay" }],
      now: () => 0,
    });
    backend.setVolume(0.8); // 청감 보정 제곱 → 0.64
    backend.playClicks("a1", 1);
    await flush();
    expect(contexts.length).toBe(2);
    expect(contexts[1].gains[0].gain.value).toBeCloseTo(0.64, 5); // 새 bus에 재적용
  });

  it("생성·재생성 양쪽에서 keep-alive 무음 루프를 시작한다", async () => {
    const { contexts, backend } = setup({
      perContext: [
        { state: "interrupted", resumeMode: "stay" },
        { state: "running", resumeMode: "run" },
      ],
      now: () => 0,
    });
    expect(contexts[0].startedLoops).toBe(1); // 최초 생성 시
    backend.playClicks("a1", 1);
    await flush();
    expect(contexts[1].startedLoops).toBe(1); // 재생성 시에도
  });

  it("죽은 상태에서 연타해도 쿨다운 안에서는 한 번만 재생성한다", async () => {
    let t = 0;
    const { contexts, backend } = setup({
      perContext: [{ state: "interrupted", resumeMode: "stay" }], // 새 컨텍스트도 계속 죽어 있음
      now: () => t,
    });
    backend.playClicks("a1", 1);
    await flush();
    const afterFirst = contexts.length;
    expect(afterFirst).toBe(2); // 첫 재생성

    backend.playClicks("a1", 1);
    await flush();
    expect(contexts.length).toBe(afterFirst); // 쿨다운 내 — 재생성 없음

    t += 10_000; // 쿨다운 경과
    backend.playClicks("a1", 1);
    await flush();
    expect(contexts.length).toBe(afterFirst + 1); // 다시 재생성
  });

  it("dispose 후에는 재생성하지 않는다", async () => {
    const { contexts, backend } = setup({
      perContext: [{ state: "interrupted", resumeMode: "stay" }],
      now: () => 0,
    });
    backend.dispose();
    backend.playClicks("a1", 1);
    await flush();
    expect(contexts.length).toBe(1);
  });
});
