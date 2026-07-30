// src/renderer/sound/soundManager.ts
//
// 사운드 조립: 스토어(설정·에이전트 목록)와 IPC(출력·알림·세션 상태)를
// 구독해 SoundBackend를 구동한다. 앱 부트에서 1회 설치(bootstrap.ts).
// deps는 테스트 주입용 — 실제 앱은 인자 없이 부른다.
//
// 정책:
// - soundEnabled=false여도 스케줄러는 계속 drain한다(버림) — 재활성 시
//   밀린 클릭이 몰아치는 것을 방지.
// - 알림 딩은 무음 모드(store.muted)도 존중.
// - disposed는 exited와 중복되는 정리 신호라 무음.
// - 확인 요청 대사 TTS: question 알림(source="hook")에서만 발화한다. 딩은
//   **생략하지 않고** 그대로 울린다 — 딩은 즉시 나고 대사는 리라이트+합성
//   왕복(수 초) 뒤에 오므로 겹치지 않는다. 딩이 "왔다"는 신호, 대사가 "무엇을
//   묻는지"라 역할이 다르고, 발화가 실패하는 경우에도 알림을 놓치지 않는다.
import { useAppStore } from "../store/appStore";
import { tauriApi } from "../ipc/tauriApi";
import { MIN_CHUNK_LETTERS, TypingScheduler, meaningfulCount } from "./typing";
import { createWebAudioBackend } from "./backend";
import { base64ToBytes, createVoiceQueue } from "./voiceQueue";
import type { SoundBackend } from "./backend";
import type { AgentOfficeApi } from "@shared/types";
import { notificationType } from "@shared/types";

const TICK_MS = 100;

/** 설치된 backend — previewKeyboardSound(프로필 다이얼로그 미리듣기)용. */
let activeBackend: SoundBackend | null = null;

/**
 * 키보드 사운드 팩 미리듣기 — 사람 타속(~9타/초)의 짧은 버스트.
 * 사운드 매니저 미설치/사운드 불가 환경/해제 후에는 no-op.
 * agentId를 주면 그 에이전트의 고유 피치로 들린다.
 */
export function previewKeyboardSound(packId?: string, agentId = "preview"): void {
  const backend = activeBackend;
  if (!backend) return;
  for (let i = 0; i < 6; i++) {
    setTimeout(() => {
      if (activeBackend === backend) backend.playClicks(agentId, 1, packId);
    }, i * 110);
  }
}

/** 설정 다이얼로그 "시청" 버튼이 쓰는 샘플 문구 — 실제 훅 알림의 기본 문구
 * (`notification/hub.rs`의 ATTENTION_FALLBACK)와 같게 둬서, 들리는 것이 실제
 * 발화와 같은 파이프라인·같은 톤임을 확인할 수 있게 한다. */
export const PREVIEW_MESSAGE = "확인이 필요합니다";

/**
 * 확인 요청 대사 TTS 미리듣기. 큐를 거치지 않고 즉시 1회 합성·재생하고,
 * 실제로 발화된 대사 텍스트를 돌려준다(설정 UI가 표시).
 *
 * 첫 캐릭터(없으면 이름 없는 미리듣기용 시드)의 목소리로 들려준다 — 시드가
 * 다르면 목소리도 다르므로 "그 캐릭터의 목소리"를 확인하는 것이 목적이다.
 * 실패는 throw한다(설정 UI가 사유를 보여줘야 하므로 여기서는 삼키지 않는다).
 */
export async function previewVoice(): Promise<string> {
  const store = useAppStore.getState();
  const agentId = store.agentOrder[0];
  const agent = agentId ? store.agents[agentId] : undefined;
  const result = await tauriApi.ttsSpeak({
    agentId: agentId ?? "preview",
    agentName: agent?.name ?? "",
    archetype: agent?.archetype,
    seed: agent?.seed ?? "preview",
    message: PREVIEW_MESSAGE,
  });
  const backend = activeBackend;
  if (backend) await backend.playVoice(base64ToBytes(result.audioBase64));
  return result.line;
}

export interface SoundManagerDeps {
  /** undefined면 createWebAudioBackend() 사용. null이면 사운드 불가 환경 시뮬레이션. */
  backend?: SoundBackend | null;
  api?: Pick<AgentOfficeApi, "onData" | "onNotification" | "onSessionState" | "ttsSpeak">;
  now?: () => number;
  tickMs?: number;
}

