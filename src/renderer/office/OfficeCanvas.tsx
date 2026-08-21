// src/renderer/office/OfficeCanvas.tsx
//
// React component hosting the office canvas — the boundary component with
// subsystem C.
//
// Frozen: no addAgent/removeAgent/setPending methods anywhere in this
// subsystem's public surface — C always re-renders with the full
// `profiles` list and this component (via `useOfficeScene`) diff-syncs.
import { useOfficeScene } from "./useOfficeScene";
import type { ThemeDef } from "../theme/themes";
import type { SceneDef } from "./scenes/sceneTypes";
import type { OfficeBus } from "./bus";
import type { AgentProfile } from "./types";

export function OfficeCanvas({
  bus,
  profiles,
  resyncSignal,
  theme,
  scene,
}: {
  bus: OfficeBus;
  profiles: readonly AgentProfile[];
  resyncSignal?: unknown;
  /** 현재 테마(theme/themes.ts). 변경 시 씬이 라이브로 재도색된다. */
  theme?: ThemeDef;
  /** 현재 풍경(office/scenes). 변경 시 맵·타일·히트영역이 통째로 재구축된다. */
  scene?: SceneDef;
}) {
  // The `<canvas>` itself is created imperatively inside `useOfficeScene`'s
  // effect (one per mount, never shared across React's StrictMode dev-mode
  // double-invoke) -- see that hook's root-cause doc comment. This div is
  // just the stable container it gets appended into.
  const { containerRef } = useOfficeScene(bus, profiles, resyncSignal, theme, scene);
  return <div ref={containerRef} style={{ position: "absolute", inset: 0, imageRendering: "pixelated" }} />;
}
