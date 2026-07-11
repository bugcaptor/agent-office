// createWebAudioBackend의 "상시 복구" 검증 — WKWebView는 장시간 유휴·OS
// 오디오 인터럽트·출력장치 변경 시 AudioContext를 suspended/interrupted로
// 돌린다. 부팅 1회성 resume만 있으면 그 뒤로는 세션 내내 무음이 된다.
// 여기서는 페이크 AudioContext를 주입해, 재생 시점·상태 전이·장치 변경·
// 사용자 입력마다 resume이 다시 시도되는지(그리고 running일 땐 건드리지
// 않는지, dispose 후엔 멈추는지)를 확인한다.
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
// ensureRunning의 !== "running" && !== "closed" 가드는 그 상태도 잡는다.
type FakeState = AudioContextState | "interrupted";

class FakeAudioContext {
  state: FakeState = "running";
  currentTime = 0;
  sampleRate = 44100;
  destination = node();
  resumeCalls = 0;
  private listeners = new Map<string, Set<EventListener>>();

  createDynamicsCompressor() {
    return node();
  }
  createGain() {
    return node();
  }
  createBiquadFilter() {
    return node();
  }
  createOscillator() {
    return node();
  }
  createBufferSource() {
    return node();
  }
  createBuffer() {
    return { getChannelData: () => new Float32Array(8) };
  }
  decodeAudioData() {
    return Promise.reject(new Error("no decode in test"));
  }
  resume() {
    this.resumeCalls++;
    this.state = "running";
    this.emit("statechange");
    return Promise.resolve();
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

function setup() {
  const ctx = new FakeAudioContext();
  const resumeTarget = new FakeTarget();
  const mediaDevices = new FakeTarget();
  const backend = createWebAudioBackend({
    context: ctx as unknown as AudioContext,
    resumeTarget: resumeTarget as unknown as EventTarget,
    mediaDevices: mediaDevices as unknown as EventTarget,
  })!;
  return { ctx, resumeTarget, mediaDevices, backend };
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
