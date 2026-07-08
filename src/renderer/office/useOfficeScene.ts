// src/renderer/office/useOfficeScene.ts
//
// React integration hook: owns the canvas container ref, the OfficeScene
// lifecycle, and profile-list sync.
//
// Deliberately thin: a re-render must never recreate the Pixi scene, so
// `OfficeScene` is kept in a ref, and the mount effect has an empty
// dependency array. The `disposed` flag guards against the async `init()`
// resolving after the effect has already been cleaned up (this is exactly
// what happens on React StrictMode's mount -> cleanup -> re-mount dev-mode
// double-invoke) — see `OfficeScene.destroy()`'s doc comment for why
// `destroy()` itself must also be idempotent/pre-init-safe for this to be
// leak-free.
//
// Root-cause note (black main view): the mount effect creates its
// own `<canvas>` via `document.createElement` and appends it into the
// container on every invocation, rather than binding a single
// React-JSX-rendered `<canvas ref>` that would survive across StrictMode's
// synchronous mount -> cleanup -> re-mount. A `<canvas>` element can only
// ever back one WebGL rendering context; if both the abandoned and the
// surviving `OfficeScene` were constructed against the *same* canvas node,
// the abandoned scene's deferred `destroy()` (which runs `Application`
// `.destroy(true, ...)` once its `init()` resolves, even post-abandonment)
// would tear the shared context out from under the still-live scene —
// confirmed via headless-Chrome CDP: the office `<canvas>` was completely
// absent from the committed DOM and Pixi logged "Could not retrieve shader
// source (WebGL context may be lost)". Giving every effect invocation a
// private canvas makes the two `OfficeScene`/`Application` instances fully
// independent, so the abandoned one's teardown can never affect the
// surviving one.
import { useEffect, useRef } from "react";
import { OfficeScene } from "./OfficeScene";
import type { PixiThemePalette } from "../theme/themes";
import type { OfficeBus } from "./bus";
import type { AgentProfile } from "./types";

export function useOfficeScene(
  bus: OfficeBus,
  profiles: readonly AgentProfile[],
  resyncSignal?: unknown,
  pixiPalette?: PixiThemePalette,
) {
  const containerRef = useRef<HTMLDivElement | null>(null);
  const sceneRef = useRef<OfficeScene | null>(null);
  // 마운트 효과는 deps가 비어 있어 최초 렌더의 값을 캡처한다 — 팔레트는 ref로
  // 최신값을 넘겨 마운트 시점의 현재 테마로 씬을 생성한다.
  const paletteRef = useRef(pixiPalette);
  paletteRef.current = pixiPalette;

  // Mount: create the scene exactly once (with StrictMode double-mount defense).
  useEffect(() => {
    const container = containerRef.current;
    if (!container) return;
    const canvas = document.createElement("canvas");
    canvas.style.display = "block";
    canvas.style.width = "100%";
    canvas.style.height = "100%";
    canvas.style.borderRadius = "0";
    container.appendChild(canvas);

    let disposed = false;
    const scene = new OfficeScene({ canvas, bus, palette: paletteRef.current });
    sceneRef.current = scene;
    scene.init().then(() => {
      if (disposed) {
        scene.destroy();
        return;
      }
      scene.syncAgents(profiles); // initial sync
    });
    return () => {
      disposed = true;
      sceneRef.current = null;
      scene.destroy();
      canvas.remove(); // this scene's own canvas only -- never shared with another instance
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []); // bus/profiles changes are handled by the effect below

  // Profile list changes -> reflect on the scene (no Pixi recreation).
  // `resyncSignal`: 프로필 객체는 그대로여도 외형 입력(커스텀 시트 디코드 완료
  // 등)이 바뀌었을 때 diff-sync를 다시 돌리기 위한 추가 신호.
  useEffect(() => {
    sceneRef.current?.syncAgents(profiles);
  }, [profiles, resyncSignal]);

  // 테마 팔레트 변경 -> 씬 배경 갱신 + 타일 텍스처 재베이크(Pixi 재생성 없음).
  // `OfficeScene.setTheme`는 init 전에도 안전(팔레트만 저장 → init이 사용).
  useEffect(() => {
    if (pixiPalette) sceneRef.current?.setTheme(pixiPalette);
  }, [pixiPalette]);

  return { containerRef };
}
