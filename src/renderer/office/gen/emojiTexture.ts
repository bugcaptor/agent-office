// src/renderer/office/gen/emojiTexture.ts
//
// 유니코드 이모지 글리프 → 픽셀아트 텍스처. 시스템 이모지 폰트로 큼직하게
// 그린 다음 **nearest로 축소**해 16px 타일 아트에 어울리는 저해상도 스프라이트를
// 굽는다(`gen/awardPortraitTexture.ts`와 같은 canvas → Texture.from 패턴).
//
// ## 왜 손그림 대신 이모지인가
//
// 트로피를 `Graphics`로 직접 찍었더니 눈검증에서 "좌우로 너무 넓다", "금색밖에
// 없어서 구분이 안 간다", 그 전엔 아예 "안 보인다"가 연달아 나왔다. 12×12px
// 안에서 컵·손잡이·기둥·받침을 다 읽히게 그리는 건 도트 예산이 너무 빡빡하다.
// 반면 🏆 글리프를 크게 그려 13px로 줄이면 손잡이·잘록한 목·짙은 받침이 전부
// 살아남는다 — 폰트 디자이너가 이미 그 실루엣을 최적화해 뒀기 때문이다.
//
// **작게 그리는 게 아니라 크게 그린 뒤 줄이는 것이 핵심이다.** 처음부터 13px로
// `fillText`하면 안티에일리어싱 때문에 뭉개져 픽셀아트가 깨진다(눈검증 확인).
//
// ## 한계 — 호출부가 반드시 폴백을 들고 있어야 한다
//
//  - **플랫폼 폰트 의존**: macOS는 Apple Color Emoji, Windows는 Segoe UI Emoji.
//    모양이 다르다. 없으면 이 함수가 null을 낸다.
//  - **테마 무관**: 이모지 본체는 항상 원색(금색 컵 + 붉은 받침)이라 팔레트
//    축을 안 탄다. 회색조 후 테마색 틴트도 해 봤지만 전 테마에서 탁해져 버렸다 —
//    트로피는 씬에서 유일하게 튀어야 하는 소품이라 원색을 그대로 쓴다. 대신
//    **외곽선만 테마 색**(`outline`)을 받아 배경과의 분리를 책임진다.
//  - **테스트 환경**: jsdom엔 2d 컨텍스트가 없다. null을 내고 호출부의
//    절차적 드로잉으로 떨어진다.
import { Texture } from "pixi.js";

/** 글리프를 그리는 고해상도 중간 캔버스 한 변(px). */
const HI_RES = 256;
/** 그 안에서 글리프가 차지하는 크기(px) — 여백을 두어 잘림을 막는다. */
const GLYPH_PX = 220;
/** 알파가 이보다 크면 글리프 픽셀로 본다(경계 상자 계산용). */
const ALPHA_FLOOR = 10;

/** 이모지 우선 폰트 스택. 이름을 못 찾으면 브라우저가 알아서 폴백한다. */
const EMOJI_FONT = '"Apple Color Emoji", "Segoe UI Emoji", "Noto Color Emoji", sans-serif';

/** 외곽선 두께(px). 1px이면 16px 타일 아트의 선 굵기와 맞는다. 외곽선을 쓰면
 * 텍스처가 이만큼 사방으로 커지므로(글리프는 안 줄인다) 호출부가 배치를
 * 보정할 수 있게 내보낸다. */
export const EMOJI_OUTLINE_PX = 1;

/** 같은 (문자, 크기, 외곽선)을 여러 씬·재구축·테마 왕복에서 다시 굽지 않는다. */
const cache = new Map<string, Texture | null>();

/** 글리프가 실제로 칠해진 영역. 글리프마다 여백이 달라 그대로 줄이면 크기가 들쭉날쭉하다. */
function glyphBounds(
  data: Uint8ClampedArray,
  side: number,
): { x: number; y: number; w: number; h: number } | null {
  let x0 = side;
  let y0 = side;
  let x1 = -1;
  let y1 = -1;
  for (let y = 0; y < side; y++) {
    for (let x = 0; x < side; x++) {
      if (data[(y * side + x) * 4 + 3] <= ALPHA_FLOOR) continue;
      if (x < x0) x0 = x;
      if (x > x1) x1 = x;
      if (y < y0) y0 = y;
      if (y > y1) y1 = y;
    }
  }
  if (x1 < 0) return null; // 아무것도 안 그려졌다 — 폰트 없음/미지원 글리프
  return { x: x0, y: y0, w: x1 - x0 + 1, h: y1 - y0 + 1 };
}

