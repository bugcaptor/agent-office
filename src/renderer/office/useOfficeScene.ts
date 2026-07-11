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
  // profiles도 같은 이유로 ref가 필요하다 — 간헐적 "빈 사무실" 레이스의
  // 근본 원인: 실제 부팅에서는 App이 agents=[]로 먼저 렌더되고(bootApp의
  // loadState는 비동기), Pixi `init()`이 끝나기 전에 hydrate가 profiles를
  // 채울 수 있다. 그 사이 아래 `[profiles]` 이펙트의 syncAgents 호출은
  // `OfficeScene.started` 가드에 걸려 드롭되므로, `init().then()`의 초기
  // 동기화가 마운트 시점 클로저에 캡처된 빈 배열을 쓰면 캐릭터가 하나도
  // 안 그려진 채 고정된다. 항상 최신 profiles로 초기 동기화해야 한다.
  const profilesRef = useRef(profiles);
  profilesRef.current = profiles;

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
      scene.syncAgents(profilesRef.current); // initial sync — 최신값(마운트 시점 캡처 아님)
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
