// src/renderer/ipc/mascotBridge.ts
//
// 데스크톱 마스코트 창(이슈 #72, docs/mascot-window-design.md)의 main 창 측
// 배선. 스토어(진실의 원천)를 구독해 "지금 보여줄 캐릭터 1명"(스프라이트)과
// "신호등 칸 목록"(docs/mascot-lights-design.md)을 계산하고, 그 스냅샷을
// `mascot-state` 이벤트로 마스코트 창에 밀어 넣는다. 마스코트는 순수 소비자라
// 스토어를 두 번 hydrate하지 않는다(상태 표류 원천 차단).
//
// 세 갈래 배선:
//  - 스토어 → 마스코트: notifications/timeTracking/agents/taskLabels/appSettings
//    구독 → pickMascotTarget(스프라이트) + computeMascotLights(신호등) →
//    변화가 있을 때만 emit(+ visible 변화 시 창 show/hide).
//  - 마스코트 부팅 → main: `mascot-ready` 수신 시 현재 상태 재방출(리스너 설치
//    전에 보낸 emit을 놓치는 부팅 레이스를 핸드셰이크로 해소).
//  - 마스코트 클릭 → main: Rust가 main을 포커스한 뒤 `mascot-open-terminal`을
//    emit_to하고, 여기서 officeBus.emitAgentClicked로 넘긴다(세션 보장 +
//    터미널 열기 + 알림 클리어가 이미 그 안에 있다 — 재구현 금지).
//
// 스프라이트 활동이 끊겨도 15초는 그대로 둔다(linger): 턴 사이 짧은 유휴마다
// 창이 사라졌다 나타나면 눈에 거슬린다. 신호등은 대시보드라 linger를 적용하지
// 않는다 — 라이브 상태 그대로 emit한다(여운이 오히려 거짓 정보). linger가
// 다 돼도 신호등 칸이 남아 있으면 창은 스프라이트만 접고 계속 떠 있는다(결정
// 6). 설정을 끄는 것은 사용자의 명시적 의사라 linger 없이 즉시 숨긴다
// (keepAwake의 즉시 해제와 같은 관례) — mascotEnabled가 신호등의 상위 게이트도
// 겸하므로 OFF면 신호등도 함께 사라진다.
import { emit, listen } from "@tauri-apps/api/event";
import { Events } from "@shared/ipc";
import { useAppStore } from "../store/appStore";
import { pickMascotTarget } from "../store/selectors";
import { computeMascotLights } from "../store/mascotLights";
import {
  HIDDEN_MASCOT_STATE,
  sameMascotState,
  type MascotState,
} from "../mascot/protocol";
import { officeBus } from "./sessionBridge";
import { tauriApi } from "./tauriApi";

/** visible·lights·lightsVertical·lightsFace·lightsWide를 뺀 스프라이트 전용
 *  필드 — buildState가 조립하는 단위. */
type SpriteFields = Omit<
  MascotState,
  "visible" | "lights" | "lightsVertical" | "lightsFace" | "lightsWide"
>;

/** 스프라이트 대상이 없을 때(활동 없음, linger도 소진)의 빈 값. */
const EMPTY_SPRITE: SpriteFields = {
  agentId: null,
  name: null,
  seed: null,
  archetype: null,
  colors: null,
  spriteUpdatedAt: null,
  hasPending: false,
  working: false,
};

/** 활동이 끊긴 뒤 마스코트를 유지하는 시간(ms). 턴 사이 깜빡임 방지. */
export const MASCOT_HIDE_LINGER_MS = 15_000;

/** 테스트 주입점 — 실제 구현은 Tauri 이벤트/커맨드와 officeBus. */
export interface MascotBridgeIo {
  emitState(state: MascotState): void;
  setVisible(visible: boolean): void;
  onMascotReady(cb: () => void): () => void;
  onOpenTerminal(cb: (agentId: string) => void): () => void;
  openTerminal(agentId: string): void;
}

