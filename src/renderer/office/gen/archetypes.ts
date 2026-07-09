// src/renderer/office/gen/archetypes.ts
//
// 아키타입(종족) 레지스트리. 각 아키타입은 자체 팔레트 규칙과 4프레임 생성
// (휴머노이드=layers 경로 재사용, 비휴머노이드=frames 경로)을 제공한다.
// human은 기존 파이프라인과 seed 단위 바이트 동일(회귀 계약, B3).
// gen/은 DOM/Pixi 비의존 — 이 파일도 마찬가지(promptBuilder가 직접 import).
import { makeRng, hashStringToSeed, type Rng } from "./prng";
import {
  generatePalette, ramp, contrastRatio, clampShirtRamp, SHIRT_SKIN_MIN_CONTRAST,
  type CharacterPalette,
} from "./palette";
import {
  BODY_BASE_FRONT, LEGS_WALK_A, LEGS_WALK_B,
  HAIR_VARIANTS, CLOTHES_VARIANTS, ACCESSORY_VARIANTS,
  HAIR_KEYS, CLOTHES_KEYS, ACCESSORY_KEYS,
  EMPTY16,
  type PixelRows,
} from "./parts";
import {
  composeSpriteSheet, composeFramesSheet, defaultCanvasFactory,
  type CharacterLayers, type FrameGrids,
  type SpriteSheetResult, type CanvasFactory,
} from "./compositor";

interface ArchetypePromptDescriptor {
  humanoid: boolean;  // true -> "bishoujo"/"anime face" 문구 사용
  subject: string;    // 주제 서술자(예: "a green-skinned tusked orc"). human은 "".
  colorHints: string; // 색 힌트 라인(아키타입별 라벨)
}

export type ArchetypeSheet =
  | { kind: "layers"; layers: CharacterLayers }
  | { kind: "frames"; frames: FrameGrids };

interface ArchetypeBuild {
  sheet: ArchetypeSheet;
  descriptor: { hair: string; clothes: string; accessory: string };
}

export interface Archetype {
  id: string;
  label: string;
  generatePalette(rng: Rng): CharacterPalette;
  buildFrames(rng: Rng, pal: CharacterPalette): ArchetypeBuild;
  promptDescriptor(pal: CharacterPalette): ArchetypePromptDescriptor;
}

function hex(rgb: number): string {
  return "#" + (rgb & 0xffffff).toString(16).padStart(6, "0");
}

/** 휴머노이드 공통 layers 조립(파츠 3종 픽 + 선택적 overlay/underlay). */
function humanoidBuild(
  rng: Rng, overlay?: CharacterLayers["overlay"], underlay?: CharacterLayers["underlay"],
): ArchetypeBuild {
  const hair = rng.pick(HAIR_KEYS);
  const clothes = rng.pick(CLOTHES_KEYS);
  const accessory = rng.pick(ACCESSORY_KEYS);
  return {
    descriptor: { hair, clothes, accessory },
    sheet: {
      kind: "layers",
      layers: {
        body: BODY_BASE_FRONT,
        clothes: CLOTHES_VARIANTS[clothes],
        hair: HAIR_VARIANTS[hair],
        accessory: ACCESSORY_VARIANTS[accessory],
        legsWalkA: LEGS_WALK_A,
        legsWalkB: LEGS_WALK_B,
        ...(overlay ? { overlay } : {}),
        ...(underlay ? { underlay } : {}),
      },
    },
  };
}

// ── human: 기존 파이프라인과 완전 동일(회귀 계약). generatePalette 그대로,
//    파츠 픽 순서 그대로 -> seed 단위 바이트 동일.
const human: Archetype = {
  id: "human",
  label: "인간",
  generatePalette: (rng) => generatePalette(rng),
  buildFrames: (rng) => humanoidBuild(rng),
  promptDescriptor: (pal) => ({
    humanoid: true,
    subject: "",
    colorHints: `Hair color approximately ${hex(pal.hair.base)}. Clothing color approximately ${hex(pal.shirt.base)}.`,
  }),
};

type Tone = readonly [number, number, number];

