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
import { useAppStore } from "../store/appStore";
import { tauriApi } from "../ipc/tauriApi";
import { MIN_CHUNK_LETTERS, TypingScheduler, meaningfulCount } from "./typing";
import { createWebAudioBackend } from "./backend";
import type { SoundBackend } from "./backend";
import type { AgentOfficeApi } from "@shared/types";

const TICK_MS = 100;

export interface SoundManagerDeps {
  /** undefined면 createWebAudioBackend() 사용. null이면 사운드 불가 환경 시뮬레이션. */
  backend?: SoundBackend | null;
  api?: Pick<AgentOfficeApi, "onData" | "onNotification" | "onSessionState">;
  now?: () => number;
  tickMs?: number;
}

export function installSoundManager(deps: SoundManagerDeps = {}): () => void {
  const backend = deps.backend !== undefined ? deps.backend : createWebAudioBackend();
  if (!backend) return () => {};
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

  const offNotif = api.onNotification(() => {
    if (enabled && !useAppStore.getState().muted) backend.playDing();
  });

  const offSession = api.onSessionState((e) => {
    if (!enabled) return;
    if (e.state === "running") backend.playSessionStart();
    else if (e.state === "exited") backend.playSessionEnd();
  });

  const timer = setInterval(() => {
    for (const [agentId, sched] of schedulers) {
      const n = sched.drain(now());
      if (n > 0 && enabled) backend.playClicks(agentId, n);
    }
  }, tickMs);

  return () => {
    clearInterval(timer);
    offSettings();
    offAgents();
    offNotif();
    offSession();
    for (const off of dataUnsubs.values()) off();
    dataUnsubs.clear();
    schedulers.clear();
    backend.dispose();
  };
}
