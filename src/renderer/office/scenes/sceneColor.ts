// src/renderer/office/scenes/sceneColor.ts
//
// 씬 고유 팔레트 × 앱 테마의 교차점 — 순수 함수만 있는 색 변환 계층.
//
// office 씬은 테마 팔레트(theme.pixi)를 그대로 쓰므로 여기를 거치지 않는다.
// beach/valley는 "한낮의 원색"으로 한 벌만 그려 두고, 테마에 따라 그 한 벌을
// 자동 변환해 앱 전체 색감과 어울리게 만든다(테마마다 손으로 3벌씩 칠하지
// 않는다 — 씬이 늘어날수록 유지 비용이 선형으로 커지므로).
//
//   daylight / sakura -> soft     (채도를 덜고 밝은 쪽에 종이빛 안개를 씌운 한낮)
//   midnight          -> dusk     (채도·명도를 낮추고 야청색으로 기울인 황혼)
//   pipboy            -> green    (휘도만 남겨 인광 초록 램프로 양자화)
//
// identity는 변환 없음 — 현재 어느 테마도 쓰지 않지만 "원색 그대로"를
// 표현할 수단으로 남겨 둔다(씬 팔레트의 기준점이자 테스트의 항등원).
//
// 전 함수 순수·결정적: 같은 입력 → 같은 0xRRGGBB. 입력 팔레트는 절대
// 변형하지 않는다(레지스트리의 씬 팔레트는 모듈 상수라 공유된다).
import type { ThemeId } from "../../theme/themes";

export type SceneColorMode = "identity" | "soft" | "dusk" | "green";

/** 테마 id → 씬 색 변환 모드. 테마가 늘면 여기만 손대면 된다. */
export function sceneColorMode(themeId: ThemeId): SceneColorMode {
  switch (themeId) {
    case "midnight":
      return "dusk";
    case "pipboy":
      return "green";
    case "daylight":
    case "sakura":
      return "soft";
  }
}

const clamp255 = (v: number): number => (v < 0 ? 0 : v > 255 ? 255 : Math.round(v));

const rgb = (c: number): [number, number, number] => [(c >> 16) & 0xff, (c >> 8) & 0xff, c & 0xff];

const pack = (r: number, g: number, b: number): number =>
  (clamp255(r) << 16) | (clamp255(g) << 8) | clamp255(b);

/** ITU-R BT.601 휘도 — 픽셀아트 대비 판단에 충분하고 정수 연산으로 안정적. */
const luma = (r: number, g: number, b: number): number => 0.299 * r + 0.587 * g + 0.114 * b;

const mix = (a: number, b: number, t: number): number => a + (b - a) * t;

// 한낮 변환 상수. 씬 팔레트는 "한낮의 원색"으로 칠해져 있어 밝은 테마에서
// 그대로 쓰면 채도가 앱 크롬(웜 크림/연분홍)보다 한참 세게 튄다 — 오래 보면
// 눈이 아프다. 그래서 채도를 조금 덜고 종이빛 안개를 옅게 씌운다.
//
// 안개의 세기는 밝기에 비례한다(√휘도): 눈을 찌르는 건 넓게 깔린 밝고 짙은
// 면(모래·바다·천막)이지, 윤곽을 잡아 주는 어두운 선이 아니다. 균일하게
// 씌우면 어두운 디테일까지 들떠 그림이 뿌예지므로 밝은 쪽만 눌러 대비를
// 남긴다.
const SOFT_DESATURATE = 0.3;
const SOFT_VEIL = [0xf7, 0xf1, 0xe4] as const;
const SOFT_VEIL_AMOUNT = 0.18;

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
  if (mode === "soft") {
    const veil = SOFT_VEIL_AMOUNT * Math.sqrt(y / 255);
    return pack(
      mix(mix(r, y, SOFT_DESATURATE), SOFT_VEIL[0], veil),
      mix(mix(g, y, SOFT_DESATURATE), SOFT_VEIL[1], veil),
      mix(mix(b, y, SOFT_DESATURATE), SOFT_VEIL[2], veil),
    );
  }
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

/**
 * 씬 팔레트 전체의 채도를 한 번에 깎는 기본 감쇠량.
 *
 * 씬은 "구경거리"가 아니라 캐릭터가 걸어 다니는 **무대**다. 무대가 캐릭터만큼
 * 진한 색을 쓰면 눈이 어디를 봐야 할지 정하지 못해 화면 전체가 어지럽다.
 * 캐릭터 스프라이트는 이 변환을 거치지 않으므로, 배경의 채도를 조금만 깎아도
 * 사람이 배경에서 떠오른다.
 *
 * 다만 절반 넘게 깎으면(0.55) 무대가 아니라 빛바랜 사진이 된다 — 캐릭터는
 * 확실히 떠오르지만 풍경 자체가 죽어 어느 씬을 골라도 같은 잿빛으로 보인다.
 * 사람이 떠오르는 데 필요한 것은 "회색 배경"이 아니라 캐릭터보다 한 단계
 * 낮은 채도이므로, 색기는 남기고 한 단계만 눌러 둔다. 넓은 면의 어지러움은
 * 채도가 아니라 무늬의 문제이고 그건 quietPalette가 따로 맡는다.
 */
