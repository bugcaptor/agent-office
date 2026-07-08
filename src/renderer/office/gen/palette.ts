// src/renderer/office/gen/palette.ts
import type { Rng } from './prng';

export interface Ramp { shadow: number; base: number; light: number; } // 0xRRGGBB
export interface CharacterPalette {
  skin: Ramp;
  hair: Ramp;
  shirt: Ramp;
  pants: Ramp;
  outline: number; // 공통 외곽선
}

function clamp01(v: number): number { return Math.min(1, Math.max(0, v)); }

export function hslToRgb(h: number, s: number, l: number): number {
  h = ((h % 360) + 360) % 360; s = clamp01(s); l = clamp01(l);
  const c = (1 - Math.abs(2 * l - 1)) * s;
  const x = c * (1 - Math.abs(((h / 60) % 2) - 1));
  const m = l - c / 2;
  let r = 0, g = 0, b = 0;
  if (h < 60) [r, g, b] = [c, x, 0];
  else if (h < 120) [r, g, b] = [x, c, 0];
  else if (h < 180) [r, g, b] = [0, c, x];
  else if (h < 240) [r, g, b] = [0, x, c];
  else if (h < 300) [r, g, b] = [x, 0, c];
  else [r, g, b] = [c, 0, x];
  return (Math.round((r + m) * 255) << 16) | (Math.round((g + m) * 255) << 8) | Math.round((b + m) * 255);
}

/** 상대 휘도 (WCAG 근사) — 대비 테스트에 사용. */
export function luminance(rgb: number): number {
  const f = (v: number) => { v /= 255; return v <= 0.03928 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4; };
  return 0.2126 * f((rgb >> 16) & 255) + 0.7152 * f((rgb >> 8) & 255) + 0.0722 * f(rgb & 255);
}
export function contrastRatio(a: number, b: number): number {
  const la = luminance(a), lb = luminance(b);
  return (Math.max(la, lb) + 0.05) / (Math.min(la, lb) + 0.05);
}

export function ramp(h: number, s: number, l: number, spread = 0.16): Ramp {
  return {
    shadow: hslToRgb(h, s, Math.max(0.06, l - spread)),
    base: hslToRgb(h, s, l),
    light: hslToRgb(h, s, Math.min(0.94, l + spread)),
  };
}

const SKIN_TONES: ReadonlyArray<[number, number, number]> = [
  [28, 0.45, 0.78], [26, 0.5, 0.66], [24, 0.5, 0.52], [20, 0.5, 0.38], [18, 0.45, 0.28],
];

/** 셔츠/피부 기본색 사이에 보장하는 최소 명도 대비. */
export const SHIRT_SKIN_MIN_CONTRAST = 1.6;

/**
 * 최종 클램프 — 원래 설계의 클램프 코드에서 의도적으로 이탈(설계 오너 승인,
 * 문서는 추후 동기화). 원문(`ramp(rng.range(0,360), 0.6, sl > 0.5 ? 0.28 : 0.7)`)은
 * hue만 랜덤이고 s/l이 고정인데, HSL의 l이 같아도 상대 휘도는 hue에 따라 달라져
 * (WCAG 가중치 R 0.2126 / G 0.7152 / B 0.0722) 대비 1.6이 보장되지 않았다
 * (검증된 반례: seed 2274 → 대비 ≈1.52).
 *
 * 수정: hue는 유지한 채, 실제 `contrastRatio`를 재계산하며 l을 피부의 반대
 * 극단(피부가 밝으면 0, 어두우면 1) 방향으로 작은 스텝으로 스캔해 대비 >= 1.6이
 * 될 때까지 조정한다. 흑/백 극단에서 항상 대비가 충족되므로 종료가 보장되며,
 * 스캔 내부에서 rng를 쓰지 않아 결정적이다. 셔츠 램프(shadow/light)는 기존처럼
 * 최종 l에서 파생된다.
 */
export function clampShirtRamp(hue: number, skinBase: number, skinIsLight: boolean): Ramp {
  const sat = 0.6;
  const step = skinIsLight ? -0.02 : 0.02;
  let l = skinIsLight ? 0.28 : 0.7;
  while (
    contrastRatio(hslToRgb(hue, sat, l), skinBase) < SHIRT_SKIN_MIN_CONTRAST &&
    l > 0 && l < 1
  ) {
    l = Math.min(1, Math.max(0, l + step));
  }
  return ramp(hue, sat, l);
}

export function generatePalette(rng: Rng): CharacterPalette {
  const [sh, ss, sl] = rng.pick(SKIN_TONES);
  const skin = ramp(sh, ss, sl, 0.1);

  const hairHue = rng.pick([20, 30, 40, 0, 200, 280, 45]); // 갈/금/흑(저채도)/빨강/파랑/보라
  const hairL = rng.range(0.18, 0.6);
  const hair = ramp(hairHue, rng.range(0.25, 0.7), hairL, 0.14);

  // 의상: 피부 대비를 만족할 때까지 재시도 (최대 8회), 실패 시 명도 강제 클램프
  let shirt = ramp(rng.range(0, 360), rng.range(0.4, 0.85), rng.range(0.35, 0.6));
  for (let i = 0; i < 8 && contrastRatio(shirt.base, skin.base) < 1.6; i++) {
    shirt = ramp(rng.range(0, 360), rng.range(0.4, 0.85), rng.range(0.3, 0.62));
  }
  if (contrastRatio(shirt.base, skin.base) < 1.6) {
    // 최종 클램프: hue 인지형 l 스캔으로 대비 >= 1.6을 실제로 보장 (clampShirtRamp 참조).
    // rng 소비는 원래 코드와 동일하게 hue 1회 — 시드 결정성 유지.
    shirt = clampShirtRamp(rng.range(0, 360), skin.base, sl > 0.5);
  }
  const pants = ramp(rng.range(0, 360), rng.range(0.2, 0.6), rng.range(0.22, 0.42));

  return { skin, hair, shirt, pants, outline: 0x1a1420 };
}
