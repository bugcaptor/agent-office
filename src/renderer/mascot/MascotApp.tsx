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
import type { MascotLightsFace } from "@shared/types";
import { tauriApi } from "../ipc/tauriApi";
import {
  HIDDEN_MASCOT_STATE,
  LIGHT_AVATAR_PX,
  MASCOT_ANIM_IDLE_MS,
  MASCOT_SPRITE_PX,
  maxLightsFor,
  parseMascotState,
  type MascotLight,
  type MascotState,
} from "./protocol";
import { avatarKey, drawAvatar, loadAvatarFrames, loadPortraitUrl, portraitKey } from "./avatar";
import { computeMascotWindowRect, foldOverflow } from "./layout";
import { loadMascotFrames, type MascotFrames } from "./sheet";
import { createDragDetector } from "./drag";
import {
  anchorOf,
  readSavedAnchor,
  resolveAnchoredPosition,
  topLeftOf,
  writeSavedAnchor,
  type Anchor,
  type MonitorRect,
  type Point,
  type Size,
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

/** 위치 복원(또는 실패 시 실측)이 확정한 결과 — 컴포넌트가 이후 계속 참조할
 *  "유일한 앵커 원천"의 초기값이다(D1). `appliedPos`는 이 함수가 직접
 *  `setPosition`을 불렀을 때만 채워진다 — onMoved 핸들러가 그 이동을
 *  사용자 이동과 구분해 앵커를 되먹임하지 않게 하는 데 쓰인다. */
interface RestoreResult {
  anchor: Anchor;
  size: Size;
  appliedPos: Point | null;
}

/** 저장된 앵커로 창을 옮긴다(없거나 화면 밖이면 주 모니터 우하단). */
async function restorePosition(): Promise<RestoreResult> {
  const win = getCurrentWindow();
  const [size, monitors, primary] = await Promise.all([
    win.outerSize(),
    availableMonitors(),
    primaryMonitor(),
  ]);
  const currentSize = { width: size.width, height: size.height };
  const saved = readSavedAnchor(
    typeof localStorage === "undefined" ? null : localStorage,
    currentSize,
  );
  const pos = resolveAnchoredPosition(
    saved,
    currentSize,
    monitors.map(toRect),
    primary ? toRect(primary) : null,
  );
  if (pos) {
    // B1: 물리 px는 정수여야 한다(Rust set_position은 i32) — 앵커 역산이
    // 홀수 크기에서 .5로 끝날 수 있다.
    const rounded = { x: Math.round(pos.x), y: Math.round(pos.y) };
    await win.setPosition(new PhysicalPosition(rounded.x, rounded.y));
    return { anchor: anchorOf(rounded, currentSize), size: currentSize, appliedPos: rounded };
  }
  // 모니터 조회가 실패해 옮길 곳을 못 정했다 — 창은 OS 기본 위치에 그대로
  // 있다. setPosition을 부르지 않았으므로 그 실제 위치를 실측해 앵커를
  // 만든다(허상 앵커 방지). 이후 onMoved는 전부 진짜 이동으로 취급한다.
  const actual = await win.outerPosition();
  return {
    anchor: anchorOf({ x: actual.x, y: actual.y }, currentSize),
    size: currentSize,
    appliedPos: null,
  };
}

/**
 * C9: 신호등 레이아웃(칸 수·방향·스프라이트 유무)이 바뀌었을 때 창 크기를
 * 다시 맞춘다. **앵커는 `anchor`/`anchorSize` 인자(컴포넌트가 들고 있는
 * ref)를 소비만 하고 갱신하지 않는다** — 예전에는 호출 시점의
 * `outerPosition()`/`outerSize()`에서 앵커를 다시 뽑았는데, B1이 물리 px를
 * 정수로 반올림하는 순간 그 반올림 오차가 사이클마다 앵커에 되먹임돼
 * 한쪽으로 드리프트했다(리뷰 D1). 앵커 갱신은 컴포넌트의 `onMoved`(사용자
 * 이동)만의 몫이다. 계산 자체는 순수 함수(`computeMascotWindowRect`)에
 * 맡기고, 여기서는 모니터 조회 + 적용만 한다.
 */
async function computeAppliedRect(
  lightCount: number,
  vertical: boolean,
  hasSprite: boolean,
  wide: boolean,
  dpr: number,
  anchor: Anchor,
  anchorSize: Size,
): Promise<{ width: number; height: number; x: number; y: number }> {
  const [monitors, primary] = await Promise.all([availableMonitors(), primaryMonitor()]);
  // topLeftOf는 anchorOf의 정확한 역함수라(반올림 없음) anchorSize로 무엇을
  // 넘기든 computeMascotWindowRect 내부의 anchorOf(topLeft, anchorSize)가
  // `anchor`를 오차 없이 그대로 복원한다 — 그래서 "지금 창의 실제 크기"가
  // 아니라 아무 유효한 크기(여기선 마지막으로 적용한 크기)를 넘겨도 안전하다.
  const currentPos = topLeftOf(anchor, anchorSize);
  return computeMascotWindowRect({
    lightCount,
    vertical,
    hasSprite,
    wide,
    dpr,
    currentPos,
    currentSize: anchorSize,
    monitors: monitors.map(toRect),
    primary: primary ? toRect(primary) : null,
  });
}

/**
 * 신호등 칸의 얼굴. 아바타 좌표가 있으면 대표 에이전트의 머리를 잘라 그리고,
 * 없으면(세션 없는 프로젝트 폴더) 이름 첫 글자 원판으로 대체한다. 캔버스
 * 백킹 해상도는 dpr을 따라가고, 좌표가 바뀔 때만 다시 그린다.
 *
 * `face==="portrait"`이고 대표 에이전트에 초상이 있으면(`portraitUpdatedAt`
 * 있음) 스프라이트 캔버스 대신 초상 이미지를 띄운다. 로드가 아직 끝나지
 * 않았거나 실패했으면(null) 항상 그려 둔 스프라이트 캔버스가 그대로 보인다
 * — 깜빡임 없는 폴백.
 */
function LightFace({ light, dpr, face }: { light: MascotLight; dpr: number; face: MascotLightsFace }) {
  const ref = useRef<HTMLCanvasElement | null>(null);
  const avatar = light.avatar;
  const key = avatar === null ? null : avatarKey(avatar, dpr);
  const backing = Math.round(LIGHT_AVATAR_PX * dpr);
  const wantsPortrait = face === "portrait" && avatar !== null && avatar.portraitUpdatedAt !== null;
  const pKey = wantsPortrait && avatar !== null ? portraitKey(avatar) : null;
  const [portraitUrl, setPortraitUrl] = useState<string | null>(null);

  useEffect(() => {
    if (avatar === null) return;
    let cancelled = false;
    void loadAvatarFrames(avatar, dpr).then((frames) => {
      if (cancelled || frames === null) return;
      const canvas = ref.current;
      if (canvas) drawAvatar(canvas, frames);
    });
    return () => {
      cancelled = true;
    };
    // key가 avatar의 모든 얼굴 좌표 + dpr을 인코딩한다.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [key, backing]);

  useEffect(() => {
    if (pKey === null || avatar === null) {
      setPortraitUrl(null);
      return;
    }
    let cancelled = false;
    void loadPortraitUrl(avatar).then((url) => {
      if (!cancelled) setPortraitUrl(url);
    });
    return () => {
      cancelled = true;
    };
    // pKey가 agentId + portraitUpdatedAt을 인코딩한다.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [pKey]);

  if (avatar === null) {
    // 비번역 텍스트만 쓴다(설계 결정 9) — 이름의 첫 글자다.
    return <span className="mascot-light-initial">{[...light.label][0] ?? "?"}</span>;
  }
  if (wantsPortrait && portraitUrl !== null) {
    return <img className="mascot-light-portrait" src={portraitUrl} alt="" draggable={false} />;
  }
  return (
    <canvas
      ref={ref}
      className="mascot-light-avatar"
      width={backing}
      height={backing}
      style={{ width: LIGHT_AVATAR_PX, height: LIGHT_AVATAR_PX }}
    />
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
  // D1: "유일한 앵커 원천". restorePosition이 마운트 시 한 번 채우고, 이후로는
  // onMoved(사용자 이동)에서만 갱신한다 — applyLayout(C9 리사이즈)은 이 값을
  // topLeftOf로 소비만 할 뿐 절대 갱신하지 않는다(되먹임 차단).
  const anchorRef = useRef<Anchor | null>(null);
  // anchorRef와 짝을 이루는 크기. computeAppliedRect가 currentPos를 역산하는
  // 데만 쓰이고(어떤 값이든 anchor 복원은 정확하다), 모니터 판정 정확도를 위해
  // 최근 실측/적용 크기로 유지한다.
  const lastSizeRef = useRef<Size | null>(null);
  // 마지막으로 우리 자신이 setPosition한 좌표 — onMoved 핸들러가 "이 이동이
  // 방금 우리가 건 것인가"를 판정하는 기준이다(D1). 사용자 드래그로 그 좌표와
  // 다른 곳으로 이동한 것이 확인되면 그때만 앵커를 갱신한다.
  const lastAppliedPosRef = useRef<Point | null>(null);
  // C1: restorePosition의 완료를 applyLayout이 기다리는 관문. 복원이 끝나기
  // 전에 레이아웃을 적용하면 복원 전 기본 위치가 앵커로 굳어 영구 저장된다.
  const restorePositionRef = useRef<Promise<void> | null>(null);
  // C2: applyLayout 세대 카운터. IPC 왕복 중에 layoutKey가 또 바뀌면 늦게
  // 끝난 옛 호출이 새 레이아웃을 덮지 못하게 막는다.
  const seqRef = useRef(0);

  /**
   * C9 배선: 신호등 레이아웃 변화 → 창 크기 재적용. anchorRef/lastSizeRef를
   * 소비만 하고(D1), gen이 최신일 때만 결과를 반영한다(C2). C1: 복원이 아직
   * 안 끝났으면(실패해도) 기다린 뒤 진행한다.
   */
  const applyLayout = async (
    lightCount: number,
    vertical: boolean,
    hasSprite: boolean,
    wide: boolean,
    dpr: number,
    gen: number,
  ): Promise<void> => {
    if (restorePositionRef.current) await restorePositionRef.current.catch(() => {});
    if (anchorRef.current === null || lastSizeRef.current === null) return; // 복원 자체가 실패
    const rect = await computeAppliedRect(
      lightCount,
      vertical,
      hasSprite,
      wide,
      dpr,
      anchorRef.current,
      lastSizeRef.current,
    );
    if (gen !== seqRef.current) return; // C2: 더 최신 요청이 이미 지나갔다 — 이 결과는 버린다
    lastAppliedPosRef.current = { x: rect.x, y: rect.y };
    lastSizeRef.current = { width: rect.width, height: rect.height };
    await tauriApi.setMascotLayout(rect.width, rect.height, rect.x, rect.y);
  };

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
    const promise = restorePosition()
      .catch(async (err) => {
        console.warn("mascot: failed to restore position", err);
        // 복원 자체가 실패해도 앵커는 확보한다 — 그러지 않으면 이후 레이아웃
        // 적용이 anchorRef===null 가드에 걸려 영구히 멈춘다.
        const win = getCurrentWindow();
        const [pos, size] = await Promise.all([win.outerPosition(), win.outerSize()]);
        const currentSize = { width: size.width, height: size.height };
        return {
          anchor: anchorOf({ x: pos.x, y: pos.y }, currentSize),
          size: currentSize,
          appliedPos: null as Point | null,
        };
      })
      .then(({ anchor, size, appliedPos }) => {
        anchorRef.current = anchor;
        lastSizeRef.current = size;
        lastAppliedPosRef.current = appliedPos;
      });
    restorePositionRef.current = promise;

    let timer: ReturnType<typeof setTimeout> | null = null;
    let un: (() => void) | null = null;
    let disposed = false;
    void getCurrentWindow()
      .onMoved(({ payload }) => {
        // D1: 이 이동이 우리 자신(복원 setPosition 또는 C9 리사이즈)이 만든
        // 것이면 앵커를 되먹임하지 않는다 — 그렇지 않으면 B1의 반올림 오차가
        // 앵커에 쌓여 사이클마다 한쪽으로 드리프트한다.
        const applied = lastAppliedPosRef.current;
        if (
          applied !== null &&
          Math.abs(payload.x - applied.x) <= 1 &&
          Math.abs(payload.y - applied.y) <= 1
        ) {
          return;
        }
        if (timer !== null) clearTimeout(timer);
        timer = setTimeout(() => {
          timer = null;
          // 앵커 환산에 필요한 크기는 이 시점에 다시 읽는다 — payload는 위치만
          // 담고 있다.
          void getCurrentWindow()
            .outerSize()
            .then((size) => {
              const currentSize = { width: size.width, height: size.height };
              const anchor = anchorOf(payload, currentSize);
              anchorRef.current = anchor;
              lastSizeRef.current = currentSize;
              writeSavedAnchor(typeof localStorage === "undefined" ? null : localStorage, anchor);
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
  // 표시 칸 수(오버플로 칩 포함)·세로 여부·스프라이트 유무·wide 여부가 바뀔
  // 때만 발동한다 — 상태 emit마다가 아니라 이 파생 키가 실제로 바뀔 때만.
  // maxLightsFor: wide(작업명 라벨)이고 가로 배열이면 상한을 8→5로 낮춘다
  // (칸이 넓어져 8칸이면 창이 지나치게 길어진다) — 세로 배열은 줄이지 않는다.
  const { shown: shownLights, overflowCount } = foldOverflow(
    state.lights,
    maxLightsFor(state.lightsWide, state.lightsVertical),
  );
  const displayedLightCount = shownLights.length + (overflowCount > 0 ? 1 : 0);
  const hasSprite = state.agentId !== null;
  const layoutKey = `${displayedLightCount}|${state.lightsVertical}|${hasSprite}|${state.lightsWide}`;

  useEffect(() => {
    // 스프라이트도 없고 칸도 없으면(완전히 숨김) 0×0으로 줄일 이유가 없다 —
    // 어차피 mascotBridge가 곧 visible:false로 창을 감춘다. 그대로 두면
    // 부팅 직후 HIDDEN_MASCOT_STATE 순간 0×0으로 리사이즈했다가 실제 상태가
    // 도착하는 즉시 다시 정상 크기로 되돌리는 낭비 왕복과, 그 사이
    // writeSavedAnchor가 0×0 기준으로 앵커를 오염시키는 것을 막는다.
    if (!hasSprite && displayedLightCount === 0) return;
    const timer = setTimeout(() => {
      const gen = ++seqRef.current; // C2: 이 호출의 세대를 못박는다
      void applyLayout(
        displayedLightCount,
        state.lightsVertical,
        hasSprite,
        state.lightsWide,
        dprRef.current,
        gen,
      ).catch((err) => console.warn("mascot: failed to apply layout", err));
    }, LAYOUT_DEBOUNCE_MS);
    return () => clearTimeout(timer);
    // layoutKey가 위 네 값을 그대로 인코딩한다 — dpr은 dprRef로 최신값을
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

  // ---- strip 드래그(D2): 스프라이트가 없어 strip만 뜬 창은 캔버스가 아예
  // 없어 잡을 곳이 없었다 — 창을 한 번 잘못된 자리에 두면 영구히 못 옮긴다.
  // 캔버스와 같은 detector 배선을 strip 컨테이너에도 붙이되, 램프(자식) 위
  // 포인터다운은 무시한다 — `onLightClick`이 그쪽을 전담하고(design §6, 램프
  // 위 드래그 시작은 v1 비지원), `data-tauri-drag-region`은 클릭을 통째로
  // 삼켜 램프 클릭이 죽으므로 쓰지 않는다(drag.ts 헤더 참고).
  const stripDetector = useRef(createDragDetector()).current;
  const onStripPointerDown = (e: React.PointerEvent<HTMLDivElement>) => {
    // 칸이 [얼굴 + 이름] 타일이 되면서 strip 여백이 6px밖에 남지 않아, 예전처럼
    // 타일 위 pointerdown을 무시하면 창을 잡을 곳이 사실상 사라진다. 버블링으로
    // 올라온 타일발 pointerdown도 드래그 후보로 받되 **포인터 캡처는 걸지 않는다**
    // — 캡처를 걸면 이어지는 click까지 strip으로 리타깃돼 칸별 onClick이 죽는다.
    if (e.target === e.currentTarget) e.currentTarget.setPointerCapture(e.pointerId);
    stripDetector.down(e.screenX, e.screenY);
  };
  const onStripPointerMove = (e: React.PointerEvent<HTMLDivElement>) => {
    if (stripDetector.move(e.screenX, e.screenY) !== "start-drag") return;
    if (e.currentTarget.hasPointerCapture(e.pointerId)) {
      e.currentTarget.releasePointerCapture(e.pointerId);
    }
    void getCurrentWindow()
      .startDragging()
      .catch((err) => console.warn("mascot: failed to start dragging(strip)", err));
  };
  const onStripPointerUp = (e: React.PointerEvent<HTMLDivElement>) => {
    if (e.currentTarget.hasPointerCapture(e.pointerId)) {
      e.currentTarget.releasePointerCapture(e.pointerId);
    }
    stripDetector.up(); // 클릭 판정은 각 램프의 onClick(네이티브 클릭)에 맡긴다.
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
          className={`mascot-lights${state.lightsVertical ? " mascot-lights-vertical" : " mascot-lights-horizontal"}${state.lightsWide ? " mascot-lights-wide" : ""}`}
          onPointerDown={onStripPointerDown}
          onPointerMove={onStripPointerMove}
          onPointerUp={onStripPointerUp}
          onPointerCancel={() => stripDetector.cancel()}
        >
          {shownLights.map((light) => (
            <div
              key={light.id}
              className={`mascot-light mascot-light-${light.state}${light.clickAgentId === null ? " mascot-light-noclick" : ""}`}
              title={light.tooltip || light.label}
              onClick={() => onLightClick(light)}
            >
              <div className="mascot-light-face">
                <LightFace light={light} dpr={dpr} face={state.lightsFace} />
                {light.state === "working" && (
                  <span className="mascot-light-mark" aria-hidden="true">
                    <svg width="10" height="10" viewBox="0 0 18 18">
                      <polygon points="6,4 14,9 6,14" fill="#eafff0" />
                    </svg>
                  </span>
                )}
                {light.state === "attention" && (
                  <span className="mascot-light-mark" aria-hidden="true">
                    !
                  </span>
                )}
              </div>
              {/* 이름은 대상(프로젝트 폴더명 또는 에이전트명) 원문 — 비번역(결정 9). */}
              <div className="mascot-light-name">{light.label}</div>
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
