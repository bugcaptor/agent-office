// src/renderer/office/gen/awardPortraitTexture.ts
//
// "이 달의 우수사원" 초상(base64, 헤더 없음) → Pixi 텍스처. 초상 → Pixi 텍스처
// 경로가 아직 없어(`portrait/portraitCache.ts`는 DOM `<img>` 전용) 새로 만든다 —
// `sprite/spriteCache.ts`의 `decodeSheet` 패턴(Image → canvas → Texture.from)을
// 그대로 따르되, 시트가 아니라 정사각 단일 이미지라 셀 분할이 없다.
import { Texture } from "pixi.js";

function decodePortrait(b64: string): Promise<HTMLCanvasElement> {
  return new Promise((resolve, reject) => {
    const img = new Image();
    img.onload = () => {
      const canvas = document.createElement("canvas");
      canvas.width = img.naturalWidth;
      canvas.height = img.naturalHeight;
      const ctx = canvas.getContext("2d")!;
      ctx.imageSmoothingEnabled = false;
      ctx.drawImage(img, 0, 0);
      resolve(canvas);
    };
    img.onerror = () => reject(new Error("award portrait decode failed"));
    img.src = `data:image/png;base64,${b64}`;
  });
}

/**
 * base64 PNG(헤더 없음) → nearest-필터 Pixi 텍스처. `antialias:false`/
 * `roundPixels:true` 환경이므로 스케일 모드를 명시적으로 nearest로 고정한다.
 * 디코드 실패 시 null(호출자가 실루엣 폴백을 유지).
 */
export async function loadAwardPortraitTexture(b64: string): Promise<Texture | null> {
  try {
    const canvas = await decodePortrait(b64);
    const texture = Texture.from(canvas);
    texture.source.scaleMode = "nearest";
    return texture;
  } catch (err) {
    console.warn("awardPortraitTexture: 초상 디코드 실패", err);
    return null;
  }
}
