// src/renderer/office/entities/__tests__/AwardFrameOverlay.test.ts
//
// "이 달의 우수사원" 벽 사진 오버레이 단위 테스트. 정사각 액자가 "영정사진
// 같다"는 눈검증을 받아 **압정으로 꽂은 작은 폴라로이드**로 바뀌었으므로, 그
// 형태 근거를 못 박는다: (1) 아래 턱이 좌·우·상 여백보다 넓다(폴라로이드의
// 정체), (2) 아래 턱에 축하 표지(별)가 있다, (3) 압정은 카드와 달리 기울지
// 않는다, (4) 카드는 압정을 축으로 기운다. 여기에 재도색·사진/실루엣 토글.
//
// `Graphics`는 렌더 컨텍스트 없이 구성 가능하다(TileRenderer.test.ts와 같은
// 판단) — 채워진 도형은 `context.instructions`에서 직접 읽는다. 텍스처는
// `entities/__tests__/helpers.ts`와 같은 방식으로 `BufferImageSource` 위의
// 진짜 `Texture`를 만들되, contain 스케일 검증을 위해 폭/높이를 원하는 값으로
// 만든다(helpers.ts의 solidTexture는 1×1 고정이라 재사용 불가).
import { describe, expect, it } from "vitest";
import { BufferImageSource, Texture, type Container, type Graphics, type Sprite } from "pixi.js";
import { AwardFrameOverlay, type AwardFramePalette } from "../AwardFrameOverlay";

const PAL: AwardFramePalette = { frameBorder: 0x111111, frameMat: 0x222222, frameSilhouette: 0x333333 };
const CARD = { w: 18, h: 21 }; // scenes/officeScene.ts의 awardFrameRectPx()가 내는 카드 크기

const mkTexture = (w: number, h: number): Texture =>
  new Texture({
    source: new BufferImageSource({ resource: new Uint8Array(w * h * 4), width: w, height: h, label: "test" }),
    label: "test",
  });

// pixi의 GraphicsInstruction 유니언 타입은 fill 전용 필드(path.shapePath, style.color)를
// 정적으로 좁혀주지 않는다 — 테스트에서만 실제 shape을 읽기 위한 최소 캐스팅.
type FillInstruction = {
  data: {
    style: { color: number };
    path: { shapePath: { shapePrimitives: { shape: { x: number; y: number; width: number; height: number } }[] } };
  };
};

function fillColorOf(g: Graphics, index = 0): number {
  return (g.context.instructions[index] as unknown as FillInstruction).data.style.color;
}

/** Graphics의 `index`번째 fill 인스트럭션에서 사각형 지오메트리 + 채움색을 꺼낸다. */
function rectOf(g: Graphics, index = 0): { x: number; y: number; w: number; h: number; color: number } {
  const ins = g.context.instructions[index] as unknown as FillInstruction;
  const shape = ins.data.path.shapePath.shapePrimitives[0].shape;
  return { x: shape.x, y: shape.y, w: shape.width, h: shape.height, color: ins.data.style.color };
}

const card = (o: AwardFrameOverlay) => o.root.children[0] as Container;
const pin = (o: AwardFrameOverlay) => o.root.children[1] as Graphics;
const paper = (o: AwardFrameOverlay) => card(o).children[0] as Graphics;
const silhouette = (o: AwardFrameOverlay) => card(o).children[1] as Graphics;

// paper의 fill 순서: 0 그림자, 1 카드 종이, 2 사진칸 바탕, 3~4 아래 턱의 별(십자).
const SHADOW = 0;
const SHEET = 1;
const PHOTO_BOX = 2;
const STAR_H = 3;
const STAR_V = 4;

describe("AwardFrameOverlay: 폴라로이드 형태(18×21, 사진칸 14×14 + 넓은 아래 턱)", () => {
  it("카드 종이는 전체를, 사진칸은 좌·우·상 2px 안쪽 14×14를 채운다", () => {
    const o = new AwardFrameOverlay(PAL, CARD);
    expect(rectOf(paper(o), SHEET)).toEqual({ x: 0, y: 0, w: 18, h: 21, color: PAL.frameMat });
    expect(rectOf(paper(o), PHOTO_BOX)).toEqual({ x: 2, y: 2, w: 14, h: 14, color: PAL.frameSilhouette });
  });

  it("아래 턱이 위 여백보다 넓다 — 이 비대칭이 액자와 폴라로이드를 갈라놓는다", () => {
    const o = new AwardFrameOverlay(PAL, CARD);
    const box = rectOf(paper(o), PHOTO_BOX);
    const top = box.y;
    const bottom = CARD.h - (box.y + box.h);
    expect(bottom).toBeGreaterThan(top);
    expect(bottom).toBe(5);
  });

  it("아래 턱 중앙에 축하 표지(3px 십자 별)를 찍는다", () => {
    const o = new AwardFrameOverlay(PAL, CARD);
    const h = rectOf(paper(o), STAR_H);
    const v = rectOf(paper(o), STAR_V);
    expect(h).toEqual({ x: 8, y: 19, w: 3, h: 1, color: PAL.frameSilhouette });
    expect(v).toEqual({ x: 9, y: 18, w: 1, h: 3, color: PAL.frameSilhouette });
    // 별은 사진칸 아래(=아래 턱 안)에 있다.
    expect(h.y).toBeGreaterThan(rectOf(paper(o), PHOTO_BOX).y + rectOf(paper(o), PHOTO_BOX).h);
  });

  it("카드 뒤에 1px 그림자를 깐다(벽에서 떠 있는 느낌)", () => {
    const o = new AwardFrameOverlay(PAL, CARD);
    expect(rectOf(paper(o), SHADOW)).toEqual({ x: 1, y: 1, w: 18, h: 21, color: PAL.frameBorder });
  });
});

