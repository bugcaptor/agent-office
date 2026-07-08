// src/renderer/portrait/retroFilter.ts
//
// 레트로(90년대 저해상) 필터의 순수 수학. 실제 다운/업스케일은 PortraitEditor가
// canvas(imageSmoothingEnabled=false)로 nearest-neighbor 처리하고, 여기서는
// 크기 상수와 채널 포스터라이즈(단계 축소)만 담당한다.

/** 1/4 해상도 다운스케일 격자(3:4). */
export const RETRO_DOWNSCALE = { w: 60, h: 80 } as const;

/** 채널당 색 단계 수(작을수록 더 거친 팔레트 느낌). */
export const RETRO_LEVELS = 6;

/** 0..255 값을 levels 단계로 균등 포스터라이즈. 결정적, 순수. */
export function posterize(value: number, levels: number): number {
  if (levels <= 1) return 0;
  const step = 255 / (levels - 1);
  return Math.round(Math.round(value / step) * step);
}

/** RGBA 배열의 R/G/B만 포스터라이즈(알파 유지). 새 number[] 반환. 순수. */
export function posterizeRgba(
  data: Uint8ClampedArray | number[],
  levels: number
): number[] {
  const out = new Array<number>(data.length);
  for (let i = 0; i < data.length; i += 4) {
    out[i] = posterize(data[i], levels);
    out[i + 1] = posterize(data[i + 1], levels);
    out[i + 2] = posterize(data[i + 2], levels);
    out[i + 3] = data[i + 3];
  }
  return out;
}
