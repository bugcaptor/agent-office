// src/renderer/sound/soundManager.ts
//
// 사운드 조립: 스토어(설정·에이전트 목록)와 IPC(출력·알림·세션 상태)를
// 구독해 SoundBackend를 구동한다. 앱 부트에서 1회 설치(bootstrap.ts).
// deps는 테스트 주입용 — 실제 앱은 인자 없이 부른다.
//
// 소리 게이트는 셋으로 갈린다(설정) + 하나의 마스터(런타임):
//   typingSoundEnabled → 타건음 / notifySoundEnabled → 딩·세션 효과음 /
//   ttsEnabled → 대사 발화. 셋 다 muted(무음 모드)를 상위 마스터로 존중한다.
//   "타건 소리는 시끄러워서 껐지만 알림은 듣고 싶다"가 성립해야 하므로 서로를
//   보지 않는다. 볼륨(soundVolume)만 셋이 공유한다.
//
// 정책:
// - 타건음이 꺼져 있어도 스케줄러는 계속 drain한다(버림) — 재활성 시 밀린
//   클릭이 몰아치는 것을 방지.
// - disposed는 exited와 중복되는 정리 신호라 무음.
// - 대사 TTS: question(hook)과 done(stop)을 발화하고 info(bell)는 제외한다.
//   딩은 **생략하지 않고** 그대로 울린다 — 딩은 즉시 나고 대사는 리라이트+합성
//   왕복(수 초) 뒤에 오므로 겹치지 않는다. 딩이 "왔다"는 신호, 대사가 "무엇을"
//   이라 역할이 다르고, 발화가 실패하는 경우에도 알림을 놓치지 않는다.
import { useAppStore } from "../store/appStore";
import { tauriApi } from "../ipc/tauriApi";
import { MIN_CHUNK_LETTERS, TypingScheduler, meaningfulCount } from "./typing";
import { createWebAudioBackend } from "./backend";
import { base64ToBytes, createVoiceQueue } from "./voiceQueue";
import { deriveTaskLabelLines } from "../labels/labelText";
import type { SoundBackend } from "./backend";
import type { AgentOfficeApi, TtsSpeakKind, TtsSpeakRequest } from "@shared/types";
import { notificationType } from "@shared/types";

const TICK_MS = 100;
// 대사 리라이트에 실을 작업 맥락(머리 위 라벨과 같은 파생 규칙, 이슈 없음 —
// TerminalSummaryBar와 같은 폭)의 절단 상한. 최종 상한은 백엔드가 300자로
// 다시 자르므로(rewrite.rs MAX_CONTEXT_CHARS) 여기서는 넉넉히 잡는다.
const CONTEXT_GOAL_MAX = 60;
const CONTEXT_CURRENT_MAX = 90;

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
 * 대사 TTS 미리듣기. 큐를 거치지 않고 즉시 1회 합성·재생하고, 실제로 발화된
 * 대사 텍스트를 돌려준다(설정/프로필 UI가 표시).
 *
 * 기본값은 첫 캐릭터(없으면 이름 없는 미리듣기용 시드)다 — 시드가 다르면
 * 목소리도 다르므로 "그 캐릭터의 목소리"를 확인하는 것이 목적이다. 프로필
 * 다이얼로그는 `overrides`로 편집 중인 캐릭터와 고른 voiceId를 넘긴다.
 *
 * 상황은 항상 question이다. 미리듣기의 목적은 "이 캐릭터가 어떤 목소리냐"이지
 * 어조 비교가 아니고, 기준 문구를 하나로 고정해야 캐시도 재사용된다.
 *
 * **무음 모드에서도 울린다.** 사용자가 방금 누른 버튼이 침묵하면 고장으로
 * 보이기 때문이다 — 대신 UI가 그 옆에 무음 상태를 알린다.
 *
 * 실패는 throw한다(UI가 사유를 보여줘야 하므로 여기서는 삼키지 않는다).
 */
export async function previewVoice(overrides: Partial<TtsSpeakRequest> = {}): Promise<string> {
  const store = useAppStore.getState();
  const agentId = store.agentOrder[0];
  const agent = agentId ? store.agents[agentId] : undefined;
  const result = await tauriApi.ttsSpeak({
    agentId: agentId ?? "preview",
    agentName: agent?.name ?? "",
    archetype: agent?.archetype,
    ...(agent?.personalityPrompt ? { personality: agent.personalityPrompt } : {}),
    seed: agent?.seed ?? "preview",
    message: PREVIEW_MESSAGE,
    kind: "question",
    ...overrides,
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

  let typingEnabled = useAppStore.getState().appSettings.typingSoundEnabled;
  let notifyEnabled = useAppStore.getState().appSettings.notifySoundEnabled;
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
      typingEnabled = as.typingSoundEnabled;
      notifyEnabled = as.notifySoundEnabled;
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
  // 타건음/알림음 스위치는 일부러 보지 않는다(위 머리말의 3분할 정책).
  // 반면 muted(무음 모드)는 "지금 아무 소리도 내지 마라"라는 전역 의사다.
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
    if (notifyEnabled && !store.muted) backend.playDing();
    // 확인 요청(question)과 완료 보고(done)를 발화한다. bell(info)은 제외 —
    // 대개 캐릭터의 말이 아니라 터미널이 낸 신호라 읽어줄 내용이 없다.
    const type = notificationType(e.source);
    const kind: TtsSpeakKind | null =
      type === "question" ? "question" : type === "done" ? "done" : null;
    if (!kind) return;
    if (!voiceAllowed()) return;
    const agent = store.agents[e.agentId];
    // 그 에이전트가 지금 무슨 작업을 하던 중인지 한 줄 — 머리 위 라벨과 같은
    // 파생 규칙(labelText.deriveTaskLabelLines)을 재사용해 "빌드 돌려도
    // 될까요?" 처럼 상황 밀착형 대사가 나오도록 리라이트 프롬프트에 참고용으로
    // 실어 보낸다. 두 줄이면 공백으로 이어붙인다.
    const { line1, line2 } = deriveTaskLabelLines(store.taskLabels[e.agentId], agent?.cwd, {
      goalMax: CONTEXT_GOAL_MAX,
      currentMax: CONTEXT_CURRENT_MAX,
    });
    const context = [line1, line2].filter(Boolean).join(" ") || undefined;
    voiceQueue.enqueue({
      agentId: e.agentId,
      agentName: agent?.name ?? "",
      // archetype은 보이스 캐스팅용, 말투는 성격 프롬프트만 — 축이 갈린다.
      archetype: agent?.archetype,
      seed: agent?.seed ?? "",
      message: e.message,
      kind,
      ...(agent?.personalityPrompt ? { personality: agent.personalityPrompt } : {}),
      ...(agent?.voiceId ? { voiceId: agent.voiceId } : {}),
      ...(context ? { context } : {}),
    });
  });

  const offSession = api.onSessionState((e) => {
    if (!notifyEnabled || useAppStore.getState().muted) return;
    if (e.state === "running") backend.playSessionStart();
    else if (e.state === "exited") backend.playSessionEnd();
  });

  const timer = setInterval(() => {
    const muted = useAppStore.getState().muted;
    for (const [agentId, sched] of schedulers) {
      const n = sched.drain(now());
      if (n > 0 && typingEnabled && !muted)
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
