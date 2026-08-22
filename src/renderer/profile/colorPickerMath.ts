// src/renderer/profile/colorPickerMath.ts
//
// 컬러 피커의 순수 계산부(kbm #2fj). DOM을 모르므로 vitest로 직접 검증한다.
//
// 왜 HSV인가: 피커의 조작 모형이 "색상 슬라이더 + 채도/명도 사각형"이라
// 사각형의 x/y가 그대로 s/v가 되는 HSV가 좌표 ↔ 색 변환을 1:1로 만든다.
// 팔레트 쪽(`gen/palette.ts`)이 쓰는 HSL과는 쓰임이 다르므로 따로 둔다 —
// 저장·전달은 항상 "#rrggbb" 문자열이라 두 모형이 섞이지 않는다.

/** 색상(0..360) · 채도(0..1) · 명도(0..1). */
export interface Hsv {
  h: number;
  s: number;
  v: number;
}

const clamp01 = (v: number): number => Math.min(1, Math.max(0, v));
const wrapHue = (h: number): number => ((h % 360) + 360) % 360;

/** 0..255 정수 3개 -> "#rrggbb". */
function toHex(r: number, g: number, b: number): string {
  const h = (n: number) => Math.round(Math.min(255, Math.max(0, n))).toString(16).padStart(2, "0");
  return `#${h(r)}${h(g)}${h(b)}`;
}

/** HSV -> "#rrggbb". */
export function hsvToHex({ h, s, v }: Hsv): string {
  const hh = wrapHue(h);
  const ss = clamp01(s);
  const vv = clamp01(v);
  const c = vv * ss;
  const x = c * (1 - Math.abs(((hh / 60) % 2) - 1));
  const m = vv - c;
  let r = 0, g = 0, b = 0;
  if (hh < 60) [r, g, b] = [c, x, 0];
  else if (hh < 120) [r, g, b] = [x, c, 0];
  else if (hh < 180) [r, g, b] = [0, c, x];
  else if (hh < 240) [r, g, b] = [0, x, c];
  else if (hh < 300) [r, g, b] = [x, 0, c];
  else [r, g, b] = [c, 0, x];
  return toHex((r + m) * 255, (g + m) * 255, (b + m) * 255);
}

/**
 * 입력 문자열 -> "#rrggbb"(소문자). "#" 생략, 3자리 축약, 대문자를 모두 받는다.
 * 형식이 아니면 null — 호출부가 "아직 입력 중"과 "확정 불가"를 구분할 수 있게
 * 예외 대신 null을 돌려준다.
 */
export function normalizeHex(input: string | undefined): string | null {
  const t = (input ?? "").trim().replace(/^#/, "");
  if (/^[0-9a-fA-F]{3}$/.test(t)) {
    const [r, g, b] = [t[0], t[1], t[2]];
    return `#${r}${r}${g}${g}${b}${b}`.toLowerCase();
  }
  if (/^[0-9a-fA-F]{6}$/.test(t)) return `#${t.toLowerCase()}`;
  return null;
}

/**
 * "#rrggbb" -> HSV. 형식이 아니면 검정(h=0,s=0,v=0). 무채색이면 색상을 0으로
 * 두는 대신 **호출부가 준 이전 색상을 유지**할 수 있도록 `keepHue`를 받는다 —
 * 채도를 0으로 끌어내렸을 때 색상 슬라이더가 빨강으로 튀는 것을 막는다.
 */
export function hexToHsv(input: string | undefined, keepHue = 0): Hsv {
  const hex = normalizeHex(input);
  if (!hex) return { h: wrapHue(keepHue), s: 0, v: 0 };
  const n = Number.parseInt(hex.slice(1), 16);
  const r = ((n >> 16) & 255) / 255;
  const g = ((n >> 8) & 255) / 255;
  const b = (n & 255) / 255;
  const max = Math.max(r, g, b);
  const min = Math.min(r, g, b);
  const d = max - min;
  const v = max;
  const s = max === 0 ? 0 : d / max;
  if (d === 0) return { h: wrapHue(keepHue), s, v };
  let h: number;
  if (max === r) h = 60 * (((g - b) / d) % 6);
  else if (max === g) h = 60 * ((b - r) / d + 2);
  else h = 60 * ((r - g) / d + 4);
  return { h: wrapHue(h), s, v };
}

/** 두 색 문자열이 같은 색인가(표기 차이 무시). 둘 다 형식이 아니면 false. */
export function sameHex(a: string | undefined, b: string | undefined): boolean {
  const na = normalizeHex(a);
  const nb = normalizeHex(b);
  return na !== null && na === nb;
}

/**
 * 프리셋 스와치 격자 — "총천연색"을 한눈에 훑고 바로 집을 수 있게. 색상 12칸 ×
 * 명도/채도 5단으로 만든 무지개 판에 마지막 줄로 무채색 12단을 덧붙인다.
 * 순수 계산이라 상수 테이블을 손으로 적을 필요가 없다.
 */
export const PRESET_HUE_STEPS = 12;

export const PRESET_SWATCHES: readonly (readonly string[])[] = [
  ...[
    { s: 0.35, v: 1.0 },
    { s: 0.65, v: 1.0 },
    { s: 1.0, v: 0.95 },
    { s: 1.0, v: 0.7 },
    { s: 1.0, v: 0.45 },
  ].map((row) =>
    Array.from({ length: PRESET_HUE_STEPS }, (_, i) =>
      hsvToHex({ h: (360 / PRESET_HUE_STEPS) * i, s: row.s, v: row.v }),
    ),
  ),
  Array.from({ length: PRESET_HUE_STEPS }, (_, i) =>
    hsvToHex({ h: 0, s: 0, v: i / (PRESET_HUE_STEPS - 1) }),
  ),
];

/**
 * 사각형/슬라이더 안에서의 포인터 위치 -> 0..1 비율. 트랙 밖으로 끌어도
 * 끝에서 멈추도록 클램프한다(드래그 중 커서가 창을 벗어나는 흔한 경우).
 */
export function ratioAt(pos: number, start: number, size: number): number {
  if (size <= 0) return 0;
  return clamp01((pos - start) / size);
}
