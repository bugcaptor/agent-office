// src/renderer/office/gen/awardPortraitTexture.ts
//
// "이 달의 우수사원" 초상(base64, 헤더 없음) → Pixi 텍스처. 초상 → Pixi 텍스처
// 경로가 아직 없어(`portrait/portraitCache.ts`는 DOM `<img>` 전용) 여기 따로 둔다.
//
// ## 왜 초상만 프리필터하는가
//
// 월드 스프라이트는 원본이 이미 월드 픽셀 격자(16px)라 nearest가 정답이지만,
// 초상은 240×320 원본을 폴라로이드 사진칸(14×14 월드 px)에 넣는다 — 17배 축소다.
// nearest 점샘플로 그렇게 줄이면 원본 픽셀 대부분이 버려지고 살아남은 몇 개가
// 그대로 찍혀 화면이 자글거린다(aliasing).
//
// 처방은 커스텀 고해상 시트와 같다(이슈 #47, `spriteResample.ts`): 실제로 찍힐
// 물리 해상도 = 사진칸 × 카메라 정수 스케일 S 로 임포트 시점에 한 번 area(box)
// 다운스케일해 두고 1:1로 찍는다. 필터는 여전히 nearest지만 축소 자체가
// 사라지므로 지글거림이 원리적으로 없어지고, 박스 평균이라 바이리니어보다
// 또렷하다. S가 바뀌면(창 크기 변경) 원본 캔버스에서 다시 만든다 —
// 그래서 디코드 결과(원본 캔버스)와 텍스처 생성을 두 단계로 나눠 둔다.
import { Texture } from "pixi.js";
import { areaDownscalePremul } from "./spriteResample";

/** 비율 유지 "contain" 목표 크기. 확대는 하지 않는다(원본보다 커지면 원본 크기). */
export function containSize(
  srcW: number,
  srcH: number,
  maxW: number,
  maxH: number,
): { w: number; h: number } {
  if (srcW <= 0 || srcH <= 0) return { w: 1, h: 1 };
  const k = Math.min(maxW / srcW, maxH / srcH);
  if (k >= 1) return { w: srcW, h: srcH };
  return { w: Math.max(1, Math.round(srcW * k)), h: Math.max(1, Math.round(srcH * k)) };
}

/** base64 PNG(헤더 없음) → 원본 해상도 캔버스. 실패 시 null. */
export function loadAwardPortraitSource(b64: string): Promise<HTMLCanvasElement | null> {
  return new Promise((resolve) => {
    const img = new Image();
    img.onload = () => {
      const canvas = document.createElement("canvas");
      canvas.width = img.naturalWidth;
      canvas.height = img.naturalHeight;
      const ctx = canvas.getContext("2d")!;
      ctx.drawImage(img, 0, 0); // 1:1 복사 — 리샘플은 텍스처 단계에서 한다
      resolve(canvas);
    };
    img.onerror = () => {
      console.warn("awardPortraitTexture: 초상 디코드 실패");
      resolve(null);
    };
    img.src = `data:image/png;base64,${b64}`;
  });
}

/**
 * 원본 캔버스 → 사진칸(월드 px) × 렌더 스케일 S 에 맞춰 area 프리필터한 nearest
 * 텍스처. 항상 새 캔버스를 만들어 텍스처화한다 — 원본 캔버스를 그대로 넘기면
 * `Texture.from` 소스 캐시를 공유해 S 변경 시 옛 텍스처 파기가 새 텍스처를
 * 함께 무효화한다.
 */
export function awardPortraitTexture(
  src: HTMLCanvasElement,
  slotPx: { w: number; h: number },
  renderScale: number,
): Texture {
  const s = Math.max(1, Math.round(renderScale));
  const d = containSize(src.width, src.height, slotPx.w * s, slotPx.h * s);
  const out = document.createElement("canvas");
  out.width = d.w;
  out.height = d.h;
  const octx = out.getContext("2d")!;
  if (d.w === src.width && d.h === src.height) {
    octx.drawImage(src, 0, 0); // 축소가 필요 없으면 그대로 복사
  } else {
    const sctx = src.getContext("2d")!;
    const srcImg = sctx.getImageData(0, 0, src.width, src.height);
    const dst = areaDownscalePremul(
      { data: srcImg.data, width: src.width, height: src.height },
      d.w,
      d.h,
    );
    const imgData = octx.createImageData(d.w, d.h);
    imgData.data.set(dst.data);
    octx.putImageData(imgData, 0, 0);
  }
  const texture = Texture.from(out);
  texture.source.scaleMode = "nearest"; // 프리필터 후에는 정수배 확대만 남는다
  return texture;
}
