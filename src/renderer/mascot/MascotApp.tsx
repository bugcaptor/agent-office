// src/renderer/mascot/MascotApp.tsx
//
// 마스코트 창(이슈 #72)의 유일한 컴포넌트. main 창이 밀어주는 상태를 받아
// 캐릭터 한 명을 그리고, 클릭/드래그만 처리하는 얇은 소비자다. 스토어도
// Pixi도 없다(설계 §1, §4.2).
//
// 부팅 레이스: main은 상태가 바뀔 때만 emit하므로, 이 창이 리스너를 걸기 전에
// 지나간 상태는 영영 못 받는다. 리스너 설치가 끝난 직후 `mascot-ready`를 쏴서
// main이 현재 상태를 다시 보내게 하는 핸드셰이크로 막는다.
import { useEffect, useRef, useState } from "react";
import { emit, listen } from "@tauri-apps/api/event";
import {
  availableMonitors,
  getCurrentWindow,
  primaryMonitor,
  PhysicalPosition,
  type Monitor,
} from "@tauri-apps/api/window";
import { Events } from "@shared/ipc";
import { tauriApi } from "../ipc/tauriApi";
import {
  HIDDEN_MASCOT_STATE,
  MASCOT_ANIM_IDLE_MS,
  MASCOT_SPRITE_PX,
  parseMascotState,
  spriteIdentityChanged,
  type MascotState,
} from "./protocol";
import { loadMascotFrames, type MascotFrames } from "./sheet";
import { createDragDetector } from "./drag";
import {
  readSavedPosition,
  resolvePosition,
  writeSavedPosition,
  type MonitorRect,
} from "./position";
import "./mascot.css";

/** 창 이동 저장 디바운스(ms) — 드래그 중 매 프레임 쓰지 않게. */
const SAVE_DEBOUNCE_MS = 500;

const toRect = (m: Monitor): MonitorRect => ({
  x: m.position.x,
  y: m.position.y,
  width: m.size.width,
  height: m.size.height,
  scaleFactor: m.scaleFactor,
});

/** 저장된 위치로 창을 옮긴다(없거나 화면 밖이면 주 모니터 우하단). */
async function restorePosition(): Promise<void> {
  const win = getCurrentWindow();
  const [size, monitors, primary] = await Promise.all([
    win.outerSize(),
    availableMonitors(),
    primaryMonitor(),
  ]);
  const pos = resolvePosition(
    readSavedPosition(typeof localStorage === "undefined" ? null : localStorage),
    { width: size.width, height: size.height },
    monitors.map(toRect),
    primary ? toRect(primary) : null,
  );
  if (pos) await win.setPosition(new PhysicalPosition(pos.x, pos.y));
}