export const SCENE_CHROMA_CUT = 0.28;

/** 색 하나의 채도만 깎는다(휘도는 유지). 0=그대로, 1=완전한 회색. */
export function desaturateColor(color: number, amount: number): number {
  const [r, g, b] = rgb(color);
  const y = luma(r, g, b);
  return pack(mix(r, y, amount), mix(g, y, amount), mix(b, y, amount));
}

/** 감쇠 예외 한 묶음: `keys`는 기본값 대신 이 `amount`로 깎는다. */
export interface KeepGroup<P> {
  keys: readonly (keyof P & string)[];
  amount: number;
}

/**
 * 팔레트 전체의 채도를 깎은 **새** 객체.
 *
 * `keep`은 예외 목록이다. 두 부류가 여기 들어간다.
 *   1) 빛나는 것(용암·불꽃) — 색이 곧 "뜨겁다"는 정보라 회색이 되면 갈색 얼룩이
 *      된다.
 *   2) 바깥 풍경(바다·개울·하늘·성벽의 깃발과 스테인드글라스) — 캐릭터가 밟고
 *      다니는 무대가 아니라 그 너머의 경치라, 진해도 사람과 경쟁하지 않는다.
 *
 * 다른 색을 다 죽인 뒤라 예외에 남긴 색은 오히려 더 또렷해진다 — 그래서 예외에도
 * 감쇠를 조금은 먹인다(0이 아니다).
 */
export function desaturatePalette<P extends Record<string, number>>(
  pal: P,
  amount: number,
  keep: readonly KeepGroup<P>[] = [],
): P {
  const kept = new Map<string, number>();
  for (const grp of keep) for (const k of grp.keys) kept.set(k, grp.amount);
  const out = {} as Record<string, number>;
  for (const key of Object.keys(pal)) out[key] = desaturateColor(pal[key], kept.get(key) ?? amount);
  return out as P;
}

/** 배경 잔무늬 하나의 죽이기 규칙: `keys`의 색을 `base` 쪽으로 `amount`만큼 당긴다. */
export interface QuietGroup<P> {
  /** 이 무늬가 깔리는 바탕 키(바닥·벽의 기본색). */
  base: keyof P & string;
  /** 바탕에 묻힐 디테일 키들. */
  keys: readonly (keyof P & string)[];
  /** 0=그대로, 1=바탕과 완전히 같은 색(무늬가 사라진다). */
  amount: number;
}

/**
 * 배경 잔무늬 죽이기 — 넓게 깔리는 면(바닥·벽)의 디테일 색을 그 면의 바탕색
 * 쪽으로 당긴다.
 *
 * 씬 팔레트는 배경 자체를 감상하도록 촘촘하게 칠해져 있는데, 실제로 이 위에는
 * 16px 캐릭터가 돌아다닌다. 바닥이 캐릭터만큼 많은 색을 쓰면 둘이 같은 층으로
 * 읽혀 사람이 무늬에 묻힌다 — 채도만 낮춰서는 해결되지 않는다(대비의 문제라
 * 색상의 문제가 아니다). 그래서 "그 씬을 그 씬이게 하는 색"(용암·홀로그램·
 * 바다·불꽃)은 손대지 않고, 이음선·얼룩·잔금처럼 정보가 없는 무늬만 바탕
 * 쪽으로 당겨 지운다.
 *
 * 무늬의 *밀도*(칸마다 찍히는 점 개수, 장식 등장 주기)는 각 씬의 drawTile이
 * 정한다 — 여기서는 *세기*만 다룬다.
 *
 * 테마 변환(adaptPalette)보다 **앞에** 온다: 이 단계는 씬 고유 팔레트 공간에서
 * 이뤄지는 저작 결정이고, 테마 변환은 그 결과를 앱 색감에 맞추는 후처리다.
 */
export function quietPalette<P extends Record<string, number>>(
  pal: P,
  groups: readonly QuietGroup<P>[],
): P {
  const out = { ...pal } as Record<string, number>;
  for (const { base, keys, amount } of groups) {
    // 바탕은 항상 *원본*에서 읽는다 — 그룹 순서가 결과를 바꾸지 않도록.
    const [br, bg, bb] = rgb(pal[base]);
    for (const key of keys) {
      const [r, g, b] = rgb(pal[key]);
      out[key] = pack(mix(r, br, amount), mix(g, bg, amount), mix(b, bb, amount));
    }
  }
  return out as P;
}

/** 팔레트(문자열 키 → 0xRRGGBB) 전체를 변환한 **새** 객체를 만든다. */
export function adaptPalette<P extends Record<string, number>>(pal: P, mode: SceneColorMode): P {
  if (mode === "identity") return { ...pal };
  const out = {} as Record<string, number>;
  for (const key of Object.keys(pal)) out[key] = adaptColor(pal[key], mode);
  return out as P;
}
