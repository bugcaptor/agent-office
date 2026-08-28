// src/renderer/mascot/layout.ts
//
// 신호등 strip을 붙였을 때 마스코트 창이 가져야 할 크기와, 칸이 넘칠 때
// 어떻게 접을지를 계산하는 순수 함수(docs/mascot-lights-design.md §5.1).
// 렌더 관심사(몇 칸까지 그릴지)는 여기 mascot 쪽에 두고, main 렌더러(store/
// mascotLights.ts)는 항상 전체 목록을 보낸다 — 프로토콜을 단순하게 유지한다.
//
// 치수는 전부 논리 px. 창 실제 적용(물리 px 환산·set_mascot_layout 호출)은
// MascotApp.tsx의 몫이라 여기서는 손대지 않는다.

import {
  LIGHT_GAP,
  LIGHT_STRIP_PAD,
  LIGHT_TILE_H,
  LIGHT_TILE_H_TALL,
  LIGHT_TILE_W,
  LIGHT_TILE_W_WIDE,
  MASCOT_WINDOW_H,
  MASCOT_WINDOW_W,
  MAX_LIGHTS,
} from "./protocol";
import {
  anchorOf,
  clampToArea,
  isOnMonitor,
  topLeftOf,
  type MonitorRect,
  type Point,
  type Size,
} from "./position";

/** wide(작업명 라벨) 칸일 때의 타일 폭 — 아니면 기본 폭. */
function tileW(wide: boolean): number {
  return wide ? LIGHT_TILE_W_WIDE : LIGHT_TILE_W;
}

/** tall(프로젝트+작업 두 줄 라벨) 칸일 때의 타일 높이 — 아니면 기본 높이. */
function tileH(tall: boolean): number {
  return tall ? LIGHT_TILE_H_TALL : LIGHT_TILE_H;
}

/** 램프가 나열되는 방향의 strip 길이(논리 px). 칸이 없으면 0(strip 자체가 없다).
 *  칸이 원이 아니라 [얼굴 + 이름] 타일이 되면서 가로/세로 치수가 달라졌다 —
 *  나열 방향에 따라 쓰는 변이 다르다. */
function stripLength(count: number, vertical: boolean, wide: boolean, tall: boolean): number {
  if (count <= 0) return 0;
  const tile = vertical ? tileH(tall) : tileW(wide);
  return LIGHT_STRIP_PAD * 2 + tile * count + LIGHT_GAP * (count - 1);
}

/** strip의 직교(두께) 방향 길이(논리 px). 칸이 없으면 0. */
function stripThickness(count: number, vertical: boolean, wide: boolean, tall: boolean): number {
  if (count <= 0) return 0;
  return (vertical ? tileW(wide) : tileH(tall)) + LIGHT_STRIP_PAD * 2;
}

/**
 * 마스코트 창이 가져야 할 크기(논리 px) — 스프라이트 유무·칸 수·배열 방향으로
 * 결정한다. 가로 모드는 strip이 폭 방향으로 늘어나고 두께(타일 높이 + 여백)가
 * 스프라이트 아래에 얹히며, 세로 모드는 반대다.
 */
export function computeMascotLayout(input: {
  lightCount: number;
  vertical: boolean;
  hasSprite: boolean;
  /** true = 칸을 wide(96px) 타일로 계산한다(`mascotLightsLabel==="task"`). */
  wide: boolean;
  /** true = 칸을 tall(60px) 타일로 계산한다(`mascotLightsLabel==="projecttask"`). */
  tall: boolean;
}): { width: number; height: number } {
  const { lightCount, vertical, hasSprite, wide, tall } = input;
  const spriteW = hasSprite ? MASCOT_WINDOW_W : 0;
  const spriteH = hasSprite ? MASCOT_WINDOW_H : 0;
  if (vertical) {
    return {
      width: Math.max(spriteW, stripThickness(lightCount, true, wide, tall)),
      height: spriteH + stripLength(lightCount, true, wide, tall),
    };
  }
  return {
    width: Math.max(spriteW, stripLength(lightCount, false, wide, tall)),
    height: spriteH + stripThickness(lightCount, false, wide, tall),
  };
}

