// src/renderer/portrait/__tests__/promptBuilder.test.ts
import { describe, expect, it } from "vitest";
import {
  buildPortraitPrompt,
  buildSpritePrompt,
  buildCodexPortraitPrompt,
  buildCodexSpritePrompt,
  buildMinimiPrompt,
  buildCodexMinimiPrompt,
  autoMinimiRequestLine,
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
  it("includes name, role, personality and portraitRequest", () => {
    const p = buildPortraitPrompt({
      name: "Ada",
      role: "backend engineer",
      personality: "calm and precise",
      portraitRequest: "short black bob, round glasses",
      seed: "seed-xyz",
    });
    expect(p).toContain("Ada");
    expect(p).toContain("backend engineer");
    expect(p).toContain("calm and precise");
    expect(p).toContain("short black bob, round glasses");
  });

  it("embeds palette-derived hair and clothing hex from the seed", () => {
    const seed = "seed-xyz";
    const p = buildPortraitPrompt({ name: "A", role: "r", personality: "", seed });
    expect(p).toContain(expectedHex(seed, "hair"));
    expect(p).toContain(expectedHex(seed, "shirt"));
  });

  it("omits personality/portraitRequest lines when empty/absent", () => {
    const p = buildPortraitPrompt({ name: "A", role: "r", personality: "", seed: "s" });
    expect(p).not.toContain("Personality");
    expect(p).not.toContain("Appearance details");
  });

  it("states the 90s bishoujo style and 240x320 / 3:4 spec", () => {
    const p = buildPortraitPrompt({ name: "A", role: "r", personality: "", seed: "s" });
    expect(p.toLowerCase()).toContain("bishoujo");
    expect(p).toContain("240x320");
    expect(p).toContain("3:4");
  });

  it("밝고 귀여운 무드 문구를 포함한다 (의도적 계약 변경)", () => {
    const p = buildPortraitPrompt({ name: "A", role: "r", personality: "", seed: "s" });
    expect(p).toContain("cheerful pastel color grading");
    expect(p).toContain("friendly smile");
  });

  it("is deterministic for a given seed", () => {
    const input = { name: "A", role: "r", personality: "n", seed: "same" };
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
    const portrait = buildPortraitPrompt({ ...base, personality: "" });
    const hexes = portrait.match(/#[0-9a-f]{6}/g)!;
    for (const h of hexes) expect(sprite).toContain(h);
  });

  it("spriteRequest가 있으면 Details로 포함한다", () => {
    const p = buildSpritePrompt({ ...base, spriteRequest: "red cloak wizard" });
    expect(p).toContain("Details: red cloak wizard.");
  });

  it("spriteRequest가 비면 Details 줄이 없다(초상화 칸으로 폴백하지 않는다)", () => {
    const p = buildSpritePrompt({ ...base, spriteRequest: "  " });
    expect(p).not.toContain("Details:");
  });

  it("둘 다 없으면 Details 줄이 없다", () => {
    expect(buildSpritePrompt(base)).not.toContain("Details:");
  });
});

describe("archetype-aware prompts", () => {
  it("human (archetype omitted) prompt is unchanged: bishoujo + hair/clothing hints", () => {
    const p = buildPortraitPrompt({ name: "A", role: "r", personality: "", seed: "s" });
    expect(p.toLowerCase()).toContain("bishoujo");
    expect(p).toContain("hand-drawn anime face");
    expect(p).toContain("Hair color approximately");
    expect(p).toContain("Clothing color approximately");
  });

  it("orc portrait injects the orc subject and keeps bishoujo (humanoid)", () => {
    const orc = buildPortraitPrompt({ name: "Grug", role: "sysadmin", personality: "", seed: "s", archetype: "orc" });
    expect(orc).toContain("green-skinned tusked orc");
    expect(orc.toLowerCase()).toContain("bishoujo"); // orc는 휴머노이드 → bishoujo 유지
  });

  it("robot portrait is non-humanoid: no bishoujo, uses 'anime style character' + chassis/accent hints", () => {
    const robot = buildPortraitPrompt({ name: "Unit", role: "ops", personality: "", seed: "s", archetype: "robot" });
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
    const p = buildPortraitPrompt({ name: "A", role: "r", personality: "", seed: "s", archetype: "orc" });
    expect(p).toContain("#" + (pal.skin.base & 0xffffff).toString(16).padStart(6, "0"));
  });

  it("unknown archetype falls back to human phrasing", () => {
    const p = buildPortraitPrompt({ name: "A", role: "r", personality: "", seed: "s", archetype: "dragon" });
    expect(p.toLowerCase()).toContain("bishoujo");
  });
});

describe("buildSpritePrompt", () => {
  const base = { name: "Ada", role: "engineer", seed: "seed-xyz" };

  it("이름/역할/의뢰 문구를 포함한다", () => {
    const p = buildSpritePrompt({ ...base, spriteRequest: "red cloak wizard" });
    expect(p).toContain("Ada");
    expect(p).toContain("engineer");
    expect(p).toContain("Details: red cloak wizard.");
  });

  it("spriteRequest가 비면 Details 줄이 없다(초상화 칸으로 폴백하지 않는다)", () => {
    const p = buildSpritePrompt({ ...base, spriteRequest: "  " });
    expect(p).not.toContain("Details:");
  });

  it("같은 입력에 결정적이고 시드 팔레트 힌트를 포함한다", () => {
    const a = buildSpritePrompt(base);
    const b = buildSpritePrompt(base);
    expect(a).toBe(b);
    expect(a).toContain(expectedHex("seed-xyz", "hair"));
  });

  // 계약 갱신(2026-07): 밝고 귀여운 JRPG 톤으로 의도적 문구 변경.
  // 클립보드 프롬프트는 크기(16x16)·배경(plain solid background) 문구를 가진다.
  it("크기/배경 문구(16x16/plain solid background)를 유지한다", () => {
    const p = buildSpritePrompt(base);
    expect(p).toContain("16x16 pixel art style");
    expect(p).toContain("plain solid background");
    expect(p).toContain("16-bit SNES-era Japanese RPG");
    expect(p).toContain("chibi");
    expect(p).toContain("soft bright pastel colors");
  });
});