export function installSoundManager(deps: SoundManagerDeps = {}): () => void {
  const backend = deps.backend !== undefined ? deps.backend : createWebAudioBackend();
  if (!backend) return () => {};
  activeBackend = backend;
  const api = deps.api ?? tauriApi;
  const now = deps.now ?? (() => performance.now());
  const tickMs = deps.tickMs ?? TICK_MS;

  let enabled = useAppStore.getState().appSettings.soundEnabled;
  const schedulers = new Map<string, TypingScheduler>();
  const dataUnsubs = new Map<string, () => void>();

  function reconcileAgents(agentIds: string[]): void {
    for (const id of agentIds) {
      if (dataUnsubs.has(id)) continue;
      const sched = new TypingScheduler(now());
      schedulers.set(id, sched);
      dataUnsubs.set(
        id,
        api.onData(id, (data) => {
          // TUI 스피너/상태줄 리페인트·키 에코는 무시 — 본문다운 청크만
          // 타이핑 시간으로 인정해 "텍스트가 많이 나올 때"만 소리를 낸다.
          const letters = meaningfulCount(data);
          if (letters >= MIN_CHUNK_LETTERS) sched.push(letters, now());
        })
      );
    }
    for (const [id, off] of dataUnsubs) {
      if (agentIds.includes(id)) continue;
      off();
      dataUnsubs.delete(id);
      schedulers.delete(id);
    }
  }

  const offSettings = useAppStore.subscribe(
    (s) => s.appSettings,
    (as) => {
      enabled = as.soundEnabled;
      backend.setVolume(as.soundVolume);
    },
    { fireImmediately: true }
  );

  const offAgents = useAppStore.subscribe((s) => s.agentOrder, reconcileAgents, {
    fireImmediately: true,
  });

  // 대사 발화 게이트 — 큐에서 꺼내는 시점에 다시 확인한다. 합성 왕복 중
  // 사용자가 무음/TTS를 끄면 그 뒤 대기 항목은 발화하지 않는다.
  //
  // soundEnabled(사무실 앰비언스)는 일부러 보지 않는다 — 그건 타이핑 소리·
  // 효과음 스위치이고, 대사는 별개의 opt-in이다. "타건 소리는 시끄러워서 껐지만
  // 확인 요청은 말로 듣고 싶다"가 성립한다. 반면 muted(무음 모드)는 "지금
  // 아무 소리도 내지 마라"라는 전역 의사라 존중한다.
  const voiceAllowed = () => {
    const s = useAppStore.getState();
    return s.appSettings.ttsEnabled && !s.muted;
  };
  const voiceQueue = createVoiceQueue({
    speak: (request) => api.ttsSpeak(request),
    play: (mp3) => backend.playVoice(mp3),
    shouldSpeak: voiceAllowed,
  });

  const offNotif = api.onNotification((e) => {
    const store = useAppStore.getState();
    if (enabled && !store.muted) backend.playDing();
    // 확인 요청(question)만 발화한다 — stop(done)/bell(info)은 대상이 아니다.
    if (notificationType(e.source) !== "question") return;
    if (!voiceAllowed()) return;
    const agent = store.agents[e.agentId];
    voiceQueue.enqueue({
      agentId: e.agentId,
      agentName: agent?.name ?? "",
      archetype: agent?.archetype,
      seed: agent?.seed ?? "",
      message: e.message,
    });
  });

  const offSession = api.onSessionState((e) => {
    if (!enabled) return;
    if (e.state === "running") backend.playSessionStart();
    else if (e.state === "exited") backend.playSessionEnd();
  });

  const timer = setInterval(() => {
    for (const [agentId, sched] of schedulers) {
      const n = sched.drain(now());
      if (n > 0 && enabled)
        backend.playClicks(agentId, n, useAppStore.getState().agents[agentId]?.keyboardSound);
    }
  }, tickMs);

  return () => {
    if (activeBackend === backend) activeBackend = null;
    clearInterval(timer);
    offSettings();
    offAgents();
    offNotif();
    offSession();
    voiceQueue.clear();
    for (const off of dataUnsubs.values()) off();
    dataUnsubs.clear();
    schedulers.clear();
    backend.dispose();
  };
}
