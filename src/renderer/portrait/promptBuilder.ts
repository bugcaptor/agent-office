// src/renderer/portrait/promptBuilder.ts
//
// 캐릭터 정보 -> 영문 이미지 생성 프롬프트. 순수 함수. 아키타입별 팔레트/서술자/
// 색 힌트를 주입한다. 휴머노이드는 90년대 PC-98 bishoujo 화풍, 비휴머노이드는
// 중립 "anime style character" 문구. characterFactory(pixi) 대신 archetypes/palette만 사용.
// 톤 방향(의도적 계약 변경): 밝고 귀여운 16비트 일본 RPG(SNES-era JRPG) 룩 —
// 치비 비율·파스텔·따뜻한 조명·웃는 표정. 테스트도 이 문구에 맞춰 갱신됨.
import { makeRng, hashStringToSeed } from "../office/gen/prng";
import { resolveArchetype, getArchetype } from "../office/gen/archetypes";

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
  const desc = arch.promptDescriptor(pal);
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
  const desc = arch.promptDescriptor(pal);
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

/** PixelLab create-image-pixen용 프롬프트. buildSpritePrompt와
 * 로직을 공유하되 크기("16x16")·배경("plain solid background") 문구를 뺀다 —
 * 크기는 image_size 파라미터, 배경은 no_background 파라미터가 담당한다.
 * 스타일 톤은 buildSpritePrompt와 동일한 밝고 귀여운 SNES-era JRPG 룩. */
export function buildPixelLabSpriteDescription(input: SpritePromptInput): string {
  const archId = resolveArchetype(input.archetype, input.seed);
  const arch = getArchetype(archId);
  const pal = arch.generatePalette(makeRng(hashStringToSeed(input.seed)));
  const desc = arch.promptDescriptor(pal);
  const request =
    (input.spriteRequest ?? "").trim() || (input.appearance ?? "").trim();
  const styleLine = desc.humanoid
    ? "Cute chibi super-deformed proportions with a large head, big expressive sparkling eyes, and a friendly smiling expression; soft bright pastel colors, warm cheerful lighting, clean black outlines, crisp pixel grid, no anti-aliasing, the character centered and facing the viewer."
    : "A cute mascot-like anime style character design with big expressive eyes and a friendly, cheerful look; soft bright pastel colors, warm cheerful lighting, clean black outlines, crisp pixel grid, no anti-aliasing, the character centered and facing the viewer.";
  const subjectSuffix = desc.subject ? ` (${desc.subject})` : "";

  const lines = [
    "A single full-body video game character sprite in pixel art style, in the bright and cheerful look of a 16-bit SNES-era Japanese RPG.",
    styleLine,
    desc.colorHints,
    `Character: ${input.name}, a ${input.role}${subjectSuffix}.`,
    request ? `Details: ${request}.` : "",
    "The character fills most of the frame. No text, no watermark, no border.",
  ];
  return lines.filter((l) => l.length > 0).join("\n");
}