describe("codex 생성 프롬프트", () => {
  const base = { name: "Ada", role: "engineer", seed: "seed-xyz" };

  it("초상: 본문은 클립보드 프롬프트와 같고 규격 줄만 1024x1536으로 바뀐다", () => {
    const input = { ...base, personality: "차분함" };
    const clip = buildPortraitPrompt(input);
    const codex = buildCodexPortraitPrompt(input);
    // 규격 줄을 뺀 본문은 완전히 동일해야 한다.
    expect(codex.split("\n").slice(0, -1)).toEqual(clip.split("\n").slice(0, -1));
    expect(codex).toContain("1024x1536");
    expect(codex).not.toContain("240x320");
    expect(codex).toContain("Ada");
  });

  it("스프라이트: 규격 줄만 1024x1024 + 투명 배경으로 바뀐다", () => {
    const clip = buildSpritePrompt(base);
    const codex = buildCodexSpritePrompt(base);
    expect(codex.split("\n").slice(0, -1)).toEqual(clip.split("\n").slice(0, -1));
    expect(codex).toContain("1024x1024");
    expect(codex).toContain("transparent background");
    // 스타일 본문은 그대로 남는다(16x16 픽셀아트 룩 지시).
    expect(codex).toContain("16x16 pixel art style");
  });

  it("같은 입력에 결정적이고 의뢰 문구를 반영한다", () => {
    const a = buildCodexSpritePrompt({ ...base, spriteRequest: "red cloak wizard" });
    const b = buildCodexSpritePrompt({ ...base, spriteRequest: "red cloak wizard" });
    expect(a).toBe(b);
    expect(a).toContain("Details: red cloak wizard.");
  });
});

describe("목록에 없는 커스텀 아키타입", () => {
  const base = { name: "Nia", role: "engineer", personality: "", seed: "seed-custom" };

  it("초상 프롬프트의 주제 서술자를 적은 문구로 대체한다", () => {
    const p = buildPortraitPrompt({ ...base, archetype: "a tiny wise dragon" });
    expect(p).toContain("Character: Nia, a engineer (a tiny wise dragon).");
  });

  it("픽셀아트 프롬프트에도 같은 문구가 들어간다", () => {
    const p = buildSpritePrompt({
      name: "Nia",
      role: "engineer",
      seed: "seed-custom",
      archetype: "드래곤",
    });
    expect(p).toContain("(드래곤)");
  });

  it("알려진 id/auto는 아키타입 서술자를 그대로 쓴다", () => {
    const orc = buildPortraitPrompt({ ...base, archetype: "orc" });
    expect(orc).toContain("green-skinned tusked orc");
    const auto = buildPortraitPrompt({ ...base, archetype: "auto" });
    expect(auto).not.toContain("(auto)");
  });
});

describe("미니미(소환수) 프롬프트", () => {
  const base = { name: "Ada", role: "engineer", seed: "seed-xyz" };

  it("소환수 관계와 16x16 단일 프레임 지시를 담는다", () => {
    const p = buildMinimiPrompt(base);
    expect(p).toContain("summoned familiar");
    expect(p).toContain("16x16 pixel art style");
    expect(p).toContain("Master: Ada, a engineer.");
  });

  it("전용 의뢰 문구가 있으면 그대로 싣고 자동 위임 문구는 넣지 않는다", () => {
    const p = buildMinimiPrompt({ ...base, minimiRequest: "a tiny flame spirit" });
    expect(p).toContain("Familiar: a tiny flame spirit.");
    expect(p).not.toContain(autoMinimiRequestLine("Ada"));
  });

  it("전용 의뢰 문구가 비면 본체에 어울리게 만들어 달라는 문구가 자동으로 들어간다", () => {
    for (const req of [undefined, "", "   "]) {
      const p = buildMinimiPrompt({ ...base, minimiRequest: req });
      expect(p).toContain(autoMinimiRequestLine("Ada"));
      expect(p).not.toContain("Familiar: .");
    }
  });

  it("본체 스프라이트 추가 프롬프트는 주인 묘사로만 실린다", () => {
    const p = buildMinimiPrompt({ ...base, spriteRequest: "red cloak wizard" });
    expect(p).toContain("Master's appearance: red cloak wizard.");
    expect(buildMinimiPrompt(base)).not.toContain("Master's appearance:");
  });

  it("커스텀 아키타입 문구가 주인 서술자를 대체한다", () => {
    const p = buildMinimiPrompt({ ...base, archetype: "드래곤" });
    expect(p).toContain("(드래곤)");
  });

  it("codex 버전은 규격 줄만 1024x1024 + 투명 배경으로 바뀐다", () => {
    const clip = buildMinimiPrompt(base);
    const codex = buildCodexMinimiPrompt(base);
    expect(codex.split("\n").slice(0, -1)).toEqual(clip.split("\n").slice(0, -1));
    expect(codex).toContain("1024x1024");
    expect(codex).toContain("transparent background");
  });

  it("같은 입력에 결정적이다", () => {
    expect(buildMinimiPrompt(base)).toBe(buildMinimiPrompt(base));
  });
});
