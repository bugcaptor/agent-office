// src/renderer/office/gen/__tests__/colorOverrides.test.ts
//
// 사용자 색 오버라이드(kbm #2fj). 시드를 바꾸지 않고 색만 갈아 끼우는 경로가
// ① 팔레트 램프 ② 키 컬러(=프롬프트에 실리는 hex) ③ 실제 시트 픽셀에서
// 모두 같은 색을 가리키는지 본다 — 셋이 어긋나면 UI가 거짓말을 하게 된다.
import { describe, expect, it } from "vitest";
import { makeRng, hashStringToSeed } from "../prng";
import {
  applyColorOverrides,
  hasColorOverrides,
  parseHexColor,
  rampFromColor,
  rgbToHsl,
  hslToRgb,
  generatePalette,
} from "../palette";
import {
  ARCHETYPE_IDS,
  basePaletteFor,
  getArchetype,
  hexColor,
  keyColorsFor,
} from "../archetypes";
import { generateSheet, selectLayers } from "../characterFactory";
import { createTestCanvasFactory, sheetToPixels } from "./helpers";

const SEED = "override-seed";

describe("parseHexColor", () => {
  it("# 생략·3자리 축약·대문자를 모두 받는다", () => {
    expect(parseHexColor("#ff8800")).toBe(0xff8800);
    expect(parseHexColor("ff8800")).toBe(0xff8800);
    expect(parseHexColor("#F80")).toBe(0xff8800);
    expect(parseHexColor(" #f80 ")).toBe(0xff8800);
  });

  it("형식이 아니면 undefined", () => {
    for (const bad of ["", "#", "#ff88", "#gggggg", undefined]) {
      expect(parseHexColor(bad)).toBeUndefined();
    }
  });
});

describe("rgbToHsl", () => {
  it("hslToRgb의 역함수다(왕복 보존)", () => {
    for (const rgb of [0xff0000, 0x00ff00, 0x0000ff, 0x123456, 0x808080, 0x000000, 0xffffff]) {
      const [h, s, l] = rgbToHsl(rgb);
      expect(hslToRgb(h, s, l)).toBe(rgb);
    }
  });
});

describe("rampFromColor", () => {
  it("base는 고른 색 그대로다 — 칩·프롬프트·픽셀이 어긋나지 않게", () => {
    expect(rampFromColor(0x123456, 0.16).base).toBe(0x123456);
  });

  it("그림자는 더 어둡고 하이라이트는 더 밝다", () => {
    const r = rampFromColor(0x3366cc, 0.16);
    const l = (rgb: number) => rgbToHsl(rgb)[2];
    expect(l(r.shadow)).toBeLessThan(l(r.base));
    expect(l(r.light)).toBeGreaterThan(l(r.base));
  });
});

describe("applyColorOverrides", () => {
  const pal = generatePalette(makeRng(hashStringToSeed(SEED)));

  it("지정한 슬롯만 갈아 끼우고 나머지는 원본 그대로다", () => {
    const out = applyColorOverrides(pal, { hair: "#ff8800" });
    expect(out.hair.base).toBe(0xff8800);
    expect(out.skin).toBe(pal.skin);
    expect(out.shirt).toBe(pal.shirt);
    expect(out.pants).toBe(pal.pants);
    expect(out.outline).toBe(pal.outline);
  });

  it("값이 없거나 형식이 아니면 원본을 그대로 돌려준다(같은 참조)", () => {
    expect(applyColorOverrides(pal, undefined)).toBe(pal);
    expect(applyColorOverrides(pal, {})).toBe(pal);
    expect(applyColorOverrides(pal, { skin: "not-a-color" })).toBe(pal);
  });

  it("원본 팔레트를 변형하지 않는다(순수)", () => {
    const before = pal.hair.base;
    applyColorOverrides(pal, { hair: "#010203" });
    expect(pal.hair.base).toBe(before);
  });

  it("셔츠/피부 대비가 낮아지는 선택도 사용자의 뜻대로 통과시킨다", () => {
    const out = applyColorOverrides(pal, { skin: "#808080", shirt: "#828282" });
    expect(out.skin.base).toBe(0x808080);
    expect(out.shirt.base).toBe(0x828282);
  });
});