/**
 * 이모지 한 글자를 `size`×`size` nearest 텍스처로 굽는다. 실패(2d 컨텍스트
 * 없음·폰트 없음·빈 글리프)하면 null — 호출부는 절차적 폴백을 유지해야 한다.
 * 결과는 (문자, 크기, 외곽선색)으로 캐시된다.
 *
 * `outline`을 주면 그 색으로 1px 둘레를 두른다. 글리프는 안 줄이고 **텍스처가
 * 사방 `EMOJI_OUTLINE_PX`만큼 커진다** — 13px 글리프를 11px로 줄이면 눈에 띄게
 * 뭉개진다(눈검증). 호출부가 그만큼 배치를 밀어야 한다.
 *
 * 외곽선 색은 호출부 책임이다. 팔레트 색을 그대로 쓰면 대개 배경과 명도가
 * 비슷해 테두리가 아니라 얼룩으로 보인다 — 충분히 어두운 값을 넘겨야 한다.
 */
export function bakeEmojiTexture(
  char: string,
  size: number,
  outline?: number,
): Texture | null {
  const key = `${char}@${size}@${outline ?? "none"}`;
  const hit = cache.get(key);
  if (hit !== undefined) return hit;
  const baked = bake(char, size, outline);
  cache.set(key, baked);
  return baked;
}

/** 불투명 픽셀에 8방향으로 맞닿은 투명 픽셀을 `color`로 채운다(1px 외곽선). */
function strokeOutline(data: Uint8ClampedArray, side: number, color: number): void {
  const src = new Uint8ClampedArray(data); // 확장이 스스로를 먹지 않게 원본 사본을 본다
  const opaque = (x: number, y: number): boolean =>
    x >= 0 && y >= 0 && x < side && y < side && src[(y * side + x) * 4 + 3] > ALPHA_FLOOR;
  const r = (color >> 16) & 0xff;
  const g = (color >> 8) & 0xff;
  const b = color & 0xff;
  for (let y = 0; y < side; y++) {
    for (let x = 0; x < side; x++) {
      if (opaque(x, y)) continue;
      let touches = false;
      for (let dy = -1; dy <= 1 && !touches; dy++) {
        for (let dx = -1; dx <= 1; dx++) {
          if ((dx !== 0 || dy !== 0) && opaque(x + dx, y + dy)) {
            touches = true;
            break;
          }
        }
      }
      if (!touches) continue;
      const i = (y * side + x) * 4;
      data[i] = r;
      data[i + 1] = g;
      data[i + 2] = b;
      data[i + 3] = 255;
    }
  }
}

function bake(char: string, size: number, outline?: number): Texture | null {
  try {
    const hi = document.createElement("canvas");
    hi.width = HI_RES;
    hi.height = HI_RES;
    const hctx = hi.getContext("2d");
    if (!hctx) return null; // jsdom 등 2d 미지원
    hctx.textAlign = "center";
    hctx.textBaseline = "middle";
    hctx.font = `${GLYPH_PX}px ${EMOJI_FONT}`;
    hctx.fillText(char, HI_RES / 2, HI_RES / 2 + GLYPH_PX * 0.045); // 컬러 이모지는 baseline이 살짝 위로 뜬다
    const box = glyphBounds(hctx.getImageData(0, 0, HI_RES, HI_RES).data, HI_RES);
    if (!box) return null;

    // 외곽선은 글리프를 줄이는 대신 캔버스를 키워서 두른다.
    const pad = outline === undefined ? 0 : EMOJI_OUTLINE_PX;
    const side = size + pad * 2;
    const lo = document.createElement("canvas");
    lo.width = side;
    lo.height = side;
    const lctx = lo.getContext("2d");
    if (!lctx) return null;
    // 보간을 끄고 줄여야 도트가 선명하다. 켜면(면적 평균) 흐릿해져 주변
    // 픽셀아트와 따로 논다.
    lctx.imageSmoothingEnabled = false;
    lctx.drawImage(hi, box.x, box.y, box.w, box.h, pad, pad, size, size);

    if (outline !== undefined) {
      const im = lctx.getImageData(0, 0, side, side);
      strokeOutline(im.data, side, outline);
      lctx.putImageData(im, 0, 0);
    }

    const texture = Texture.from(lo);
    texture.source.scaleMode = "nearest";
    return texture;
  } catch (err) {
    console.warn(`emojiTexture: failed to bake ${char} — keeping procedural fallback`, err);
    return null;
  }
}