/** `listen()`의 비동기 UnlistenFn을 동기 해제 함수로 감싼다(tauriApi.wrapListen과 동일 패턴). */
function wrapListen<T>(event: string, cb: (payload: T) => void): () => void {
  let un: (() => void) | null = null;
  let disposed = false;
  void listen<T>(event, (e) => cb(e.payload)).then((f) => {
    if (disposed) f();
    else un = f;
  });
  return () => {
    if (disposed) return;
    disposed = true;
    if (un) {
      un();
      un = null;
    }
  };
}

function defaultIo(): MascotBridgeIo {
  return {
    emitState(state) {
      void emit(Events.mascotState, state).catch((err) =>
        console.warn("mascotBridge: state emit failed", err),
      );
    },
    setVisible(visible) {
      void tauriApi
        .setMascotVisible(visible)
        .catch((err) => console.warn("mascotBridge: window visibility toggle failed", err));
    },
    onMascotReady(cb) {
      return wrapListen<unknown>(Events.mascotReady, () => cb());
    },
    onOpenTerminal(cb) {
      return wrapListen<{ agentId?: unknown }>(Events.mascotOpenTerminal, (p) => {
        if (typeof p?.agentId === "string") cb(p.agentId);
      });
    },
    openTerminal(agentId) {
      officeBus.emitAgentClicked(agentId);
    },
  };
}

/**
 * 마스코트 브리지 설치. bootstrap에서 installSessionBridge 직후 1회 호출하고,
 * 반환된 함수로 해제한다(앱 자체는 수명 내내 유지 — 테스트 teardown용).
 */
