// src/renderer/office/gen/__tests__/characterFactory.custom.test.ts
//
// 커스텀 시트 오버라이드 분기 TDD. Texture.from은 실제 캔버스/WebGL이 필요해
// node 환경에서 못 쓰므로 pixi.js를 최소 페이크로 목킹하고, 분기 선택과 프레임
// 슬라이스 좌표만 검증한다(절차 생성 경로는 기존 characterFactory.test.ts 담당).
import { afterEach, describe, expect, it, vi } from "vitest";

vi.mock("pixi.js", () => {
  class Rectangle {
    constructor(
      public x: number,
      public y: number,
      public w: number,
      public h: number
    ) {}
  }
  class Texture {
    source: { scaleMode: string; src?: unknown };
    frame?: Rectangle;
    static from(src: unknown) {
      const t = new Texture();
      t.source = { scaleMode: "linear", src };
      return t;
    }
    constructor(opts?: { source: Texture["source"]; frame: Rectangle }) {
      this.source = opts?.source ?? { scaleMode: "linear" };
      this.frame = opts?.frame;
    }
  }
  return { Texture, Rectangle };
});

import {
  setSpriteOverride,
  resetSpriteOverrides,
} from "../spriteOverrides";
import { createCharacterAssets, assetsFromCustomSheet } from "../characterFactory";
import type { AgentProfile } from "../../types";

afterEach(() => resetSpriteOverrides());

const profile: AgentProfile = {
  id: "a1",
  name: "Ada",
  role: "backend",
  note: "",
  seed: "seed-1",
  createdAt: 1,
  deskIndex: 0,
};

const FAKE_SHEET = { fake: "sheet" } as unknown as CanvasImageSource;

describe("createCharacterAssets custom override", () => {
  it("오버라이드가 있으면 커스텀 시트 텍스처를 쓴다", () => {
    setSpriteOverride("a1", FAKE_SHEET);
    const assets = createCharacterAssets(profile);
    expect((assets.base.source as { src?: unknown }).src).toBe(FAKE_SHEET);
    expect(assets.base.source.scaleMode).toBe("nearest");
    expect(assets.descriptor).toEqual({
      archetype: "custom",
      hair: "custom",
      clothes: "custom",
      accessory: "custom",
    });
  });

  it("프레임을 idle0/idle1/walk0/walk1 순서로 16px 간격 슬라이스한다", () => {
    const assets = assetsFromCustomSheet(FAKE_SHEET);
    expect(assets.frames.idle0.frame).toMatchObject({ x: 0, y: 0, w: 16, h: 16 });
    expect(assets.frames.idle1.frame).toMatchObject({ x: 16, y: 0 });
    expect(assets.frames.walk0.frame).toMatchObject({ x: 32, y: 0 });
    expect(assets.frames.walk1.frame).toMatchObject({ x: 48, y: 0 });
    expect(assets.idle).toEqual([assets.frames.idle0, assets.frames.idle1]);
    expect(assets.walk).toEqual([assets.frames.walk0, assets.frames.walk1]);
  });

  it("절차 없는 커스텀 에셋의 cellSize 기본은 16이다", () => {
    const assets = assetsFromCustomSheet(FAKE_SHEET);
    expect(assets.cellSize).toBe(16);
  });

  it("N=height 기준으로 슬라이스하고 cellSize를 N으로 설정한다", () => {
    const sheet = { width: 128, height: 32 } as unknown as CanvasImageSource;
    const assets = assetsFromCustomSheet(sheet);
    expect(assets.cellSize).toBe(32);
    expect(assets.frames.idle0.frame).toMatchObject({ x: 0, y: 0, w: 32, h: 32 });
    expect(assets.frames.idle1.frame).toMatchObject({ x: 32, y: 0, w: 32, h: 32 });
    expect(assets.frames.walk0.frame).toMatchObject({ x: 64, y: 0 });
    expect(assets.frames.walk1.frame).toMatchObject({ x: 96, y: 0 });
  });
});
