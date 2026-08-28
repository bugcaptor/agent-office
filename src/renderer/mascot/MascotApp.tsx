// src/renderer/mascot/MascotApp.tsx
//
// 마스코트 창(이슈 #72)의 유일한 컴포넌트. main 창이 밀어주는 상태를 받아
// 캐릭터 한 명을 그리고, 클릭/드래그만 처리하는 얇은 소비자다. 스토어도
// Pixi도 없다(설계 §1, §4.2).
//
// 부팅 레이스: main은 상태가 바뀔 때만 emit하므로, 이 창이 리스너를 걸기 전에
// 지나간 상태는 영영 못 받는다. 리스너 설치가 끝난 직후 `mascot-ready`를 쏴서
// main이 현재 상태를 다시 보내게 하는 핸드셰이크로 막는다.
//
// i18n 한계(의도된 것): 이 창은 **메인 창과 다른 webview**이고 설정을 직접 읽지
// 않는다. `main.tsx`가 `import "../i18n"`으로 i18next를 켜지만 언어는 메인 창이
// localStorage에 남긴 캐시값으로 정해지고, 메인 창에서 언어를 바꿔도 **여기로
// 실시간 전파되지 않는다** — 다음 마스코트 창 생성 때 반영된다. 지금 이 창에
// 번역 대상 문구가 사실상 없어(캔버스와 배지뿐) 실해가 없으므로 그대로 둔다.
// 나중에 텍스트가 늘면 `mascot-state` 페이로드에 언어를 실어 보내거나 전용
// 이벤트로 `applyLanguageSetting`을 밀어 주어야 한다.
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
  type MascotLight,
  type MascotState,
} from "./protocol";
import { computeMascotWindowRect, foldOverflow } from "./layout";
import { loadMascotFrames, type MascotFrames } from "./sheet";
import { createDragDetector } from "./drag";
import {
  anchorOf,
  readSavedAnchor,
  resolveAnchoredPosition,
  writeSavedAnchor,
  type MonitorRect,
} from "./position";
import "./mascot.css";

/** 창 이동 저장 디바운스(ms) — 드래그 중 매 프레임 쓰지 않게. */
const SAVE_DEBOUNCE_MS = 500;
/** 신호등 레이아웃 변화 → 창 리사이즈 적용 디바운스(ms). 에이전트 모드는
 *  턴 경계마다 칸 수가 바뀌므로(결정 2), 연속 변화 중 리사이즈가 폭주하지
 *  않게 마지막 값만 적용한다(docs/mascot-lights-design.md §5.3). */
const LAYOUT_DEBOUNCE_MS = 300;

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

/** 저장된 앵커로 창을 옮긴다(없거나 화면 밖이면 주 모니터 우하단). */
async function restorePosition(): Promise<void> {
  const win = getCurrentWindow();
  const [size, monitors, primary] = await Promise.all([
    win.outerSize(),
    availableMonitors(),
    primaryMonitor(),
  ]);
  const currentSize = { width: size.width, height: size.height };
  const anchor = readSavedAnchor(
    typeof localStorage === "undefined" ? null : localStorage,
    currentSize,
  );
  const pos = resolveAnchoredPosition(
    anchor,
    currentSize,
    monitors.map(toRect),
    primary ? toRect(primary) : null,
  );
  if (pos) await win.setPosition(new PhysicalPosition(pos.x, pos.y));
}

/**
 * C9: 신호등 레이아웃(칸 수·방향·스프라이트 유무)이 바뀌었을 때 창 크기를
 * 다시 맞춘다. 앵커는 저장값이 아니라 **호출 시점**의 `outerPosition()`/
 * `outerSize()`에서 다시 뽑는다 — 사용자가 방금 끌어다 둔 자리를 존중하기
 * 위해서다. 계산은 순수 함수(`computeMascotWindowRect`)에 전부 맡기고,
 * 여기서는 Tauri 호출로 값을 모으고 결과를 적용하는 얇은 배선만 한다.
 */