export function installMascotBridge(io: MascotBridgeIo = defaultIo()): () => void {
  let last: MascotState = HIDDEN_MASCOT_STATE;
  /** 마지막으로 실제 선택된 에이전트 — sticky 판정과 linger 표시의 기준. */
  let lastPickId: string | null = null;
  /** linger 시간을 다 쓰고 스프라이트를 접었는지 — true면 활동이 돌아오기
   * 전까지 다시 조용한 스프라이트를 보여주지 않는다(재-linger 방지). */
  let spriteLingerDone = false;
  let lingerTimer: ReturnType<typeof setTimeout> | null = null;

  const clearLinger = () => {
    if (lingerTimer !== null) {
      clearTimeout(lingerTimer);
      lingerTimer = null;
    }
  };

  const publish = (next: MascotState) => {
    const visibilityChanged = next.visible !== last.visible;
    if (sameMascotState(next, last)) return;
    last = next;
    // 창을 먼저 띄우고 상태를 보내면 빈 창이 한 프레임 보일 수 있다 — 상태를
    // 먼저 emit한다. 마스코트가 아직 리스너를 안 걸었어도 ready 핸드셰이크가
    // 현재 상태를 다시 받아가므로 유실되지 않는다.
    io.emitState(next);
    if (visibilityChanged) io.setVisible(next.visible);
  };

  const buildState = (): MascotState => {
    const s = useAppStore.getState();
    if (!s.appSettings.mascotEnabled) {
      // 설정 OFF는 즉시 숨김 — linger 없음. mascotEnabled가 신호등의 상위
      // 게이트이기도 하므로(결정 6) 신호등도 함께 꺼진다.
      clearLinger();
      lastPickId = null;
      spriteLingerDone = false;
      return HIDDEN_MASCOT_STATE;
    }

    const lights = computeMascotLights({
      mode: s.appSettings.mascotLightsMode,
      labelMode: s.appSettings.mascotLightsLabel,
      projects: s.appSettings.mascotLightsProjects,
      agentOrder: s.agentOrder,
      agents: s.agents,
      timeTracking: s.timeTracking,
      notifications: s.notifications,
      taskLabels: s.taskLabels,
    });

    const pick = pickMascotTarget({
      notifications: s.notifications,
      timeTracking: s.timeTracking,
      agents: s.agents,
      prevAgentId: lastPickId,
    });

    let sprite: SpriteFields;
    if (pick.agentId !== null) {
      // 활동 재개 — linger 해제하고 새 대상을 싣는다.
      clearLinger();
      spriteLingerDone = false;
      lastPickId = pick.agentId;
      const agent = s.agents[pick.agentId];
      sprite = {
        agentId: pick.agentId,
        name: agent?.name ?? pick.agentId,
        seed: agent?.seed || pick.agentId,
        archetype: agent?.archetype ?? null,
        colors: agent?.colors ?? null,
        spriteUpdatedAt: agent?.spriteUpdatedAt ?? null,
        hasPending: pick.hasPending,
        working: pick.working,
      };
    } else if (last.agentId !== null && !spriteLingerDone) {
      // 활동 없음, linger 아직 안 씀: 직전 캐릭터를 조용한 모습으로
      // linger 시간만큼 유지한다.
      if (lingerTimer === null) {
        lingerTimer = setTimeout(() => {
          lingerTimer = null;
          spriteLingerDone = true;
          lastPickId = null;
          // HIDDEN_MASCOT_STATE를 직접 보내지 않는다 — 신호등 칸이 남아
          // 있으면 창은 스프라이트만 접고 계속 떠 있어야 하므로, 가시성
          // 판단은 항상 buildState에게 다시 맡긴다.
          publish(buildState());
        }, MASCOT_HIDE_LINGER_MS);
      }
      sprite = {
        agentId: last.agentId,
        name: last.name,
        seed: last.seed,
        archetype: last.archetype,
        colors: last.colors,
        spriteUpdatedAt: last.spriteUpdatedAt,
        hasPending: false,
        working: false,
      };
    } else {
      // 활동 없음 + linger 소진(또는 애초에 스프라이트가 없었음).
      sprite = EMPTY_SPRITE;
    }

    return {
      visible: sprite.agentId !== null || lights.length > 0,
      ...sprite,
      lights,
      lightsVertical: s.appSettings.mascotLightsVertical,
      lightsFace: s.appSettings.mascotLightsFace,
      // 마스코트 창은 설정 의미를 모르고 렌더 관심사(칸 폭)만 받는다 —
      // task 라벨은 60자 절단 텍스트가 들어가 54px 타일에서 심하게 잘리므로
      // wide(96px) 타일로 넓힌다(§7 개정).
      lightsWide: s.appSettings.mascotLightsLabel === "task",
    };
  };

  const recompute = () => publish(buildState());

  const offNotifications = useAppStore.subscribe((s) => s.notifications, recompute);
  const offTiming = useAppStore.subscribe((s) => s.timeTracking, recompute);
  const offAgents = useAppStore.subscribe((s) => s.agents, recompute);
  const offTaskLabels = useAppStore.subscribe((s) => s.taskLabels, recompute);
  // mascotEnabled 하나만이 아니라 appSettings 전체를 구독한다 — 신호등 설정
  // (mode/vertical/projects) 변경도 재계산을 트리거해야 한다. 신호등과
  // 무관한 설정 변경은 결과가 같아 sameMascotState dedupe가 흡수한다.
  const offSetting = useAppStore.subscribe((s) => s.appSettings, recompute);

  // 마스코트 부팅 핸드셰이크: 현재 상태를 무조건 다시 보낸다(dedupe 우회).
  const offReady = io.onMascotReady(() => {
    io.emitState(last);
    io.setVisible(last.visible);
  });
  const offClick = io.onOpenTerminal((agentId) => io.openTerminal(agentId));

  recompute(); // 부트 시 1회 반영(설정 hydrate 이후에 설치되므로 값이 유효하다).

  return () => {
    offNotifications();
    offTiming();
    offAgents();
    offTaskLabels();
    offSetting();
    offReady();
    offClick();
    clearLinger();
  };
}
