// @vitest-environment jsdom
//
// src/renderer/office/entities/__tests__/TrophyOverlay.test.ts
//
// 트로피는 **🏆 글리프를 구운 스프라이트가 1순위, 절차적 도트가 폴백**이다.
// 구운 픽셀 자체는 시스템 이모지 폰트에 달려 있어 검증 대상이 아니고, 여기서
// 못 박는 것은 두 경로의 계약이다.
//  - 굽기 실패(폰트 없음/2d 없음) → 폴백 Graphics가 **보인다**. 이게 깨지면
//    트로피가 통째로 사라진다(과거 회귀: "트로피 안 보이는데?").
//  - 굽기 성공 → 스프라이트가 폴백을 가리고, 글리프 바닥이 폴백 받침 바닥(y=+5)에
//    맞는다. 텍스처가 외곽선만큼 크므로 그 몫을 보정하지 않으면 1px 뜬다.
//  - 테마 전환(`paint`) → 외곽선 색이 테마 축이라 텍스처가 교체된다.
import { afterEach, describe, expect, it, vi } from "vitest";
import type { Graphics, Sprite } from "pixi.js";
import { TrophyOverlay, type TrophyPalette } from "../TrophyOverlay";
import { EMOJI_OUTLINE_PX } from "../../gen/emojiTexture";

const PAL: TrophyPalette = {
  trophyCup: 0xe0a53d,
  trophyCupShine: 0xfff0c2,
  trophyBase: 0x8a6a2f,
  trophyRibbon: 0xc0453a,
};

/** 굽기가 성공하도록 흉내 낸 2d 컨텍스트 — 글리프 픽셀이 있는 것처럼 군다. */
function stubWorkingContext(): void {
  const opaque = new Uint8ClampedArray(256 * 256 * 4);
  for (let i = 3; i < opaque.length; i += 4) opaque[i] = 255;
  vi.spyOn(HTMLCanvasElement.prototype, "getContext").mockReturnValue({
    textAlign: "",
    textBaseline: "",
    font: "",
    imageSmoothingEnabled: true,
    fillText: () => {},
    drawImage: () => {},
    putImageData: () => {},
    getImageData: (_x: number, _y: number, w: number, h: number) => ({
      data: new Uint8ClampedArray(w * h * 4).fill(255),
    }),
  } as unknown as ReturnType<HTMLCanvasElement["getContext"]>);
}

afterEach(() => {
  vi.restoreAllMocks();
});

describe("TrophyOverlay", () => {
  it("굽기가 실패하면 절차적 폴백이 보인다 — 트로피가 사라지면 안 된다", () => {
    vi.spyOn(HTMLCanvasElement.prototype, "getContext").mockReturnValue(null);
    const o = new TrophyOverlay(PAL);
    expect(o.usesEmoji).toBe(false);
    expect((o.root.children[0] as Graphics).visible).toBe(true);
  });

  it("굽기가 성공하면 스프라이트가 폴백을 가리고 바닥이 y=+5에 맞는다", () => {
    stubWorkingContext();
    const o = new TrophyOverlay({ ...PAL, trophyBase: 0x111111 }); // 캐시 키 분리
    expect(o.usesEmoji).toBe(true);
    expect((o.root.children[0] as Graphics).visible).toBe(false);
    const sprite = o.root.children[1] as Sprite;
    expect(sprite.anchor.y).toBe(1);
    // 텍스처가 외곽선만큼 사방으로 크다 — 그 몫을 더해야 글리프 바닥이 +5다.
    expect(sprite.position.y).toBe(5 + EMOJI_OUTLINE_PX);
    expect(sprite.position.x).toBe(0);
  });

  it("테마를 바꾸면 텍스처를 교체한다(외곽선 색이 테마 축이다)", () => {
    stubWorkingContext();
    const o = new TrophyOverlay({ ...PAL, trophyBase: 0x222222 });
    const before = (o.root.children[1] as Sprite).texture;
    o.paint({ ...PAL, trophyBase: 0x333333 });
    const after = (o.root.children[1] as Sprite).texture;
    expect(after).not.toBe(before);
    expect(o.root.children.length).toBe(2); // 스프라이트를 새로 얹지 않고 갈아끼운다
  });

  it("setVisible이 root.visible을 토글한다", () => {
    vi.spyOn(HTMLCanvasElement.prototype, "getContext").mockReturnValue(null);
    const o = new TrophyOverlay(PAL);
    o.setVisible(true);
    expect(o.root.visible).toBe(true);
    o.setVisible(false);
    expect(o.root.visible).toBe(false);
  });
});