async function applyMascotLayout(
  lightCount: number,
  vertical: boolean,
  hasSprite: boolean,
  dpr: number,
): Promise<void> {
  const win = getCurrentWindow();
  const [pos, size, monitors, primary] = await Promise.all([
    win.outerPosition(),
    win.outerSize(),
    availableMonitors(),
    primaryMonitor(),
  ]);
  const rect = computeMascotWindowRect({
    lightCount,
    vertical,
    hasSprite,
    dpr,
    currentPos: { x: pos.x, y: pos.y },
    currentSize: { width: size.width, height: size.height },
    monitors: monitors.map(toRect),
    primary: primary ? toRect(primary) : null,
  });
  await tauriApi.setMascotLayout(rect.width, rect.height, rect.x, rect.y);
  // set_mascot_layout이 유발하는 onMoved도 같은 앵커로 계산되므로 멱등하다.
  writeSavedAnchor(
    typeof localStorage === "undefined" ? null : localStorage,
    anchorOf({ x: rect.x, y: rect.y }, { width: rect.width, height: rect.height }),
  );
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
  // 레이아웃 적용 effect는 파생 키(아래 layoutKey)에만 반응시키고 dpr 변화만
  // 으로는 재실행하지 않는다(§C9 — 트리거는 칸 수·방향·스프라이트 유무뿐).
  // 그래도 실제 호출 시점의 최신 dpr은 필요하므로 ref로 들고 있는다.
  const dprRef = useRef(dpr);
  dprRef.current = dpr;

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
      .catch((err) => console.warn("mascot: failed to subscribe to state", err));
    return () => {
      disposed = true;
      un?.();
    };
  }, []);

  // ---- 위치 복원 + 이동 저장 ----
  useEffect(() => {
    void restorePosition().catch((err) => console.warn("mascot: failed to restore position", err));

    let timer: ReturnType<typeof setTimeout> | null = null;
    let un: (() => void) | null = null;
    let disposed = false;
    void getCurrentWindow()
      .onMoved(({ payload }) => {
        if (timer !== null) clearTimeout(timer);
        timer = setTimeout(() => {
          timer = null;
          // 앵커 환산에 필요한 크기는 이 시점에 다시 읽는다 — payload는 위치만
          // 담고 있고, onMoved가 리사이즈 직후(C9)에도 발화할 수 있어 크기가
          // 방금 바뀌었을 수 있다.
          void getCurrentWindow()
            .outerSize()
            .then((size) => {
              writeSavedAnchor(
                typeof localStorage === "undefined" ? null : localStorage,
                anchorOf(payload, { width: size.width, height: size.height }),
              );
            })
            .catch((err) => console.warn("mascot: failed to read size for anchor save", err));
        }, SAVE_DEBOUNCE_MS);
      })
      .then((f) => {
        if (disposed) f();
        else un = f;
      })
      .catch((err) => console.warn("mascot: failed to subscribe to window moves", err));

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
      .catch((err) => console.warn("mascot: failed to subscribe to scale changes", err));
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
      : `${state.agentId}|${state.seed}|${state.archetype}|${JSON.stringify(state.colors ?? null)}|${state.spriteUpdatedAt}|${dpr}`;

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
      .catch((err) => console.warn("mascot: failed to build sprite frames", err));
    return () => {
      cancelled = true;
    };
    // dpr은 spriteKey에 이미 포함돼 있지만, 본문이 직접 쓰므로 함께 선언한다.
  }, [spriteKey, dpr]);

  const backing = Math.round(MASCOT_SPRITE_PX * dpr);

  // ---- 신호등 레이아웃 변화 → 창 크기 재적용(C9) ----
  // 표시 칸 수(오버플로 칩 포함)·세로 여부·스프라이트 유무가 바뀔 때만
  // 발동한다 — 상태 emit마다가 아니라 이 파생 키가 실제로 바뀔 때만.
  const { shown: shownLights, overflowCount } = foldOverflow(state.lights);
  const displayedLightCount = shownLights.length + (overflowCount > 0 ? 1 : 0);
  const hasSprite = state.agentId !== null;
  const layoutKey = `${displayedLightCount}|${state.lightsVertical}|${hasSprite}`;

  useEffect(() => {
    // 스프라이트도 없고 칸도 없으면(완전히 숨김) 0×0으로 줄일 이유가 없다 —
    // 어차피 mascotBridge가 곧 visible:false로 창을 감춘다. 그대로 두면
    // 부팅 직후 HIDDEN_MASCOT_STATE 순간 0×0으로 리사이즈했다가 실제 상태가
    // 도착하는 즉시 다시 정상 크기로 되돌리는 낭비 왕복과, 그 사이
    // writeSavedAnchor가 0×0 기준으로 앵커를 오염시키는 것을 막는다.
    if (!hasSprite && displayedLightCount === 0) return;
    const timer = setTimeout(() => {
      void applyMascotLayout(displayedLightCount, state.lightsVertical, hasSprite, dprRef.current).catch(
        (err) => console.warn("mascot: failed to apply layout", err),
      );
    }, LAYOUT_DEBOUNCE_MS);
    return () => clearTimeout(timer);
    // layoutKey가 위 세 값을 그대로 인코딩한다 — dpr은 dprRef로 최신값을
    // 읽으므로 dpr 변화 자체는 재실행 트리거가 아니다(의도된 것, §C9).
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [layoutKey]);

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
      .catch((err) => console.warn("mascot: failed to start dragging", err));
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
      .catch((err) => console.warn("mascot: activation failed", err));
  };

  // ---- 신호등 램프 클릭: 대표 에이전트를 활성화(clickAgentId=null이면 no-op) ----
  const onLightClick = (light: MascotLight) => {
    if (light.clickAgentId === null) return;
    void tauriApi
      .mascotActivate(light.clickAgentId)
      .catch((err) => console.warn("mascot: light activation failed", err));
  };

  return (
    <div className="mascot-root">
      {state.agentId !== null && (
        <div className="mascot-sprite-wrap">
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
      )}
      {state.lights.length > 0 && (
        <div
          className={`mascot-lights${state.lightsVertical ? " mascot-lights-vertical" : " mascot-lights-horizontal"}`}
        >
          {shownLights.map((light) => (
            <div
              key={light.id}
              className={`mascot-light mascot-light-${light.state}`}
              title={light.label}
              onClick={() => onLightClick(light)}
            >
              {light.state === "working" && (
                <svg width="18" height="18" viewBox="0 0 18 18" aria-hidden="true">
                  <polygon points="6,4 14,9 6,14" fill="#eafff0" />
                </svg>
              )}
              {light.state === "attention" && <span aria-hidden="true">!</span>}
            </div>
          ))}
          {overflowCount > 0 && (
            <div className="mascot-light-chip">{`+${overflowCount}`}</div>
          )}
        </div>
      )}
    </div>
  );
}