/**
 * 휴머노이드 공통 팔레트 규칙(피부 톤/헤어 휴만 아키타입별로 교체).
 * rng 소비 순서는 `generatePalette`(palette.ts)와 동일: skin pick → hair hue
 * pick + hairL range + hair ramp(hue/sat/l) → shirt ramp(hue/sat/l) [+ 재시도
 * 루프 최대 8회 hue/sat/l 3개씩 소비] [+ 클램프 시 hue 1회 추가] → pants
 * ramp(hue/sat/l). 이 순서를 어기면 시드 결정성 계약(byte-compat)이 깨진다.
 */
function humanoidPalette(rng: Rng, skinTones: readonly Tone[], hairHues: number[]): CharacterPalette {
  const [sh, ss, sl] = rng.pick(skinTones);
  const skin = ramp(sh, ss, sl, 0.1);

  const hairHue = rng.pick(hairHues);
  const hairL = rng.range(0.18, 0.6);
  const hair = ramp(hairHue, rng.range(0.25, 0.7), hairL, 0.14);

  let shirt = ramp(rng.range(0, 360), rng.range(0.4, 0.85), rng.range(0.35, 0.6));
  for (let i = 0; i < 8 && contrastRatio(shirt.base, skin.base) < SHIRT_SKIN_MIN_CONTRAST; i++) {
    shirt = ramp(rng.range(0, 360), rng.range(0.4, 0.85), rng.range(0.3, 0.62));
  }
  if (contrastRatio(shirt.base, skin.base) < SHIRT_SKIN_MIN_CONTRAST) {
    shirt = clampShirtRamp(rng.range(0, 360), skin.base, sl > 0.5);
  }
  const pants = ramp(rng.range(0, 360), rng.range(0.2, 0.6), rng.range(0.22, 0.42));
  return { skin, hair, shirt, pants, outline: 0x1a1420 };
}

const ELF_SKIN: readonly Tone[] = [[35, 0.28, 0.86], [140, 0.16, 0.8], [28, 0.2, 0.82], [150, 0.2, 0.72]];
const ELF_HAIR = [45, 0, 120, 200, 280];       // 금/은/녹/청/보라
const ORC_SKIN: readonly Tone[] = [[110, 0.35, 0.4], [110, 0.28, 0.32], [100, 0.22, 0.5], [150, 0.12, 0.45]];
const ORC_HAIR = [20, 0, 220, 280];            // 흑/갈/어두운 톤
const BEASTFOLK_SKIN: readonly Tone[] = [[28, 0.45, 0.78], [26, 0.5, 0.66], [24, 0.5, 0.52], [20, 0.5, 0.38], [18, 0.45, 0.28]];
const BEASTFOLK_HAIR = [20, 30, 40, 0, 200];   // 털색(갈/황/흑/적/청)
const ANDROID_SKIN: readonly Tone[] = [[210, 0.08, 0.72], [30, 0.12, 0.75], [0, 0.0, 0.7], [200, 0.05, 0.66]];
const ANDROID_HAIR = [200, 280, 0, 180];

// ── 오버레이/언더레이 그리드(16x16). 문자는 parts.ts의 문자→팔레트 레전드를 따름:
//    's'=skin.base, 'o'=outline(=eye 'e'와 동일 색), 'a'=hair.base, 'W'=흰색 고정.
const ELF_EARS: PixelRows = [
  '................','................','................','.o..........o...',
  '..s........s....','..o........o....','................','................',
  '................','................','................','................',
  '................','................','................','................',
];
const ORC_TUSKS: PixelRows = [
  '................','................','................','................',
  '................','................','....W....W......','....W....W......',
  '................','................','................','................',
  '................','................','................','................',
];
const BEASTFOLK_EARS: PixelRows = [
  '....a....a......','....aa..aa......','................','................',
  '................','................','................','................',
  '................','................','................','................',
  '................','................','................','................',
];
const BEASTFOLK_TAIL: PixelRows = [
  '................','................','................','................',
  '................','................','................','................',
  '................','................','................','...........aa...',
  '............aa..','.............a..','................','................',
];
const ANDROID_OVERLAY: PixelRows = [
  '................','................','................','................',
  '................','................','.....W..W.......','................',
  '.....oooo.......','.......o........','.......o........','.......o........',
  '.......o........','................','................','................',
];