/**
 * 넘치는 칸을 접는다(결정 8) — max를 넘으면 앞 (max-1)칸만 보이고 나머지는
 * `+k` 오버플로 칩 하나로 뭉친다. max 이하면 전부 보인다(overflowCount=0).
 */
export function foldOverflow<T>(
  lights: readonly T[],
  max: number = MAX_LIGHTS,
): { shown: T[]; overflowCount: number } {
  if (lights.length <= max) {
    return { shown: [...lights], overflowCount: 0 };
  }
  const shownCount = max - 1;
  return {
    shown: lights.slice(0, shownCount),
    overflowCount: lights.length - shownCount,
  };
}

/**
 * C9(동적 리사이즈): 새 논리 레이아웃 + **현재** 창의 물리 위치/크기로부터
 * `set_mascot_layout`에 넘길 물리 px 사각형을 계산한다. 앵커(하단중앙)는
 * 저장값이 아니라 호출 시점의 `outerPosition()`/`outerSize()`에서 다시 뽑는다
 * — 사용자가 방금 끌어다 둔 자리를 존중하기 위해서다(계획 C9). 화면 밖으로
 * 밀려나면 창이 현재 걸쳐 있는 모니터(없으면 주 모니터, 그마저 없으면 목록의
 * 첫 모니터) 안으로 clampToArea가 되돌린다. Tauri API를 전혀 부르지 않는
 * 순수 함수라 vitest로 앵커→top-left→클램프 전 과정을 검증할 수 있다.
 */
export function computeMascotWindowRect(input: {
  lightCount: number;
  vertical: boolean;
  hasSprite: boolean;
  /** true = 칸을 wide(96px) 타일로 계산한다(`mascotLightsLabel==="task"`). */
  wide: boolean;
  /** true = 칸을 tall(60px) 타일로 계산한다(`mascotLightsLabel==="projecttask"`). */
  tall: boolean;
  /** 물리 px 환산 배율. */
  dpr: number;
  /** 리사이즈 직전 창의 물리 px 좌상단. */
  currentPos: Point;
  /** 리사이즈 직전 창의 물리 px 크기. */
  currentSize: Size;
  monitors: ReadonlyArray<MonitorRect>;
  primary: MonitorRect | null;
}): { width: number; height: number; x: number; y: number } {
  const {
    lightCount,
    vertical,
    hasSprite,
    wide,
    tall,
    dpr,
    currentPos,
    currentSize,
    monitors,
    primary,
  } = input;
  const logical = computeMascotLayout({ lightCount, vertical, hasSprite, wide, tall });
  const physicalSize: Size = {
    width: Math.round(logical.width * dpr),
    height: Math.round(logical.height * dpr),
  };
  const anchor = anchorOf(currentPos, currentSize);
  const topLeft = topLeftOf(anchor, physicalSize);
  const monitor =
    monitors.find((m) => isOnMonitor(currentPos, currentSize, m)) ?? primary ?? monitors[0] ?? null;
  const pos = monitor ? clampToArea(topLeft, physicalSize, monitor) : topLeft;
  // B1: 논리 폭은 짝수(120, 30, 24n+6)지만 dpr 1.5류에서 물리 strip 폭은
  // 항상 홀수라(예: 45×1.5=67.5→반올림 후에도 앵커 역산 중 절반 나누기에서
  // .5가 남는다) topLeft가 x.5로 끝날 수 있다. Rust `set_mascot_layout`의
  // x/y는 i32라 소수점 인자를 주면 serde_json 역직렬화가 거부해 호출 자체가
  // 실패한다(창이 다시는 리사이즈되지 않는다) — 클램프 이후 정수로 반올림해
  // 그 경로를 원천 차단한다.
  return {
    width: physicalSize.width,
    height: physicalSize.height,
    x: Math.round(pos.x),
    y: Math.round(pos.y),
  };
}