export default function MascotApp() {
  const [state, setState] = useState<MascotState>(HIDDEN_MASCOT_STATE);
  const canvasRef = useRef<HTMLCanvasElement | null>(null);
  const framesRef = useRef<MascotFrames | null>(null);
  const prevStateRef = useRef<MascotState>(HIDDEN_MASCOT_STATE);

  // ---- main → mascot 상태 수신 + ready 핸드셰이크 ----
  useEffect(() => {
    let un: (() => void) | null = null;
    let disposed = false;
    void listen<unknown>(Events.mascotState, (e) => {
      const next = parseMascotState(e.payload);
      if (next) setState(next);
    })
      .then((f) => {
        if (disposed) {
          f();
          return;
        }
        un = f;
        // 리스너가 살아있는 것이 확정된 뒤에 ready를 알린다.
        void emit(Events.mascotReady).catch(() => {
          /* main이 아직 없으면 다음 상태 변화 때 자연히 받는다 */
        });
      })
      .catch((err) => console.warn("mascot: 상태 구독 실패", err));
    return () => {
      disposed = true;
      un?.();
    };
  }, []);

  // ---- 위치 복원 + 이동 저장 ----
  useEffect(() => {
    void restorePosition().catch((err) => console.warn("mascot: 위치 복원 실패", err));

    let timer: ReturnType<typeof setTimeout> | null = null;
    let un: (() => void) | null = null;
    let disposed = false;
    void getCurrentWindow()
      .onMoved(({ payload }) => {
        if (timer !== null) clearTimeout(timer);
        timer = setTimeout(() => {
          timer = null;
          writeSavedPosition(typeof localStorage === "undefined" ? null : localStorage, payload);
        }, SAVE_DEBOUNCE_MS);
      })
      .then((f) => {
        if (disposed) f();
        else un = f;
      })
      .catch((err) => console.warn("mascot: 이동 구독 실패", err));

    return () => {
      disposed = true;
      if (timer !== null) clearTimeout(timer);
      un?.();
    };
  }, []);

  // ---- 스프라이트 확보: 외형에 영향을 주는 필드가 바뀔 때만 다시 만든다 ----
  useEffect(() => {
    const prev = prevStateRef.current;
    prevStateRef.current = state;
    if (framesRef.current !== null && !spriteIdentityChanged(prev, state)) return;
    if (state.agentId === null) {
      framesRef.current = null;
      return;
    }
    let cancelled = false;
    void loadMascotFrames(state)
      .then((frames) => {
        if (!cancelled) framesRef.current = frames;
      })
      .catch((err) => console.warn("mascot: 스프라이트 생성 실패", err));
    return () => {
      cancelled = true;
    };
  }, [state]);

  // ---- idle 애니메이션 루프. 숨김 상태에서는 아예 돌지 않는다 ----
  useEffect(() => {
    if (!state.visible) return;
    let raf = 0;
    let frameIndex = -1;
    let start = performance.now();

    const tick = (now: number) => {
      raf = requestAnimationFrame(tick);
      const frames = framesRef.current;
      const canvas = canvasRef.current;
      if (!frames || !canvas) return;
      const next = Math.floor((now - start) / MASCOT_ANIM_IDLE_MS) % frames.idle.length;
      if (next === frameIndex) return;
      frameIndex = next;
      const ctx = canvas.getContext("2d");
      if (!ctx) return;
      ctx.clearRect(0, 0, canvas.width, canvas.height);
      ctx.imageSmoothingEnabled = false;
      ctx.drawImage(
        frames.idle[next],
        0,
        0,
        frames.cell,
        frames.cell,
        0,
        0,
        canvas.width,
        canvas.height,
      );
    };
    raf = requestAnimationFrame(tick);
    return () => {
      cancelAnimationFrame(raf);
      start = 0;
    };
  }, [state.visible, state.agentId]);

  // ---- 클릭 vs 드래그 ----
  const detector = useRef(createDragDetector()).current;
  const onPointerDown = (e: React.PointerEvent<HTMLCanvasElement>) => {
    e.currentTarget.setPointerCapture(e.pointerId);
    detector.down(e.screenX, e.screenY);
  };
  const onPointerMove = (e: React.PointerEvent<HTMLCanvasElement>) => {
    if (detector.move(e.screenX, e.screenY) !== "start-drag") return;
    // OS 창 드래그로 넘어간다 — 이후 pointerup은 오지 않는다(정상).
    e.currentTarget.releasePointerCapture(e.pointerId);
    void getCurrentWindow()
      .startDragging()
      .catch((err) => console.warn("mascot: 드래그 시작 실패", err));
  };
  const onPointerUp = (e: React.PointerEvent<HTMLCanvasElement>) => {
    if (e.currentTarget.hasPointerCapture(e.pointerId)) {
      e.currentTarget.releasePointerCapture(e.pointerId);
    }
    if (detector.up() !== "click") return;
    const agentId = state.agentId;
    if (agentId === null) return;
    void tauriApi
      .mascotActivate(agentId)
      .catch((err) => console.warn("mascot: 활성화 실패", err));
  };

  const dpr = typeof window === "undefined" ? 1 : window.devicePixelRatio || 1;
  const backing = Math.round(MASCOT_SPRITE_PX * dpr);

  return (
    <div className="mascot-root">
      {state.hasPending && (
        <div className="mascot-badge" aria-hidden="true">
          !
        </div>
      )}
      <canvas
        ref={canvasRef}
        className={`mascot-sprite${state.hasPending ? " pending" : ""}`}
        width={backing}
        height={backing}
        style={{ width: MASCOT_SPRITE_PX, height: MASCOT_SPRITE_PX }}
        title={state.name ?? undefined}
        onPointerDown={onPointerDown}
        onPointerMove={onPointerMove}
        onPointerUp={onPointerUp}
        onPointerCancel={() => detector.cancel()}
      />
    </div>
  );
}
