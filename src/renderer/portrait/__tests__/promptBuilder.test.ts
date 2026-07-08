// src/renderer/portrait/__tests__/promptBuilder.test.ts
import { describe, expect, it } from "vitest";
import {
  buildPortraitPrompt,
  buildSpritePrompt,
  buildPixelLabSpriteDescription,
} from "../promptBuilder";
import { makeRng, hashStringToSeed } from "../../office/gen/prng";
import { generatePalette } from "../../office/gen/palette";
import { ARCHETYPES } from "../../office/gen/archetypes";

function expectedHex(seed: string, which: "hair" | "shirt"): string {
  const pal = generatePalette(makeRng(hashStringToSeed(seed)));
  const rgb = which === "hair" ? pal.hair.base : pal.shirt.base;
  return "#" + (rgb & 0xffffff).toString(16).padStart(6, "0");
}

describe("buildPortraitPrompt", () => {
  it("includes name, role, note and appearance", () => {
    const p = buildPortraitPrompt({
      name: "Ada",
      role: "backend engineer",
      note: "calm and precise",
      appearance: "short black bob, round glasses",
      seed: "seed-xyz",
    });
    expect(p).toContain("Ada");
    expect(p).toContain("backend engineer");
    expect(p).toContain("calm and precise");
    expect(p).toContain("short black bob, round glasses");
  });

  it("embeds palette-derived hair and clothing hex from the seed", () => {
    const seed = "seed-xyz";
    const p = buildPortraitPrompt({ name: "A", role: "r", note: "", seed });
    expect(p).toContain(expectedHex(seed, "hair"));
    expect(p).toContain(expectedHex(seed, "shirt"));
  });

  it("omits note/appearance lines when empty/absent", () => {
    const p = buildPortraitPrompt({ name: "A", role: "r", note: "", seed: "s" });
    expect(p).not.toContain("Personality");
    expect(p).not.toContain("Appearance details");
  });

  it("states the 90s bishoujo style and 240x320 / 3:4 spec", () => {
    const p = buildPortraitPrompt({ name: "A", role: "r", note: "", seed: "s" });
    expect(p.toLowerCase()).toContain("bishoujo");
    expect(p).toContain("240x320");
    expect(p).toContain("3:4");
  });

  it("밝고 귀여운 무드 문구를 포함한다 (의도적 계약 변경)", () => {
    const p = buildPortraitPrompt({ name: "A", role: "r", note: "", seed: "s" });
    expect(p).toContain("cheerful pastel color grading");
    expect(p).toContain("friendly smile");
  });

  it("is deterministic for a given seed", () => {
    const input = { name: "A", role: "r", note: "n", seed: "same" };
    expect(buildPortraitPrompt(input)).toBe(buildPortraitPrompt(input));
  });
});

