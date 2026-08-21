// src/renderer/office/scenes/sceneColor.ts
//
// 씬 고유 팔레트 × 앱 테마의 교차점 — 순수 함수만 있는 색 변환 계층.
//
// office 씬은 테마 팔레트(theme.pixi)를 그대로 쓰므로 여기를 거치지 않는다.
// beach/valley는 "한낮의 원색"으로 한 벌만 그려 두고, 테마에 따라 그 한 벌을
// 자동 변환해 앱 전체 색감과 어울리게 만든다(테마마다 손으로 3벌씩 칠하지
// 않는다 — 씬이 늘어날수록 유지 비용이 선형으로 커지므로).
//
//   daylight / sakura -> identity (원색 그대로. 밝은 테마라 그대로 어울린다)
//   midnight          -> dusk     (채도·명도를 낮추고 야청색으로 기울인 황혼)
//   pipboy            -> green    (휘도만 남겨 인광 초록 램프로 양자화)
//
// 전 함수 순수·결정적: 같은 입력 → 같은 0xRRGGBB. 입력 팔레트는 절대
// 변형하지 않는다(레지스트리의 씬 팔레트는 모듈 상수라 공유된다).
import type { ThemeId } from "../../theme/themes";

export type SceneColorMode = "identity" | "dusk" | "green";

/** 테마 id → 씬 색 변환 모드. 테마가 늘면 여기만 손대면 된다. */
export function sceneColorMode(themeId: ThemeId): SceneColorMode {
  switch (themeId) {
    case "midnight":
      return "dusk";
    case "pipboy":
      return "green";
    case "daylight":
    case "sakura":
      return "identity";
  }
}

const clamp255 = (v: number): number => (v < 0 ? 0 : v > 255 ? 255 : Math.round(v));

const rgb = (c: number): [number, number, number] => [(c >> 16) & 0xff, (c >> 8) & 0xff, c & 0xff];

const pack = (r: number, g: number, b: number): number =>
  (clamp255(r) << 16) | (clamp255(g) << 8) | clamp255(b);

/** ITU-R BT.601 휘도 — 픽셀아트 대비 판단에 충분하고 정수 연산으로 안정적. */
const luma = (r: number, g: number, b: number): number => 0.299 * r + 0.587 * g + 0.114 * b;

const mix = (a: number, b: number, t: number): number => a + (b - a) * t;

// 황혼 변환 상수. 채도를 절반 이하로 죽이고 전체를 어둡게 깐 뒤 야청색을
// 옅게 덮는다 — midnight 테마의 오피스 팔레트(어두운 남보라)와 같은 결.
const DUSK_DESATURATE = 0.45;
const DUSK_DARKEN = 0.46;
const DUSK_TINT = [0x2a, 0x33, 0x58] as const;
const DUSK_TINT_AMOUNT = 0.16;

// 인광 초록 램프의 양끝 + 단계 수(pipboy 테마의 CRT 톤).
// 감마: 휘도를 그대로 쓰면 모래사장·설산 같은 밝은 씬이 통째로 하얗게 떠서
// pipboy의 어두운 앱 크롬(--bg-base #071209)과 따로 논다. 지수를 올려
// 중간~밝은 영역을 눌러 오피스 씬의 pipboy 팔레트와 비슷한 대역에 맞춘다.
const GREEN_DARK = [0x06, 0x14, 0x0a] as const;
const GREEN_BRIGHT = [0x63, 0xe0, 0x8a] as const;
const GREEN_GAMMA = 2.4;
// 단계가 적을수록 계단이 도드라진다 — 바닥 체커처럼 원본 차이가 미세한 두 색이
// 서로 다른 단으로 갈라져 무늬가 요란해지므로, 계단감은 남기되 8단으로 완만하게.
const GREEN_STEPS = 8;

/** 색 하나를 변환한다. `identity`는 입력을 그대로 돌려준다. */
export function adaptColor(color: number, mode: SceneColorMode): number {
  if (mode === "identity") return color >>> 0;
  const [r, g, b] = rgb(color);
  const y = luma(r, g, b);
  if (mode === "dusk") {
    const dr = mix(mix(r, y, DUSK_DESATURATE) * DUSK_DARKEN, DUSK_TINT[0], DUSK_TINT_AMOUNT);
    const dg = mix(mix(g, y, DUSK_DESATURATE) * DUSK_DARKEN, DUSK_TINT[1], DUSK_TINT_AMOUNT);
    const db = mix(mix(b, y, DUSK_DESATURATE) * DUSK_DARKEN, DUSK_TINT[2], DUSK_TINT_AMOUNT);
    return pack(dr, dg, db);
  }
  // green: 휘도에 감마를 먹인 뒤 GREEN_STEPS 단계로 양자화해 램프 위 한 점으로
  // 보낸다. 양자화가 있어야 인접한 두 색이 같은 초록으로 뭉개지지 않고 CRT
  // 특유의 계단 톤이 남는다.
  const t = Math.pow(y / 255, GREEN_GAMMA);
  const q = Math.round(t * (GREEN_STEPS - 1)) / (GREEN_STEPS - 1);
  return pack(
    mix(GREEN_DARK[0], GREEN_BRIGHT[0], q),
    mix(GREEN_DARK[1], GREEN_BRIGHT[1], q),
    mix(GREEN_DARK[2], GREEN_BRIGHT[2], q),
  );
}

/** 팔레트(문자열 키 → 0xRRGGBB) 전체를 변환한 **새** 객체를 만든다. */
export function adaptPalette<P extends Record<string, number>>(pal: P, mode: SceneColorMode): P {
  if (mode === "identity") return { ...pal };
  const out = {} as Record<string, number>;
  for (const key of Object.keys(pal)) out[key] = adaptColor(pal[key], mode);
  return out as P;
}
