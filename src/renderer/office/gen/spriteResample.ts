// src/renderer/office/gen/spriteResample.ts
//
// 커스텀 고해상 스프라이트의 S-적응 프리필터(이슈 #47).
//
// 문제: 커스텀 시트 셀 N(≤256)을 오피스뷰에서 겉보기 16px로 nearest 축소하면,
// 카메라 정수 스케일 S와 겹쳐 최종 물리 크기는 16·S px밖에 안 되는데 매 프레임
// nearest 점샘플로 minification하므로 캐릭터가 서브픽셀로 움직일 때 어느 텍셀이
// 뽑히는지가 바뀌어 지글거린다(aliasing).
//
// 해법: 카메라가 이미 계산하는 정수 S에 맞춰 프레임을 D = min(N, 16·S) 픽셀로
// 임포트 시점에 한 번 area(box) 다운스케일한다. 그러면 sprite.scale = 16/D과
// 월드 스케일 S가 곱해져 텍셀:물리픽셀 = 1:1이 되어(roundPixels와 함께 픽셀
// 그리드에 스냅) minification 자체가 사라진다 → 지글거림 원리적 제거. 정확한
// 크기로 직접 area 리샘플하므로 트라이리니어/바이리니어보다 또렷하다.
//
// 순수(ImageData 배열 위 연산) — DOM/Pixi 비의존이라 vitest로 픽셀 검증 가능.
import { CELL } from "./compositor";

/** 렌더 스케일 S에서 셀 N의 목표 해상도 D = min(N, 16·S). S는 정수 반올림(최소 1). */
export function detailCellSize(n: number, renderScale: number): number {
  const s = Math.max(1, Math.round(renderScale));
  return Math.min(n, CELL * s);
}

export interface Rgba {
  data: Uint8ClampedArray;
  width: number;
  height: number;
}

/**
 * area(box) 다운스케일. dest 픽셀마다 대응하는 source 박스를 부분 겹침 가중치로
 * 적분한다. 알파는 premultiply해서 평균(α=0 텍셀의 RGB가 색 평균에 섞여 반투명
 * 가장자리에 프린지가 생기는 것을 막는다) 후 다시 나눠 되돌린다. 확대(dw≥sw)는
 * 상위에서 걸러 호출하지 않는 전제(그 경우 nearest 확대 경로를 쓴다). 순수.
 */
export function areaDownscalePremul(src: Rgba, dw: number, dh: number): Rgba {
  const { data: s, width: sw, height: sh } = src;
  const out = new Uint8ClampedArray(dw * dh * 4);
  const scaleX = sw / dw;
  const scaleY = sh / dh;
  for (let dy = 0; dy < dh; dy++) {
    const sy0 = dy * scaleY;
    const sy1 = sy0 + scaleY;
    const iy0 = Math.floor(sy0);
    const iy1 = Math.ceil(sy1);
    for (let dx = 0; dx < dw; dx++) {
      const sx0 = dx * scaleX;
      const sx1 = sx0 + scaleX;
      const ix0 = Math.floor(sx0);
      const ix1 = Math.ceil(sx1);
      let rAcc = 0;
      let gAcc = 0;
      let bAcc = 0;
      let aAcc = 0; // Σ(α·w) — premult 색 평균 분모
      let wAcc = 0; // Σ(w)   — 커버리지, 알파 평균 분모
      for (let sy = iy0; sy < iy1; sy++) {
        const wy = Math.min(sy1, sy + 1) - Math.max(sy0, sy);
        if (wy <= 0) continue;
        for (let sx = ix0; sx < ix1; sx++) {
          const wx = Math.min(sx1, sx + 1) - Math.max(sx0, sx);
          if (wx <= 0) continue;
          const w = wx * wy;
          const i = (sy * sw + sx) * 4;
          const aw = (s[i + 3] / 255) * w;
          rAcc += s[i] * aw;
          gAcc += s[i + 1] * aw;
          bAcc += s[i + 2] * aw;
          aAcc += aw;
          wAcc += w;
        }
      }
      const o = (dy * dw + dx) * 4;
      if (aAcc > 0) {
        out[o] = rAcc / aAcc;
        out[o + 1] = gAcc / aAcc;
        out[o + 2] = bAcc / aAcc;
      }
      out[o + 3] = wAcc > 0 ? (aAcc / wAcc) * 255 : 0;
    }
  }
  return { data: out, width: dw, height: dh };
}