describe("buildSpritePrompt", () => {
  const base = { name: "Ada", role: "backend engineer", seed: "seed-1" };

  it("16x16 픽셀 아트 단일 캐릭터를 의뢰하고 이름/역할을 포함한다", () => {
    const p = buildSpritePrompt(base);
    expect(p).toContain("16x16 pixel art");
    expect(p).toContain("Ada");
    expect(p).toContain("backend engineer");
    expect(p).toContain("No text, no watermark");
  });

  it("밝고 귀여운 SNES-era JRPG 스타일 문구를 포함한다 (의도적 계약 변경)", () => {
    const p = buildSpritePrompt(base);
    expect(p).toContain("16-bit SNES-era Japanese RPG");
    expect(p).toContain("chibi");
    expect(p).toContain("soft bright pastel colors");
    expect(p).toContain("clean black outlines");
    expect(p).toContain("no anti-aliasing");
  });

  it("같은 seed면 초상 프롬프트와 동일한 머리/옷 hex 색을 쓴다", () => {
    const sprite = buildSpritePrompt(base);
    const portrait = buildPortraitPrompt({ ...base, note: "" });
    const hexes = portrait.match(/#[0-9a-f]{6}/g)!;
    for (const h of hexes) expect(sprite).toContain(h);
  });

  it("spriteRequest가 있으면 Details로 포함한다", () => {
    const p = buildSpritePrompt({ ...base, spriteRequest: "red cloak wizard" });
    expect(p).toContain("Details: red cloak wizard.");
  });

  it("spriteRequest가 비면 appearance로 폴백한다", () => {
    const p = buildSpritePrompt({ ...base, spriteRequest: "  ", appearance: "short black hair" });
    expect(p).toContain("Details: short black hair.");
  });

  it("둘 다 없으면 Details 줄이 없다", () => {
    expect(buildSpritePrompt(base)).not.toContain("Details:");
  });
});

describe("archetype-aware prompts", () => {
  it("human (archetype omitted) prompt is unchanged: bishoujo + hair/clothing hints", () => {
    const p = buildPortraitPrompt({ name: "A", role: "r", note: "", seed: "s" });
    expect(p.toLowerCase()).toContain("bishoujo");
    expect(p).toContain("hand-drawn anime face");
    expect(p).toContain("Hair color approximately");
    expect(p).toContain("Clothing color approximately");
  });

  it("orc portrait injects the orc subject and keeps bishoujo (humanoid)", () => {
    const orc = buildPortraitPrompt({ name: "Grug", role: "sysadmin", note: "", seed: "s", archetype: "orc" });
    expect(orc).toContain("green-skinned tusked orc");
    expect(orc.toLowerCase()).toContain("bishoujo"); // orc는 휴머노이드 → bishoujo 유지
  });

  it("robot portrait is non-humanoid: no bishoujo, uses 'anime style character' + chassis/accent hints", () => {
    const robot = buildPortraitPrompt({ name: "Unit", role: "ops", note: "", seed: "s", archetype: "robot" });
    expect(robot.toLowerCase()).not.toContain("bishoujo");
    expect(robot).toContain("anime style character");
    expect(robot).toContain("Chassis color approximately");
    expect(robot).toContain("Accent color approximately");
    expect(robot).toContain("boxy utility robot with a monitor face");
  });

  it("sprite prompt mirrors archetype color hints (slime body color)", () => {
    const slime = buildSpritePrompt({ name: "Goo", role: "intern", seed: "s", archetype: "slime" });
    expect(slime).toContain("Body color approximately");
    expect(slime).toContain("translucent gelatinous slime creature");
  });

  it("embedded hex matches the archetype palette (not the human palette)", () => {
    const pal = ARCHETYPES.orc.generatePalette(makeRng(hashStringToSeed("s")));
    const p = buildPortraitPrompt({ name: "A", role: "r", note: "", seed: "s", archetype: "orc" });
    expect(p).toContain("#" + (pal.skin.base & 0xffffff).toString(16).padStart(6, "0"));
  });

  it("unknown archetype falls back to human phrasing", () => {
    const p = buildPortraitPrompt({ name: "A", role: "r", note: "", seed: "s", archetype: "dragon" });
    expect(p.toLowerCase()).toContain("bishoujo");
  });
});

describe("buildPixelLabSpriteDescription", () => {
  const base = { name: "Ada", role: "engineer", seed: "seed-xyz" };

  it("이름/역할/의뢰 문구를 포함한다", () => {
    const p = buildPixelLabSpriteDescription({ ...base, spriteRequest: "red cloak wizard" });
    expect(p).toContain("Ada");
    expect(p).toContain("engineer");
    expect(p).toContain("Details: red cloak wizard.");
  });

  it("spriteRequest가 비면 appearance로 폴백한다", () => {
    const p = buildPixelLabSpriteDescription({ ...base, spriteRequest: "  ", appearance: "short black hair" });
    expect(p).toContain("Details: short black hair.");
  });

  it("크기·배경 문구가 없다 (image_size/no_background 파라미터가 담당)", () => {
    const p = buildPixelLabSpriteDescription(base);
    expect(p).not.toContain("16x16");
    expect(p.toLowerCase()).not.toContain("background");
  });

  it("같은 입력에 결정적이고 시드 팔레트 힌트를 포함한다", () => {
    const a = buildPixelLabSpriteDescription(base);
    const b = buildPixelLabSpriteDescription(base);
    expect(a).toBe(b);
    expect(a).toContain(expectedHex("seed-xyz", "hair"));
  });

  it("buildSpritePrompt와 동일한 JRPG 톤을 공유한다", () => {
    const p = buildPixelLabSpriteDescription(base);
    expect(p).toContain("16-bit SNES-era Japanese RPG");
    expect(p).toContain("chibi");
    expect(p).toContain("soft bright pastel colors");
  });

  // 계약 갱신(2026-07): 밝고 귀여운 JRPG 톤으로 의도적 문구 변경.
  // 클립보드 프롬프트는 여전히 크기(16x16)·배경(plain solid background) 문구를 가진다.
  it("buildSpritePrompt는 크기/배경 문구(16x16/plain solid background)를 유지한다", () => {
    const p = buildSpritePrompt(base);
    expect(p).toContain("16x16 pixel art style");
    expect(p).toContain("plain solid background");
  });
});