const elf: Archetype = {
  id: "elf", label: "엘프",
  generatePalette: (rng) => humanoidPalette(rng, ELF_SKIN, ELF_HAIR),
  buildFrames: (rng) => humanoidBuild(rng, ELF_EARS),
  promptDescriptor: (pal) => ({
    humanoid: true, subject: "a slender pale pointy-eared elf",
    colorHints: `Hair color approximately ${hex(pal.hair.base)}. Clothing color approximately ${hex(pal.shirt.base)}.`,
  }),
};
const orc: Archetype = {
  id: "orc", label: "오크",
  generatePalette: (rng) => humanoidPalette(rng, ORC_SKIN, ORC_HAIR),
  buildFrames: (rng) => humanoidBuild(rng, ORC_TUSKS),
  promptDescriptor: (pal) => ({
    humanoid: true, subject: "a green-skinned tusked orc",
    colorHints: `Skin color approximately ${hex(pal.skin.base)}. Clothing color approximately ${hex(pal.shirt.base)}.`,
  }),
};
const beastfolk: Archetype = {
  id: "beastfolk", label: "수인",
  generatePalette: (rng) => humanoidPalette(rng, BEASTFOLK_SKIN, BEASTFOLK_HAIR),
  buildFrames: (rng) => humanoidBuild(rng, BEASTFOLK_EARS, BEASTFOLK_TAIL),
  promptDescriptor: (pal) => ({
    humanoid: true, subject: "a beastfolk with animal ears and a tail",
    colorHints: `Fur color approximately ${hex(pal.hair.base)}. Clothing color approximately ${hex(pal.shirt.base)}.`,
  }),
};
const android: Archetype = {
  id: "android", label: "안드로이드",
  generatePalette: (rng) => humanoidPalette(rng, ANDROID_SKIN, ANDROID_HAIR),
  buildFrames: (rng) => humanoidBuild(rng, ANDROID_OVERLAY),
  promptDescriptor: (pal) => ({
    humanoid: true, subject: "a humanoid android with visible panel seams and solid glowing eyes",
    colorHints: `Plating color approximately ${hex(pal.skin.base)}. Clothing color approximately ${hex(pal.shirt.base)}.`,
  }),
};

// ── 비휴머노이드(B5): 파츠(HAIR/CLOTHES/ACCESSORY) 픽 없음 -> descriptor 고정값.
const NON_PARTS_DESCRIPTOR: ArchetypeBuild["descriptor"] = { hair: "none", clothes: "none", accessory: "none" };

// ── Robot: 박스형 유틸리티 로봇 + 모니터 얼굴. chassis=skin, accent=shirt, dark metal=pants.
const ROBOT_BODY: PixelRows = [
  '................','....o....o......','....o....o......','..oooooooooo....',
  '..osssssssso....','..osWWWWWWso....','..osWeWWeWso....','..osWWWWWWso....',
  '..oossssssoo....','..oCcccccCo.....','.ooCcccccCoo....','.osCcccccCso....',
  '..oCcccccCo.....','..opppppppo.....','..opp..ppo......','..oPP..PPo......',
];
const ROBOT_LEGS_A: PixelRows = [
  '................','................','................','................',
  '................','................','................','................',
  '................','................','................','................',
  '................','..opppppppo.....','..oppp.pppo.....','..oPP..PPo......',
];
const ROBOT_LEGS_B: PixelRows = [
  '................','................','................','................',
  '................','................','................','................',
  '................','................','................','................',
  '................','..opppppppo.....','..opp.pppo......','..oPP..PPo......',
];

