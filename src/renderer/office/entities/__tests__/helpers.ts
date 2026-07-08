// src/renderer/office/entities/__tests__/helpers.ts
//
// Test-only `CharacterAssets` factory for `CharacterEntity` tests.
//
// `createCharacterAssets` (gen/characterFactory.ts) needs `Texture.from()`
// on a real `<canvas>`, which — per 3C's test suite — needs a renderer
// context this vitest environment doesn't provide. `CharacterEntity` itself
// only ever reads `.idle`/`.walk` as opaque `Texture[]` and swaps between
// them by reference, so distinct-but-real `Texture` instances built on top
// of `BufferImageSource` (pure JS, no canvas/WebGL) are enough to exercise
// the frame-swap logic and stay reference-distinguishable in assertions.
import { BufferImageSource, Texture } from "pixi.js";
import type { CharacterAssets } from "../../gen/characterFactory";

const solidTexture = (label: string): Texture =>
  new Texture({
    source: new BufferImageSource({
      resource: new Uint8Array([255, 255, 255, 255]),
      width: 1,
      height: 1,
      label,
    }),
    label,
  });

export function makeTestCharacterAssets(): CharacterAssets {
  const idle0 = solidTexture("idle0");
  const idle1 = solidTexture("idle1");
  const walk0 = solidTexture("walk0");
  const walk1 = solidTexture("walk1");
  return {
    base: idle0,
    frames: { idle0, idle1, walk0, walk1 },
    idle: [idle0, idle1],
    walk: [walk0, walk1],
    cellSize: 16,
    descriptor: { archetype: "test", hair: "test", clothes: "test", accessory: "test" },
  };
}
