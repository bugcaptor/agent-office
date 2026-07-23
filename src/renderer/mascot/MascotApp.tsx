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
  // workArea는 작업표시줄/Dock을 뺀 영역이다. 구버전 API에는 없을 수 있어
  // 옵셔널로 다루고, 없으면 position.ts가 어림 인셋으로 폴백한다(이슈 #73).
  workArea: m.workArea
    ? {
        x: m.workArea.position.x,
        y: m.workArea.position.y,
        width: m.workArea.size.width,
        height: m.workArea.size.height,
      }
    : undefined,
});

const readDpr = (): number =>
  typeof window === "undefined" ? 1 : window.devicePixelRatio || 1;

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
  // 창이 다른 배율의 모니터로 옮겨가면 dpr이 바뀐다(이슈 #73). 마운트 때 한 번만
  // 읽으면 캔버스 백킹 해상도와 커스텀 시트 프리필터가 낡은 배율에 묶여 흐려지거나
  // 과하게 커진다 — Windows per-monitor DPI에서 흔하고, macOS Retina↔외장에서도 난다.
  const [dpr, setDpr] = useState<number>(readDpr);
  const [framesVersion, setFramesVersion] = useState(0);
  const canvasRef = useRef<HTMLCanvasElement | null>(null);
  const framesRef = useRef<MascotFrames | null>(null);
  // 스프라이트 재생성 effect가 상태 전체 변화(hasPending 등)에 재실행되지 않도록,
  // 본문에서 읽을 최신 상태는 ref로 들고 effect는 아래 spriteKey에만 반응시킨다.
  const stateRef = useRef(state);
  stateRef.current = state;

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

  // ---- 배율 변화 추적: 다른 배율의 모니터로 옮기거나 OS 배율이 바뀌면 갱신 ----
  useEffect(() => {
    let un: (() => void) | null = null;
    let disposed = false;
    void getCurrentWindow()
      .onScaleChanged(({ payload }) => {
        // 이벤트의 scaleFactor가 권위 있는 값이다(webview의 devicePixelRatio는
        // 이 시점에 아직 갱신 전일 수 있다). 값이 이상하면 실측으로 폴백.
        const next = Number.isFinite(payload.scaleFactor) && payload.scaleFactor > 0
          ? payload.scaleFactor
          : readDpr();
        setDpr((prev) => (prev === next ? prev : next));
      })
      .then((f) => {
        if (disposed) f();
        else un = f;
      })
      .catch((err) => console.warn("mascot: 배율 변화 구독 실패", err));
    return () => {
      disposed = true;
      un?.();
    };
  }, []);

  // ---- 스프라이트 확보: 외형에 영향을 주는 것이 바뀔 때만 다시 만든다 ----
  // dpr도 키에 들어간다 — 고해상 커스텀 시트의 프리필터 해상도(mascotDetailCell)가
  // dpr에 의존하므로 배율이 바뀌면 리샘플을 다시 해야 한다.
  const spriteKey =
    state.agentId === null
      ? null
      : `${state.agentId}|${state.seed}|${state.archetype}|${state.spriteUpdatedAt}|${dpr}`;

  useEffect(() => {
    if (spriteKey === null) {
      framesRef.current = null;
      setFramesVersion((v) => v + 1);
      return;
    }
    let cancelled = false;
    void loadMascotFrames(stateRef.current, dpr)
      .then((frames) => {
        if (cancelled) return;
        framesRef.current = frames;
        setFramesVersion((v) => v + 1); // 애니 루프를 다시 걸어 즉시 다시 그리게 한다.
      })
      .catch((err) => console.warn("mascot: 스프라이트 생성 실패", err));
    return () => {
      cancelled = true;
    };
    // dpr은 spriteKey에 이미 포함돼 있지만, 본문이 직접 쓰므로 함께 선언한다.
  }, [spriteKey, dpr]);

  const backing = Math.round(MASCOT_SPRITE_PX * dpr);

  // ---- idle 애니메이션 루프. 숨김 상태에서는 아예 돌지 않는다 ----
  // backing/framesVersion이 바뀌면 루프를 다시 건다: 캔버스 크기 변경은 내용을
  // 지우고, frameIndex 캐시 때문에 다음 프레임 교체까지 빈 화면이 남는다.
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
  }, [state.visible, framesVersion, backing]);

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
