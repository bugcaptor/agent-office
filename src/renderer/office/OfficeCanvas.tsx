// src/renderer/office/OfficeCanvas.tsx
//
// React component hosting the office canvas — the boundary component with
// subsystem C.
//
// Frozen: no addAgent/removeAgent/setPending methods anywhere in this
// subsystem's public surface — C always re-renders with the full
// `profiles` list and this component (via `useOfficeScene`) diff-syncs.
import { useOfficeScene } from "./useOfficeScene";
import type { PixiThemePalette } from "../theme/themes";
import type { OfficeBus } from "./bus";
import type { AgentProfile } from "./types";

export function OfficeCanvas({
  bus,
  profiles,
  resyncSignal,
  pixiPalette,
}: {
  bus: OfficeBus;
  profiles: readonly AgentProfile[];
  resyncSignal?: unknown;
  /** 현재 테마의 Pixi 팔레트(theme/themes.ts). 변경 시 씬이 라이브로 재도색된다. */
  pixiPalette?: PixiThemePalette;
}) {
  // The `<canvas>` itself is created imperatively inside `useOfficeScene`'s
  // effect (one per mount, never shared across React's StrictMode dev-mode
  // double-invoke) -- see that hook's root-cause doc comment. This div is
  // just the stable container it gets appended into.
  const { containerRef } = useOfficeScene(bus, profiles, resyncSignal, pixiPalette);
  return <div ref={containerRef} style={{ position: "absolute", inset: 0, imageRendering: "pixelated" }} />;
}