// ── Slime: 다리 없는 블롭. idle1=스쿼시(납작·광폭), walk0/1=좌/우 기울임.
const SLIME_IDLE0: PixelRows = [
  '................','................','................','................',
  '................','.....oooo.......','....osssso......','...osWsssso.....',
  '...osHssHso.....','..osseSSesso....','..osssssssso....','..osssssssso....',
  '.osssssssssso...','.osssssssssso...','.osssssssssso...','.oooooooooooo...',
];
const SLIME_IDLE1: PixelRows = [
  '................','................','................','................',
  '................','................','................','....oooooo......',
  '...osWsssso.....','..osseSSesso....','..osssssssso....','.osssssssssso...',
  '.osssssssssso...','osssssssssssso..','osssssssssssso..','.oooooooooooo...',
];
const SLIME_WALK0: PixelRows = [
  '................','................','................','................',
  '................','....oooo........','...osssso.......','..osWsssso......',
  '..osHssHso......','.osseSSesso.....','.osssssssso.....','.osssssssso.....',
  'osssssssssso....','osssssssssso....','osssssssssso....','oooooooooooo....',
];
const SLIME_WALK1: PixelRows = [
  '................','................','................','................',
  '................','......oooo......','.....osssso.....','....osWsssso....',
  '....osHssHso....','...osseSSesso...','...osssssssso...','...osssssssso...',
  '..osssssssssso..','..osssssssssso..','..osssssssssso..','..oooooooooooo..',
];

// ── Ghost: 다리 없음, 하단 물결(찢긴 자락). idle1=부유(전체 1px 상승), walk0/1=자락 위상 변화.
const GHOST_IDLE0: PixelRows = [
  '................','................','................','....oooooo......',
  '...osssssso.....','..osssssssso....','..osseSSesso....','..osssssssso....',
  '..osssssssso....','..osssssssso....','..osssssssso....','..osssssssso....',
  '..osssssssso....','..ossossosso....','..ooo.oo.ooo....','................',
];
const GHOST_IDLE1: PixelRows = [
  '................','................','....oooooo......','...osssssso.....',
  '..osssssssso....','..osseSSesso....','..osssssssso....','..osssssssso....',
  '..osssssssso....','..osssssssso....','..osssssssso....','..osssssssso....',
  '..ossossosso....','..ooo.oo.ooo....','................','................',
];
const GHOST_WALK0: PixelRows = [
  '................','................','................','....oooooo......',
  '...osssssso.....','..osssssssso....','..osseSSesso....','..osssssssso....',
  '..osssssssso....','..osssssssso....','..osssssssso....','..osssssssso....',
  '..ossossosso....','..oo.oo.oo.o....','................','................',
];
const GHOST_WALK1: PixelRows = [
  '................','................','................','....oooooo......',
  '...osssssso.....','..osssssssso....','..osseSSesso....','..osssssssso....',
  '..osssssssso....','..osssssssso....','..osssssssso....','..osssssssso....',
  '..osossossos....','..o.oo.oo.oo....','................','................',
];

function robotPalette(rng: Rng): CharacterPalette {
  const grayL = rng.range(0.5, 0.72);
  const chassis = ramp(rng.pick([210, 200, 220, 0]), 0.06, grayL, 0.16); // 근-회색
  const accent = clampShirtRamp(rng.range(0, 360), chassis.base, grayL > 0.5); // 액센트 가독성 보장
  const dark = ramp(210, 0.08, 0.3, 0.12);
  return { skin: chassis, hair: chassis, shirt: accent, pants: dark, outline: 0x11141a };
}
function slimePalette(rng: Rng): CharacterPalette {
  const hue = rng.range(0, 360);
  const body = ramp(hue, rng.range(0.65, 0.95), rng.range(0.45, 0.6), 0.16); // 고채도
  return { skin: body, hair: body, shirt: body, pants: body, outline: 0x11121a };
}
function ghostPalette(rng: Rng): CharacterPalette {
  const hue = rng.pick([210, 180, 260, 160]);
  const body = ramp(hue, rng.range(0.05, 0.18), rng.range(0.78, 0.9), 0.1); // 저채도·창백
  return { skin: body, hair: body, shirt: body, pants: body, outline: 0x555a66 };
}

