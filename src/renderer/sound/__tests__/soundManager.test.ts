// soundManager 조립 검증 — backend/api를 목으로 대체하고 실제 zustand
// 스토어를 조작해 구독·틱·설정 반영을 확인한다. (appStore가 import하는
// tauriApi는 appStore.test.ts와 같은 방식으로 목 처리.)
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("../../ipc/tauriApi", () => ({
  tauriApi: {
    setAppSettings: vi.fn().mockResolvedValue(undefined),
    appendSessionTurn: vi.fn(),
  },
}));

import { useAppStore } from "../../store/appStore";
import { installSoundManager, previewKeyboardSound } from "../soundManager";
import type { SoundBackend } from "../backend";
import type {
  NotificationEvent,
  NotificationSource,
  SessionStateEvent,
  TtsSpeakRequest,
} from "@shared/types";

function mockBackend(): SoundBackend & { calls: Record<string, unknown[][]> } {
  const calls: Record<string, unknown[][]> = {};
  const rec =
    (name: string) =>
    (...args: unknown[]) => {
      (calls[name] ??= []).push(args);
    };
  return {
    calls,
    playClicks: rec("playClicks") as SoundBackend["playClicks"],
    playDing: rec("playDing"),
    playSessionStart: rec("playSessionStart"),
    playSessionEnd: rec("playSessionEnd"),
    playVoice: (async (mp3: ArrayBuffer) => {
      (calls.playVoice ??= []).push([mp3.byteLength]);
    }) as SoundBackend["playVoice"],
    setVolume: rec("setVolume") as SoundBackend["setVolume"],
    dispose: rec("dispose"),
  };
}

function mockApi() {
  const dataCbs = new Map<string, (d: string, bytes: number) => void>();
  let notifCb: ((n: NotificationEvent) => void) | null = null;
  let sessionCb: ((e: SessionStateEvent) => void) | null = null;
  const dataUnsubs: string[] = [];
  const ttsCalls: TtsSpeakRequest[] = [];
  return {
    dataCbs,
    dataUnsubs,
    ttsCalls,
    emitNotification: (n: NotificationEvent) => notifCb?.(n),
    emitSession: (e: SessionStateEvent) => sessionCb?.(e),
    api: {
      onData(agentId: string, cb: (d: string, bytes: number) => void) {
        dataCbs.set(agentId, cb);
        return () => {
          dataCbs.delete(agentId);
          dataUnsubs.push(agentId);
        };
      },
      onNotification(cb: (n: NotificationEvent) => void) {
        notifCb = cb;
        return () => {
          notifCb = null;
        };
      },
      onSessionState(cb: (e: SessionStateEvent) => void) {
        sessionCb = cb;
        return () => {
          sessionCb = null;
        };
      },
      // TTS는 기본 꺼짐이라 아래 대부분의 테스트에서 호출되지 않는다. 호출되면
      // 그 자체가 회귀 신호이므로 spy로 세어둔다(전용 테스트가 검사).
      ttsSpeak: async (request: TtsSpeakRequest) => {
        ttsCalls.push(request);
        return {
          audioBase64: "AAAA",
          mimeType: "audio/mpeg",
          line: "[nervous] " + request.message,
          voiceId: "v1",
          modelId: "eleven_v3",
          cached: false,
          rewritten: true,
          rewriteVia: "api" as const,
        };
      },
    },
  };
}

import type { AgentProfile } from "../../store/types";

const AGENT: AgentProfile = {
  id: "a1",
  name: "테스트",
  role: "dev",
  seed: "seed",
  createdAt: 0,
  deskIndex: 0,
};

function notif(agentId: string, source: NotificationSource = "hook"): NotificationEvent {
  return { id: "n1", sessionId: "s1", agentId, source, message: "m", dedupKey: "k", at: 1 };
}

