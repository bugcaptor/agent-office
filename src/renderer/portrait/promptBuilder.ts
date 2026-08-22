// src/renderer/portrait/promptBuilder.ts
//
// 캐릭터 정보 -> 영문 이미지 생성 프롬프트. 순수 함수. 아키타입별 팔레트/서술자/
// 색 힌트를 주입한다. 휴머노이드는 90년대 PC-98 bishoujo 화풍, 비휴머노이드는
// 중립 "anime style character" 문구. characterFactory(pixi) 대신 archetypes/palette만 사용.
// 톤 방향(의도적 계약 변경): 밝고 귀여운 16비트 일본 RPG(SNES-era JRPG) 룩 —
// 치비 비율·파스텔·따뜻한 조명·웃는 표정. 테스트도 이 문구에 맞춰 갱신됨.
import { makeRng, hashStringToSeed } from "../office/gen/prng";
import { resolveArchetype, getArchetype, customArchetypeSubject } from "../office/gen/archetypes";

export interface PortraitPromptInput {
  name: string;
  role: string;
  note: string;
  appearance?: string;
  seed: string;
  archetype?: string;
}

export function buildPortraitPrompt(input: PortraitPromptInput): string {
  const archId = resolveArchetype(input.archetype, input.seed);
  const arch = getArchetype(archId);
  const pal = arch.generatePalette(makeRng(hashStringToSeed(input.seed)));
  // 목록에 없는 종족을 적어 넣었으면 그 문구가 주제 서술자를 대신한다 —
  // 스프라이트(파츠)는 human으로 폴백해도 그림 의뢰는 적은 대로 나가야 한다.
  const custom = customArchetypeSubject(input.archetype);
  const base = arch.promptDescriptor(pal);
  const desc = custom ? { ...base, subject: custom } : base;
  const note = input.note.trim();
  const appearance = (input.appearance ?? "").trim();

  const firstLine = desc.humanoid
    ? "A bust-up character portrait in the visual style of an early-1990s Japanese PC bishoujo game (PC-98 era)."
    : "A bust-up character portrait in the visual style of an early-1990s Japanese PC game (PC-98 era).";
  const styleLine = desc.humanoid
    ? "Low-resolution pixel art, visible dithering, a limited indexed color palette, a hand-drawn anime face with a friendly smile, and soft CRT-like shading."
    : "Low-resolution pixel art, visible dithering, a limited indexed color palette, an anime style character design with a friendly, cheerful look, and soft CRT-like shading.";
  const subjectSuffix = desc.subject ? ` (${desc.subject})` : "";

  const lines = [
    firstLine,
    styleLine,
    "Bright and cute mood: cheerful pastel color grading and soft warm light.",
    "Vertical 3:4 aspect ratio, head-and-shoulders framing, the character facing the viewer.",
    desc.colorHints,
    `Character: ${input.name}, a ${input.role}${subjectSuffix}.`,
    note ? `Personality / notes: ${note}.` : "",
    appearance ? `Appearance details: ${appearance}.` : "",
    "Output a single 240x320 pixel PNG in a 3:4 portrait ratio. No text, no watermark, no border.",
  ];
  return lines.filter((l) => l.length > 0).join("\n");
}

export interface SpritePromptInput {
  name: string;
  role: string;
  /** 픽셀아트 의뢰 문구. 비면 appearance로 폴백. */
  spriteRequest?: string;
  appearance?: string;
  seed: string;
  archetype?: string;
}

/** 오피스 캐릭터 커스텀용: 단일 캐릭터 16×16 픽셀 아트 프롬프트. 시트(4프레임)가
 * 아니라 단일 이미지를 의뢰한다 — 업로드 시 크롭 에디터가 시트로 정규화한다. */
export function buildSpritePrompt(input: SpritePromptInput): string {
  const archId = resolveArchetype(input.archetype, input.seed);
  const arch = getArchetype(archId);
  const pal = arch.generatePalette(makeRng(hashStringToSeed(input.seed)));
  // 목록에 없는 종족을 적어 넣었으면 그 문구가 주제 서술자를 대신한다 —
  // 스프라이트(파츠)는 human으로 폴백해도 그림 의뢰는 적은 대로 나가야 한다.
  const custom = customArchetypeSubject(input.archetype);
  const base = arch.promptDescriptor(pal);
  const desc = custom ? { ...base, subject: custom } : base;
  const request =
    (input.spriteRequest ?? "").trim() || (input.appearance ?? "").trim();
  const styleLine = desc.humanoid
    ? "Cute chibi super-deformed proportions with a large head, big expressive sparkling eyes, and a friendly smiling expression; soft bright pastel colors, warm cheerful lighting, clean black outlines, crisp pixel grid, no anti-aliasing, plain solid background, the character centered and facing the viewer."
    : "A cute mascot-like anime style character design with big expressive eyes and a friendly, cheerful look; soft bright pastel colors, warm cheerful lighting, clean black outlines, crisp pixel grid, no anti-aliasing, plain solid background, the character centered and facing the viewer.";
  const subjectSuffix = desc.subject ? ` (${desc.subject})` : "";

  const lines = [
    "A single full-body video game character sprite in 16x16 pixel art style, in the bright and cheerful look of a 16-bit SNES-era Japanese RPG.",
    styleLine,
    desc.colorHints,
    `Character: ${input.name}, a ${input.role}${subjectSuffix}.`,
    request ? `Details: ${request}.` : "",
    "The character fills most of the frame. No text, no watermark, no border.",
  ];
  return lines.filter((l) => l.length > 0).join("\n");
}