const robot: Archetype = {
  id: "robot", label: "로봇",
  generatePalette: robotPalette,
  buildFrames: () => ({
    descriptor: { ...NON_PARTS_DESCRIPTOR },
    sheet: {
      kind: "layers",
      layers: {
        body: ROBOT_BODY, clothes: EMPTY16(), hair: EMPTY16(), accessory: EMPTY16(),
        legsWalkA: ROBOT_LEGS_A, legsWalkB: ROBOT_LEGS_B,
      },
    },
  }),
  promptDescriptor: (pal) => ({
    humanoid: false, subject: "a boxy utility robot with a monitor face",
    colorHints: `Chassis color approximately ${hex(pal.skin.base)}. Accent color approximately ${hex(pal.shirt.base)}.`,
  }),
};
const slime: Archetype = {
  id: "slime", label: "슬라임",
  generatePalette: slimePalette,
  buildFrames: () => ({
    descriptor: { ...NON_PARTS_DESCRIPTOR },
    sheet: { kind: "frames", frames: { idle0: SLIME_IDLE0, idle1: SLIME_IDLE1, walk0: SLIME_WALK0, walk1: SLIME_WALK1 } },
  }),
  promptDescriptor: (pal) => ({
    humanoid: false, subject: "a translucent gelatinous slime creature",
    colorHints: `Body color approximately ${hex(pal.skin.base)}.`,
  }),
};
const ghost: Archetype = {
  id: "ghost", label: "유령",
  generatePalette: ghostPalette,
  buildFrames: () => ({
    descriptor: { ...NON_PARTS_DESCRIPTOR },
    sheet: { kind: "frames", frames: { idle0: GHOST_IDLE0, idle1: GHOST_IDLE1, walk0: GHOST_WALK0, walk1: GHOST_WALK1 } },
  }),
  promptDescriptor: (pal) => ({
    humanoid: false, subject: "a floating pale ghost with no legs and a wavy tattered hem",
    colorHints: `Body color approximately ${hex(pal.skin.base)} (pale and translucent).`,
  }),
};

/** 레지스트리. B4(elf/orc/beastfolk/android) + B5(robot/slime/ghost) 완료 — 8종. */
export const ARCHETYPES: Record<string, Archetype> = {
  human, elf, orc, beastfolk, robot, android, slime, ghost,
};

/** 시드 추첨 대상 id의 고정 순서(변경 금지 — pickArchetype 결정성의 기준). */
export const ARCHETYPE_IDS = [
  "human", "elf", "orc", "beastfolk", "robot", "android", "slime", "ghost",
] as const;

export const ARCHETYPE_SELECT_OPTIONS: ReadonlyArray<{ value: string; label: string }> = [
  { value: "auto", label: "자동(시드)" },
  { value: "human", label: "인간" },
  { value: "elf", label: "엘프" },
  { value: "orc", label: "오크" },
  { value: "beastfolk", label: "수인" },
  { value: "robot", label: "로봇" },
  { value: "android", label: "안드로이드" },
  { value: "slime", label: "슬라임" },
  { value: "ghost", label: "유령" },
];

export function getArchetype(id: string | undefined): Archetype {
  return (id && ARCHETYPES[id]) || ARCHETYPES.human;
}

/** 시드 → 아키타입 id. 별도 해시 스트림(seed + ":archetype")으로 팔레트/파츠 rng 미오염. */
export function pickArchetype(seed: string): string {
  return makeRng(hashStringToSeed(seed + ":archetype")).pick(ARCHETYPE_IDS as readonly string[]);
}

/** "auto" -> 시드 추첨, 알려진 id -> 그대로, 그 외(undefined/미지) -> "human". */
export function resolveArchetype(archetype: string | undefined, seed: string): string {
  if (archetype === "auto") return pickArchetype(seed);
  if (archetype && ARCHETYPES[archetype]) return archetype;
  return "human";
}

export function composeArchetypeSheet(
  sheet: ArchetypeSheet, pal: CharacterPalette, factory: CanvasFactory = defaultCanvasFactory,
): SpriteSheetResult {
  return sheet.kind === "layers"
    ? composeSpriteSheet(sheet.layers, pal, factory)
    : composeFramesSheet(sheet.frames, pal, factory);
}