describe("installSoundManager", () => {
  const initial = useAppStore.getState();
  let now = 0;

  beforeEach(() => {
    vi.useFakeTimers();
    now = 0;
    useAppStore.setState(initial, true);
  });
  afterEach(() => {
    vi.useRealTimers();
  });

  function install(backend = mockBackend()) {
    const m = mockApi();
    const off = installSoundManager({
      backend,
      api: m.api,
      now: () => now,
      tickMs: 100,
    });
    return { backend, m, off };
  }

  it("에이전트 추가/제거에 따라 onData 구독을 동기화한다", () => {
    const { m, off } = install();
    useAppStore.getState().addAgent(AGENT);
    expect(m.dataCbs.has("a1")).toBe(true);
    useAppStore.getState().removeAgent("a1");
    expect(m.dataCbs.has("a1")).toBe(false);
    expect(m.dataUnsubs).toContain("a1");
    off();
  });

  it("출력이 흐르면 타이핑 시간 동안 playClicks가 호출된다", () => {
    const { backend, m, off } = install();
    useAppStore.getState().addAgent(AGENT);
    m.dataCbs.get("a1")!("x".repeat(600), 600); // 타이핑 시간 확보
    // 차분한 타속(최저 초당 3클릭)이라 첫 클릭까지 몇 틱 걸릴 수 있다
    for (let i = 0; i < 10; i++) {
      now += 100;
      vi.advanceTimersByTime(100);
    }
    expect(backend.calls.playClicks?.[0]?.[0]).toBe("a1");
    expect(backend.calls.playClicks?.[0]?.[1]).toBeGreaterThan(0);
    off();
  });

  it("에이전트의 keyboardSound 팩 id가 playClicks에 전달된다", () => {
    const { backend, m, off } = install();
    useAppStore.getState().addAgent({ ...AGENT, keyboardSound: "topre" });
    m.dataCbs.get("a1")!("x".repeat(600), 600);
    for (let i = 0; i < 10; i++) {
      now += 100;
      vi.advanceTimersByTime(100);
    }
    expect(backend.calls.playClicks?.[0]?.[2]).toBe("topre");
    off();
  });

  it("keyboardSound 미지정이면 팩 id로 undefined를 전달한다 (backend가 기본 팩 해석)", () => {
    const { backend, m, off } = install();
    useAppStore.getState().addAgent(AGENT);
    m.dataCbs.get("a1")!("x".repeat(600), 600);
    for (let i = 0; i < 10; i++) {
      now += 100;
      vi.advanceTimersByTime(100);
    }
    expect(backend.calls.playClicks?.[0]).toBeDefined();
    expect(backend.calls.playClicks?.[0]?.[2]).toBeUndefined();
    off();
  });

  it("스피너 리페인트 같은 잡음 청크는 소리를 내지 않는다", () => {
    const { backend, m, off } = install();
    useAppStore.getState().addAgent(AGENT);
    // 유효 글자가 적은 TUI 프레임이 반복돼도(대기 중 스피너) 무음이어야 한다.
    const frame = "\x1b[2K\x1b[1G✳ Deliberating… (esc to interrupt · 12s)";
    for (let i = 0; i < 10; i++) {
      m.dataCbs.get("a1")!(frame, frame.length);
      now += 100;
      vi.advanceTimersByTime(100);
    }
    expect(backend.calls.playClicks).toBeUndefined();
    off();
  });

  it("typingSoundEnabled=false면 클릭을 재생하지 않는다", () => {
    const { backend, m, off } = install();
    useAppStore.getState().addAgent(AGENT);
    useAppStore.getState().updateAppSettings({ typingSoundEnabled: false });
    m.dataCbs.get("a1")!("x".repeat(600), 600);
    now += 100;
    vi.advanceTimersByTime(100);
    expect(backend.calls.playClicks).toBeUndefined();
    off();
  });

  // ── 소리 3분할: 세 스위치는 서로를 보지 않는다 ─────────────────────
  it("타건음을 꺼도 알림 딩은 울린다(그리고 그 반대도)", () => {
    const { backend, m, off } = install();
    useAppStore.getState().addAgent(AGENT);
    useAppStore.getState().updateAppSettings({ typingSoundEnabled: false });
    m.emitNotification(notif("a1"));
    expect(backend.calls.playDing).toHaveLength(1);

    // 알림음만 끄면 딩은 멎지만 타건음은 그대로다.
    useAppStore
      .getState()
      .updateAppSettings({ typingSoundEnabled: true, notifySoundEnabled: false });
    m.emitNotification(notif("a1"));
    expect(backend.calls.playDing).toHaveLength(1);
    m.dataCbs.get("a1")!("x".repeat(600), 600);
    for (let i = 0; i < 10; i++) {
      now += 100;
      vi.advanceTimersByTime(100);
    }
    expect(backend.calls.playClicks?.length).toBeGreaterThan(0);
    off();
  });

  it("무음 모드는 타건음·세션 효과음까지 덮는 마스터다", () => {
    const { backend, m, off } = install();
    useAppStore.getState().addAgent(AGENT);
    useAppStore.getState().toggleMuted();
    m.dataCbs.get("a1")!("x".repeat(600), 600);
    for (let i = 0; i < 10; i++) {
      now += 100;
      vi.advanceTimersByTime(100);
    }
    expect(backend.calls.playClicks).toBeUndefined();
    m.emitSession({ sessionId: "s1", agentId: "a1", at: 1, state: "running" });
    expect(backend.calls.playSessionStart).toBeUndefined();
    off();
  });

  it("notifySoundEnabled=false면 세션 시작·종료 효과음도 멎는다", () => {
    const { backend, m, off } = install();
    useAppStore.getState().updateAppSettings({ notifySoundEnabled: false });
    const base = { sessionId: "s1", agentId: "a1", at: 1 } as const;
    m.emitSession({ ...base, state: "running" });
    m.emitSession({ ...base, state: "exited" });
    expect(backend.calls.playSessionStart).toBeUndefined();
    expect(backend.calls.playSessionEnd).toBeUndefined();
    off();
  });

  it("볼륨 변경이 backend.setVolume으로 전파된다", () => {
    const { backend, off } = install();
    useAppStore.getState().updateAppSettings({ soundVolume: 0.8 });
    const vols = backend.calls.setVolume!;
    expect(vols[vols.length - 1][0]).toBe(0.8);
    off();
  });

  it("알림 도착 시 딩 — 단, 무음 모드(muted)면 침묵", () => {
    const { backend, m, off } = install();
    m.emitNotification(notif("a1"));
    expect(backend.calls.playDing).toHaveLength(1);
    useAppStore.getState().toggleMuted();
    m.emitNotification(notif("a1"));
    expect(backend.calls.playDing).toHaveLength(1); // 그대로
    off();
  });

  // ── 확인 요청 대사 TTS 게이팅 ──────────────────────────────────────
  it("ttsEnabled=false(기본)면 question 알림에도 발화하지 않는다", async () => {
    const { m, off } = install();
    m.emitNotification(notif("a1"));
    await vi.advanceTimersByTimeAsync(0);
    expect(m.ttsCalls).toHaveLength(0);
    off();
  });

  it("ttsEnabled=true면 question(hook) 알림을 발화한다", async () => {
    const { backend, m, off } = install();
    useAppStore.getState().addAgent(AGENT);
    useAppStore.getState().updateAppSettings({ ttsEnabled: true });

    // bell(info)은 발화 대상이 아니다 — 캐릭터의 말이 아니라 터미널 신호다.
    m.emitNotification(notif("a1", "bell"));
    await vi.advanceTimersByTimeAsync(0);
    expect(m.ttsCalls).toHaveLength(0);

    m.emitNotification(notif("a1", "hook"));
    await vi.waitFor(() => expect(m.ttsCalls).toHaveLength(1));
    // 캐릭터 정보가 스토어에서 실려 나간다(보이스 결정적 배정 키 = seed).
    expect(m.ttsCalls[0]).toMatchObject({
      agentId: "a1",
      agentName: "테스트",
      seed: "seed",
      kind: "question",
    });
    await vi.waitFor(() => expect(backend.calls.playVoice).toHaveLength(1));
    off();
  });

  it("stop(done) 알림은 kind=\"done\"으로 발화한다", async () => {
    const { m, off } = install();
    useAppStore.getState().addAgent(AGENT);
    useAppStore.getState().updateAppSettings({ ttsEnabled: true });
    m.emitNotification(notif("a1", "stop"));
    await vi.waitFor(() => expect(m.ttsCalls).toHaveLength(1));
    expect(m.ttsCalls[0]).toMatchObject({ agentId: "a1", kind: "done" });
    off();
  });

  it("프로필의 voiceId가 발화 요청에 실린다(미지정이면 필드 자체가 없다)", async () => {
    const { m, off } = install();
    useAppStore.getState().addAgent({ ...AGENT, voiceId: "v-manual" });
    useAppStore.getState().updateAppSettings({ ttsEnabled: true });
    m.emitNotification(notif("a1", "hook"));
    await vi.waitFor(() => expect(m.ttsCalls).toHaveLength(1));
    expect(m.ttsCalls[0].voiceId).toBe("v-manual");

    useAppStore.getState().updateAgent("a1", { voiceId: undefined });
    m.emitNotification(notif("a1", "hook"));
    await vi.waitFor(() => expect(m.ttsCalls).toHaveLength(2));
    expect("voiceId" in m.ttsCalls[1]).toBe(false);
    off();
  });

  // ── 성격(personality) 주입 ──────────────────────────────────────────
  // 대사 말투의 근거는 성격 프롬프트뿐이다. 종족(archetype)은 보이스 캐스팅
  // 축으로만 실려야 하며 리라이트 프롬프트로는 넘어가지 않는다.
  it("프로필의 성격 프롬프트를 personality로 싣는다(없으면 필드 자체가 없다)", async () => {
    const { m, off } = install();
    useAppStore.getState().addAgent({ ...AGENT, personalityPrompt: "차분하게 말한다" });
    useAppStore.getState().updateAppSettings({ ttsEnabled: true });
    m.emitNotification(notif("a1", "hook"));
    await vi.waitFor(() => expect(m.ttsCalls).toHaveLength(1));
    expect(m.ttsCalls[0].personality).toBe("차분하게 말한다");

    useAppStore.getState().updateAgent("a1", { personalityPrompt: undefined });
    m.emitNotification(notif("a1", "hook"));
    await vi.waitFor(() => expect(m.ttsCalls).toHaveLength(2));
    expect("personality" in m.ttsCalls[1]).toBe(false);
    off();
  });

  // ── 작업 맥락(context) 주입(TTS 리라이트 품질 개선) ──────────────────
  it("발화 요청에 그 에이전트의 현재 작업 라벨(목표+실황)을 context로 싣는다", async () => {
    const { m, off } = install();
    useAppStore.getState().addAgent(AGENT);
    useAppStore.getState().updateAppSettings({ ttsEnabled: true });
    useAppStore.setState({
      taskLabels: {
        a1: {
          sessionId: "s1",
          goal: "빌드 스크립트 정리",
          currentSummary: "테스트 돌리는 중",
        },
      },
    });
    m.emitNotification(notif("a1", "hook"));
    await vi.waitFor(() => expect(m.ttsCalls).toHaveLength(1));
    // deriveTaskLabelLines의 line1(프로젝트·목표) + line2(실황)을 공백으로 이어붙인다.
    expect(m.ttsCalls[0].context).toContain("빌드 스크립트 정리");
    expect(m.ttsCalls[0].context).toContain("테스트 돌리는 중");
    off();
  });

  it("작업 라벨이 없으면 context 필드 자체를 싣지 않는다", async () => {
    const { m, off } = install();
    useAppStore.getState().addAgent(AGENT);
    useAppStore.getState().updateAppSettings({ ttsEnabled: true });
    m.emitNotification(notif("a1", "hook"));
    await vi.waitFor(() => expect(m.ttsCalls).toHaveLength(1));
    expect("context" in m.ttsCalls[0]).toBe(false);
    off();
  });

  it("무음 모드(muted)면 발화하지 않는다", async () => {
    const { m, off } = install();
    useAppStore.getState().addAgent(AGENT);
    useAppStore.getState().updateAppSettings({ ttsEnabled: true });
    useAppStore.getState().toggleMuted();
    m.emitNotification(notif("a1"));
    await vi.advanceTimersByTimeAsync(0);
    expect(m.ttsCalls).toHaveLength(0);
    off();
  });

  // 딩은 대사와 역할이 다르다(즉시 "왔다" vs 수 초 뒤 "무엇을") — TTS가 켜져
  // 있어도 딩은 그대로 울려야 하고, 합성이 실패해도 알림을 놓치지 않는다.
  it("TTS가 켜져 있어도 알림 딩은 그대로 울린다", async () => {
    const { backend, m, off } = install();
    useAppStore.getState().addAgent(AGENT);
    useAppStore.getState().updateAppSettings({ ttsEnabled: true });
    m.emitNotification(notif("a1"));
    expect(backend.calls.playDing).toHaveLength(1);
    off();
  });

  it("세션 running→시작음, exited→종료음, disposed→무음", () => {
    const { backend, m, off } = install();
    const base = { sessionId: "s1", agentId: "a1", at: 1 } as const;
    m.emitSession({ ...base, state: "running" });
    m.emitSession({ ...base, state: "exited" });
    m.emitSession({ ...base, state: "disposed" });
    expect(backend.calls.playSessionStart).toHaveLength(1);
    expect(backend.calls.playSessionEnd).toHaveLength(1);
    off();
  });

  it("teardown이 타이머·구독을 정리하고 backend를 dispose한다", () => {
    const { backend, m, off } = install();
    useAppStore.getState().addAgent(AGENT);
    off();
    expect(m.dataCbs.size).toBe(0);
    expect(backend.calls.dispose).toHaveLength(1);
    m.dataCbs.get("a1"); // 없음
    now += 100;
    vi.advanceTimersByTime(200); // 틱이 죽었으므로 playClicks 없음
    expect(backend.calls.playClicks).toBeUndefined();
  });

  it("previewKeyboardSound는 설치된 backend로 해당 팩의 클릭을 재생한다", () => {
    const { backend, off } = install();
    previewKeyboardSound("topre", "a1");
    vi.advanceTimersByTime(1000);
    expect(backend.calls.playClicks?.length).toBeGreaterThan(0);
    expect(backend.calls.playClicks!.every((c) => c[0] === "a1" && c[2] === "topre")).toBe(true);
    off();
  });

  it("previewKeyboardSound는 설치 해제 후에는 no-op", () => {
    const { backend, off } = install();
    off();
    previewKeyboardSound("topre");
    vi.advanceTimersByTime(1000);
    expect(backend.calls.playClicks).toBeUndefined();
  });

  it("backend가 null이면 아무것도 설치하지 않는다", () => {
    const m = mockApi();
    const off = installSoundManager({ backend: null, api: m.api, now: () => 0 });
    useAppStore.getState().addAgent(AGENT);
    expect(m.dataCbs.size).toBe(0);
    off(); // no-op, 예외 없음
  });
});