export interface MinimiPromptInput {
  name: string;
  role: string;
  /** 미니미(소환수) 전용 의뢰 문구. 비면 "본체에 어울리는 소환수를 알아서" 문구로 자동 폴백. */
  minimiRequest?: string;
  /** 본체 픽셀아트 의뢰 문구 — 소환수가 주인을 닮게 하는 맥락으로만 쓴다. */
  spriteRequest?: string;
  appearance?: string;
  seed: string;
  archetype?: string;
}

/** 미니미 의뢰 문구가 비었을 때 자동으로 들어가는 문장(순수, 테스트 대상).
 *  "주인에게 어울리는 소환수를 알아서 디자인해 달라"는 위임 문구다. */
export function autoMinimiRequestLine(masterName: string): string {
  const who = masterName.trim() || "the master";
  return `No specific request was given: invent a small summoned familiar that suits ${who} — echo their colors, silhouette motifs and role so it reads at a glance as their own companion.`;
}

/**
 * 서브에이전트 미니미(=소환수) 단일 프레임 픽셀아트 프롬프트.
 * 본체 스프라이트와 달리 **주인의 분신/소환수**라는 관계를 명시하고, 전용
 * 의뢰 문구가 없으면 `autoMinimiRequestLine`으로 자동 위임한다.
 */
export function buildMinimiPrompt(input: MinimiPromptInput): string {
  const archId = resolveArchetype(input.archetype, input.seed);
  const arch = getArchetype(archId);
  const pal = arch.generatePalette(makeRng(hashStringToSeed(input.seed)));
  const custom = customArchetypeSubject(input.archetype);
  const base = arch.promptDescriptor(pal);
  const desc = custom ? { ...base, subject: custom } : base;
  const master = (input.spriteRequest ?? "").trim() || (input.appearance ?? "").trim();
  const request = (input.minimiRequest ?? "").trim();
  const subjectSuffix = desc.subject ? ` (${desc.subject})` : "";

  const lines = [
    "A single tiny summoned familiar creature sprite in 16x16 pixel art style, in the bright and cheerful look of a 16-bit SNES-era Japanese RPG.",
    "The familiar is a palm-sized companion that floats beside its master's head, so it must read clearly at a very small size: a simple bold silhouette, few colors, big expressive eyes, and a friendly cheerful look.",
    "Soft bright pastel colors, warm cheerful lighting, clean black outlines, crisp pixel grid, no anti-aliasing, plain solid background, the creature centered and facing the viewer.",
    desc.colorHints,
    `Master: ${input.name}, a ${input.role}${subjectSuffix}.`,
    master ? `Master's appearance: ${master}.` : "",
    request ? `Familiar: ${request}.` : autoMinimiRequestLine(input.name),
    "The creature fills most of the frame. No text, no watermark, no border.",
  ];
  return lines.filter((l) => l.length > 0).join("\n");
}

// ── codex CLI 내장 이미지 생성용(kbm #2fa) ──────────────────────────────
// 클립보드 프롬프트(buildPortraitPrompt/buildSpritePrompt)와 본문을 그대로
// 공유하고, codex가 실제로 만들 이미지 규격 한 줄만 갈아 끼운다. 최종 규격화
// (초상 240×320 / 스프라이트 4프레임 시트)는 기존 크롭 에디터가 담당하므로
// 여기서는 크롭 여유가 있는 큰 캔버스를 의뢰한다.

/** codex 초상 생성 프롬프트. 세로 1024x1536 — PortraitEditor가 3:4로 크롭한다. */
export function buildCodexPortraitPrompt(input: PortraitPromptInput): string {
  const base = buildPortraitPrompt(input)
    .split("\n")
    .slice(0, -1) // 마지막 출력 규격 줄만 교체
    .join("\n");
  return `${base}\nGenerate a single PNG image, portrait orientation, 1024x1536. No text, no watermark, no border.`;
}

/** codex 스프라이트 생성 프롬프트. 정사각 1024x1024 + 투명 배경 —
 * SpriteEditor가 크롭·배경 투명화 후 4프레임 시트로 정규화한다. */
export function buildCodexSpritePrompt(input: SpritePromptInput): string {
  const base = buildSpritePrompt(input)
    .split("\n")
    .slice(0, -1) // 마지막 "The character fills most of the frame..." 줄 교체
    .join("\n");
  return `${base}\nThe character fills most of the frame. Generate a single PNG image, square, 1024x1024, with a fully transparent background. No text, no watermark, no border.`;
}

/** codex 미니미 생성 프롬프트. 정사각 1024x1024 + 투명 배경 —
 * SpriteEditor(target="minimi")가 크롭·배경 투명화 후 단일 프레임으로 정규화한다. */
export function buildCodexMinimiPrompt(input: MinimiPromptInput): string {
  const base = buildMinimiPrompt(input)
    .split("\n")
    .slice(0, -1) // 마지막 "The creature fills most of the frame..." 줄 교체
    .join("\n");
  return `${base}\nThe creature fills most of the frame. Generate a single PNG image, square, 1024x1024, with a fully transparent background. No text, no watermark, no border.`;
}
