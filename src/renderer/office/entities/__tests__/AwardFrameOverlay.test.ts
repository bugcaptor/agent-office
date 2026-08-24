// src/renderer/office/entities/__tests__/AwardFrameOverlay.test.ts
//
// "이 달의 우수사원" 액자 오버레이 단위 테스트. 액자를 타일 드로잉(officeScene의
// Tile.Wall 케이스)에서 빼내면서 틀(테두리+매트)까지 이 클래스가 그리게 됐다 —
// 지오메트리(외곽 28×28 → 테두리 2px → 매트 2px → 콘텐츠 20×20)가 맞는지,
// 재도색이 세 겹(테두리·매트·실루엣)을 전부 건드리는지, 사진/실루엣 토글이
// 여전히 동작하는지를 못 박는다.
//
// `Graphics`는 렌더 컨텍스트 없이 구성 가능하다(TileRenderer.test.ts와 같은
// 판단) — 채워진 도형은 `context.instructions`에서 직접 읽는다. 텍스처는
// `entities/__tests__/helpers.ts`와 같은 방식으로 `BufferImageSource` 위의
// 진짜 `Texture`를 만들되, contain 스케일 검증을 위해 폭/높이를 원하는 값으로
// 만든다(helpers.ts의 solidTexture는 1×1 고정이라 재사용 불가).
import { describe, expect, it } from "vitest";
import { BufferImageSource, Texture, type Graphics, type Sprite } from "pixi.js";
import { AwardFrameOverlay, type AwardFramePalette } from "../AwardFrameOverlay";

const PAL: AwardFramePalette = { frameBorder: 0x111111, frameMat: 0x222222, frameSilhouette: 0x333333 };
const OUTER = { w: 28, h: 28 }; // scenes/officeScene.ts의 awardFrameRectPx()가 내는 외곽 크기

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

/** Graphics의 첫 fill 인스트럭션에서 사각형 지오메트리 + 채움색을 꺼낸다. */
function rectOf(g: Graphics, index = 0): { x: number; y: number; w: number; h: number; color: number } {
  const ins = g.context.instructions[index] as unknown as FillInstruction;
  const shape = ins.data.path.shapePath.shapePrimitives[0].shape;
  return { x: shape.x, y: shape.y, w: shape.width, h: shape.height, color: ins.data.style.color };
}

const border = (o: AwardFrameOverlay) => o.root.children[0] as Graphics;
const mat = (o: AwardFrameOverlay) => o.root.children[1] as Graphics;
const silhouette = (o: AwardFrameOverlay) => o.root.children[2] as Graphics;

describe("AwardFrameOverlay: 지오메트리(외곽 28×28 → 콘텐츠 20×20)", () => {
  it("테두리는 외곽 전체(28×28)를, 매트는 테두리 2px 안쪽(24×24)을 채운다", () => {
    const o = new AwardFrameOverlay(PAL, OUTER);
    expect(rectOf(border(o))).toEqual({ x: 0, y: 0, w: 28, h: 28, color: PAL.frameBorder });
    expect(rectOf(mat(o))).toEqual({ x: 2, y: 2, w: 24, h: 24, color: PAL.frameMat });
  });

  it("콘텐츠는 테두리+매트(각 2px)만큼 들어간 20×20 — showPhoto의 contain 스케일로 확인", () => {
    const o = new AwardFrameOverlay(PAL, OUTER);
    // 콘텐츠(20×20)보다 가로로 2배 긴 텍스처(40×20) → contain 스케일은 세로 기준(0.5).
    o.showPhoto(mkTexture(40, 20));
    const sprite = o.root.children[3] as Sprite;
    expect(sprite.width).toBe(20); // round(40 * 0.5)
    expect(sprite.height).toBe(10); // round(20 * 0.5)
    // 콘텐츠 원점(4,4) + 콘텐츠 중심(10,10)
    expect(sprite.position.x).toBe(14);
    expect(sprite.position.y).toBe(14);
  });
});

describe("AwardFrameOverlay: 사진/실루엣 토글", () => {
  it("생성 직후엔 틀+매트+실루엣 3겹만 있다(사진 없음)", () => {
    const o = new AwardFrameOverlay(PAL, OUTER);
    expect(o.root.children.length).toBe(3);
  });

  it("showPhoto는 4번째 자식으로 Sprite를 얹고, showSilhouette은 그걸 내린다", () => {
    const o = new AwardFrameOverlay(PAL, OUTER);
    o.showPhoto(mkTexture(10, 10));
    expect(o.root.children.length).toBe(4);
    o.showSilhouette();
    expect(o.root.children.length).toBe(3);
  });

  it("showSilhouette은 사진이 없을 때 아무것도 하지 않는다", () => {
    const o = new AwardFrameOverlay(PAL, OUTER);
    expect(() => o.showSilhouette()).not.toThrow();
    expect(o.root.children.length).toBe(3);
  });
});

describe("AwardFrameOverlay: 재도색·가시성·파기", () => {
  it("paint()는 테두리·매트·실루엣(머리+어깨) 색을 전부 다시 칠한다", () => {
    const o = new AwardFrameOverlay(PAL, OUTER);
    const next: AwardFramePalette = { frameBorder: 0xaaaaaa, frameMat: 0xbbbbbb, frameSilhouette: 0xcccccc };
    o.paint(next);

    expect(rectOf(border(o)).color).toBe(next.frameBorder);
    expect(rectOf(mat(o)).color).toBe(next.frameMat);
    const sil = silhouette(o);
    expect(sil.context.instructions).toHaveLength(2); // 머리(원) + 어깨(사다리꼴)
    expect(fillColorOf(sil, 0)).toBe(next.frameSilhouette);
    expect(fillColorOf(sil, 1)).toBe(next.frameSilhouette);
  });

  it("setVisible이 root.visible을 토글한다", () => {
    const o = new AwardFrameOverlay(PAL, OUTER);
    o.setVisible(true);
    expect(o.root.visible).toBe(true);
    o.setVisible(false);
    expect(o.root.visible).toBe(false);
  });

  it("destroy는 떠 있는 사진까지 예외 없이 파기한다", () => {
    const o = new AwardFrameOverlay(PAL, OUTER);
    o.showPhoto(mkTexture(10, 10));
    expect(() => o.destroy()).not.toThrow();
  });
});