describe("AwardFrameOverlay: 압정과 기울기", () => {
  it("카드는 압정(상단 중앙)을 축으로 살짝 기울고, 압정 자신은 기울지 않는다", () => {
    const o = new AwardFrameOverlay(PAL, CARD);
    const c = card(o);
    expect(c.rotation).toBeLessThan(0); // 반시계로 살짝
    expect(Math.abs(c.rotation)).toBeLessThan(0.1); // 픽셀아트 아티팩트가 안 보일 만큼만
    // 회전축 = 압정 위치(상단 중앙). pivot과 position이 같아 그 점이 고정된다.
    expect(c.pivot.x).toBe(9);
    expect(c.pivot.y).toBe(0);
    expect(c.position.x).toBe(9);
    expect(c.position.y).toBe(0);
    expect(pin(o).rotation).toBe(0);
  });

  it("압정 머리는 꽉 찬 사각형이 아니라 십자 3×3이다(둥근 머리로 읽히게)", () => {
    const o = new AwardFrameOverlay(PAL, CARD);
    expect(rectOf(pin(o), 0)).toEqual({ x: 9, y: -1, w: 1, h: 3, color: PAL.frameBorder }); // 세로
    expect(rectOf(pin(o), 1)).toEqual({ x: 8, y: 0, w: 3, h: 1, color: PAL.frameBorder }); // 가로
    expect(rectOf(pin(o), 2)).toEqual({ x: 9, y: -1, w: 1, h: 1, color: PAL.frameMat }); // 꼭대기 하이라이트
  });
});

describe("AwardFrameOverlay: 사진/실루엣 토글", () => {
  it("생성 직후 root는 카드+압정 두 겹, 카드는 종이+실루엣 두 겹이다(사진 없음)", () => {
    const o = new AwardFrameOverlay(PAL, CARD);
    expect(o.root.children.length).toBe(2);
    expect(card(o).children.length).toBe(2);
  });

  it("사진은 사진칸에 contain 스케일로 들어가고 카드에 붙는다(카드와 같이 기운다)", () => {
    const o = new AwardFrameOverlay(PAL, CARD);
    // 사진칸(14×14)보다 가로로 2배 긴 텍스처(28×14) → contain 스케일은 세로 기준(0.5).
    o.showPhoto(mkTexture(28, 14));
    const sprite = card(o).children[2] as Sprite;
    expect(sprite.width).toBe(14); // round(28 * 0.5)
    expect(sprite.height).toBe(7); // round(14 * 0.5)
    // 사진칸 원점(2,2) + 사진칸 중심(7,7)
    expect(sprite.position.x).toBe(9);
    expect(sprite.position.y).toBe(9);
    expect(o.root.children.length).toBe(2); // 압정 옆이 아니라 카드 안이다
  });

  it("showSilhouette은 사진을 내린다", () => {
    const o = new AwardFrameOverlay(PAL, CARD);
    o.showPhoto(mkTexture(10, 10));
    expect(card(o).children.length).toBe(3);
    o.showSilhouette();
    expect(card(o).children.length).toBe(2);
  });

  it("showSilhouette은 사진이 없을 때 아무것도 하지 않는다", () => {
    const o = new AwardFrameOverlay(PAL, CARD);
    expect(() => o.showSilhouette()).not.toThrow();
    expect(card(o).children.length).toBe(2);
  });
});

describe("AwardFrameOverlay: 재도색·가시성·파기", () => {
  it("paint()는 카드·별·압정·실루엣 색을 전부 다시 칠한다", () => {
    const o = new AwardFrameOverlay(PAL, CARD);
    const next: AwardFramePalette = { frameBorder: 0xaaaaaa, frameMat: 0xbbbbbb, frameSilhouette: 0xcccccc };
    o.paint(next);

    expect(rectOf(paper(o), SHEET).color).toBe(next.frameMat);
    expect(rectOf(paper(o), PHOTO_BOX).color).toBe(next.frameSilhouette);
    expect(rectOf(paper(o), STAR_H).color).toBe(next.frameSilhouette);
    expect(rectOf(pin(o), 0).color).toBe(next.frameBorder);

    const sil = silhouette(o);
    expect(sil.context.instructions).toHaveLength(2); // 머리(원) + 어깨(사다리꼴)
    // 사진칸 바탕과 대비되게 짙은 색으로 찍는다.
    expect(fillColorOf(sil, 0)).toBe(next.frameBorder);
    expect(fillColorOf(sil, 1)).toBe(next.frameBorder);
  });

  it("setVisible이 root.visible을 토글한다(카드·압정 통째로)", () => {
    const o = new AwardFrameOverlay(PAL, CARD);
    o.setVisible(true);
    expect(o.root.visible).toBe(true);
    o.setVisible(false);
    expect(o.root.visible).toBe(false);
  });

  it("destroy는 떠 있는 사진까지 예외 없이 파기한다", () => {
    const o = new AwardFrameOverlay(PAL, CARD);
    o.showPhoto(mkTexture(10, 10));
    expect(() => o.destroy()).not.toThrow();
  });
});