describe("hasColorOverrides", () => {
  it("유효한 hex가 하나라도 있어야 참", () => {
    expect(hasColorOverrides(undefined)).toBe(false);
    expect(hasColorOverrides({})).toBe(false);
    expect(hasColorOverrides({ hair: "" })).toBe(false);
    expect(hasColorOverrides({ hair: "nope" })).toBe(false);
    expect(hasColorOverrides({ hair: "#abc" })).toBe(true);
  });
});

describe("keyColorsFor + 오버라이드", () => {
  it("모든 아키타입에서 오버라이드한 슬롯의 칩 색이 고른 색과 정확히 같다", () => {
    for (const id of ARCHETYPE_IDS) {
      const slots = new Set(keyColorsFor(SEED, id).map((c) => c.slot));
      for (const slot of slots) {
        const colors = { [slot]: "#0f9d58" };
        const chip = keyColorsFor(SEED, id, colors).find((c) => c.slot === slot);
        expect(hexColor(chip!.rgb)).toBe("#0f9d58");
      }
    }
  });

  it("색 힌트 라인도 같은 hex를 싣는다 — 칩과 프롬프트가 한 값", () => {
    const arch = getArchetype("human");
    const pal = applyColorOverrides(
      arch.generatePalette(makeRng(hashStringToSeed(SEED))),
      { hair: "#0f9d58" },
    );
    expect(arch.promptDescriptor(pal).colorHints).toContain("Hair color approximately #0f9d58");
  });

  it("오버라이드가 없으면 기본 팔레트와 같은 색이다", () => {
    expect(keyColorsFor(SEED, "human", {})).toEqual(keyColorsFor(SEED, "human"));
    const base = basePaletteFor(SEED, "human");
    const hair = keyColorsFor(SEED, "human").find((c) => c.slot === "hair");
    expect(hair!.rgb).toBe(base.hair.base);
  });

  it("basePaletteFor는 오버라이드와 무관하게 시드의 기본색을 준다", () => {
    const base = basePaletteFor(SEED, "human");
    const overridden = keyColorsFor(SEED, "human", { hair: "#0f9d58" });
    expect(overridden.find((c) => c.slot === "hair")!.rgb).toBe(0x0f9d58);
    expect(basePaletteFor(SEED, "human").hair.base).toBe(base.hair.base);
  });
});

describe("시트 생성 + 오버라이드", () => {
  it("파츠 픽(시드 결정)은 그대로 두고 팔레트만 바뀐다", () => {
    const plain = selectLayers(SEED, "human");
    const tinted = selectLayers(SEED, "human", { hair: "#0f9d58" });
    expect(tinted.descriptor).toEqual(plain.descriptor);
    expect(tinted.pal.hair.base).toBe(0x0f9d58);
    expect(tinted.pal.skin).toEqual(plain.pal.skin);
  });

  it("실제 시트 픽셀이 달라지고, 같은 오버라이드에 결정적이다", () => {
    const plain = sheetToPixels(generateSheet(SEED, createTestCanvasFactory(), "human").sheet);
    const a = sheetToPixels(
      generateSheet(SEED, createTestCanvasFactory(), "human", { hair: "#0f9d58" }).sheet,
    );
    const b = sheetToPixels(
      generateSheet(SEED, createTestCanvasFactory(), "human", { hair: "#0f9d58" }).sheet,
    );
    expect(a).toEqual(b);
    expect(a).not.toEqual(plain);
  });

  it("빈 오버라이드는 오버라이드 없음과 픽셀 단위로 같다(회귀 계약)", () => {
    const plain = sheetToPixels(generateSheet(SEED, createTestCanvasFactory(), "human").sheet);
    const empty = sheetToPixels(
      generateSheet(SEED, createTestCanvasFactory(), "human", {}).sheet,
    );
    expect(empty).toEqual(plain);
  });
});
