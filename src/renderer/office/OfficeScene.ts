// src/renderer/office/OfficeScene.ts
//
// Pixi Application wrapper: init/resize/destroy + the fixed, integer-scale
// camera.
//
// Scope note (frozen): `OfficeWorld` owns entity diff-sync/behavior
// FSM/click hit-testing; this class only owns the Pixi Application
// lifecycle, static map render, camera, and the ticker->world.update() wire.
// The scene deliberately has no addAgent/removeAgent/setPending methods —
// `syncAgents` is the single entry point C ever needs.
//
// Dispose-safety note: `init()` is
// async, but React (StrictMode double-mount in particular) can invoke the
// effect cleanup — and thus `destroy()` — before `app.init()` has resolved.
// Pixi's `Application.renderer` is only assigned once `init()` completes, so
// calling `app.destroy()` beforehand throws (`this.renderer` is undefined).
// `destroy()` therefore guards on `started` and is a safe no-op pre-init;
// `useOfficeScene`'s `disposed` flag + init-`.then` callback performs the
// real teardown once init later resolves. This keeps `destroy()` idempotent
// and leak-free regardless of when it's called. `world.destroy()` runs
// unconditionally (before the `started` guard) since it only unsubscribes
// from `bus` and tears down entities — neither depends on the Pixi app being
// initialized, and it must not leak listeners on the pre-init destroy path.
import { Application, Container, Rectangle, type FederatedPointerEvent, type Ticker } from "pixi.js";
import { TileRenderer } from "./map/TileRenderer";
import { OFFICE_MAP, TILE_SIZE } from "./map/mapData";
import { OfficeWorld } from "./world/OfficeWorld";
import { THEMES } from "../theme/themes";
import type { PixiThemePalette } from "../theme/themes";
import type { LabelAnchor, OfficeBus } from "./bus";
import type { AgentProfile } from "./types";

export interface OfficeSceneOptions {
  canvas: HTMLCanvasElement;
  bus: OfficeBus; // handed straight to this scene's `OfficeWorld` (3H)
  /** 테마 팔레트(타일 색 + 배경색). 기본은 테마 도입 이전 룩(midnight). */
  palette?: PixiThemePalette;
}

export class OfficeScene {
  private app: Application;
  private worldContainer: Container; // camera transform target
  private sortableLayer: Container; // furniture + characters (zIndex = worldY)
  private floorLayer: Container;
  private overlayLayer: Container;
  private ro?: ResizeObserver;
  private onWake?: () => void;
  private opts: OfficeSceneOptions;
  private started = false;
  private world: OfficeWorld;
  private tickerCallback?: (ticker: Ticker) => void;
  private labelAnchorsWorld = new Map<string, LabelAnchor>();
  private labelAnchorsScreen = new Map<string, LabelAnchor>();
  // 현재 테마 팔레트 + 테마 전환 시 파기/재구축해야 하는 타일 표시 객체 추적.
  // (캐릭터 엔티티는 sortableLayer를 공유하므로 레이어 통째 removeChildren은 불가.)
  private palette: PixiThemePalette;
  private floorTiles?: Container;
  private furnitureTiles: Container[] = [];

  constructor(opts: OfficeSceneOptions) {
    this.opts = opts;
    this.palette = opts.palette ?? THEMES.midnight.pixi;
    this.app = new Application();
    this.worldContainer = new Container();
    this.floorLayer = new Container();
    this.sortableLayer = new Container();
    this.overlayLayer = new Container();
    this.sortableLayer.sortableChildren = true;
    // Constructed eagerly (doesn't need the Pixi renderer): `characterLayer`
    // is just a plain Container, and bus subscriptions are pure JS. Actual
    // entity creation only ever happens via `syncAgents`, which stays
    // guarded on `started` below.
    this.world = new OfficeWorld({
      bus: this.opts.bus,
      characterLayer: this.sortableLayer,
      overlayLayer: this.overlayLayer,
      map: OFFICE_MAP,
    });
  }

  /** Async init. Awaited from the React hook. */
  async init(): Promise<void> {
    await this.app.init({
      canvas: this.opts.canvas,
      background: this.palette.background, // init 전 setTheme()가 왔어도 최신 팔레트가 반영된다
      antialias: false, // pixel art: no AA
      roundPixels: true, // avoid subpixel rendering
      resolution: 1, // sharpness comes from the integer-scale camera, not DPR
      autoDensity: false,
      resizeTo: this.opts.canvas.parentElement ?? undefined,
    });

    this.worldContainer.addChild(this.floorLayer, this.sortableLayer, this.overlayLayer);
    this.app.stage.addChild(this.worldContainer);

    // Static map render.
    this.buildMapLayers();
    this.buildDeskHitAreas();

    this.applyCamera();
    this.started = true;

    // Drive entity FSM/movement/animation from Pixi's own frame clock —
    // never a real timer/Date.now (keeps `CharacterEntity.update` testable
    // and consistent with the rest of this subsystem's determinism rules).
    this.tickerCallback = (ticker) => {
      this.world.update(ticker.deltaMS);
      this.publishLabelAnchors();
    };
    this.app.ticker.add(this.tickerCallback);

    // Parent resize -> recompute camera (Pixi's resizeTo already matches canvas pixels).
    const parent = this.opts.canvas.parentElement;
    if (parent) {
      this.ro = new ResizeObserver(() => this.resize());
      this.ro.observe(parent);
    }

    // Wake (tab refocused / window restored) -> recompute camera. A minimized
    // window or hidden tab can report a 0-size parent while backgrounded;
    // `applyCamera()` bails out on that (see below), so the camera can go
    // stale until something explicitly recomputes it once real dimensions
    // are back. Skip while still hidden (`document.hidden`) since the
    // measurement would still be 0.
    this.onWake = () => {
      if (!document.hidden) this.resize();
    };
    document.addEventListener("visibilitychange", this.onWake);
    window.addEventListener("focus", this.onWake);
  }

  resize(): void {
    if (!this.started) return;
    this.applyCamera();
  }

  /** 현재 팔레트로 정적 바닥/벽 레이어 + 가구를 (재)구축한다. */
  private buildMapLayers(): void {
    const tiles = new TileRenderer(OFFICE_MAP, TILE_SIZE, this.palette);
    this.floorTiles = tiles.build();
    this.floorLayer.addChild(this.floorTiles);
    this.furnitureTiles = tiles.buildFurniture();
    this.sortableLayer.addChild(...this.furnitureTiles);
  }

  /**
   * 데스크 슬롯마다 데스크 쌍(2x1 타일)을 덮는 보이지 않는 히트영역을
   * floorLayer(최하단)에 만든다 — 캐릭터·가구보다 아래라서 캐릭터 클릭이
   * 항상 우선하고, 빈 책상 클릭만 여기로 떨어진다. 테마 전환과 무관하게
   * 한 번만 생성(색이 없으므로 재베이크 불필요).
   */
  private buildDeskHitAreas(): void {
    for (const desk of OFFICE_MAP.desks) {
      const hit = new Container();
      // 좌석은 데스크 상판 바로 위 타일 — 상판 행은 seat.ty + 1.
      hit.position.set(desk.seat.tx * TILE_SIZE, (desk.seat.ty + 1) * TILE_SIZE);
      hit.eventMode = "static";
      hit.cursor = "pointer";
      hit.hitArea = new Rectangle(0, 0, TILE_SIZE * 2, TILE_SIZE);
      hit.on("pointertap", (e: FederatedPointerEvent) =>
        this.opts.bus.emitDeskClicked(desk.index, e.global.x, e.global.y),
      );
      this.floorLayer.addChild(hit);
    }
  }

  /**
   * 테마 전환: 배경색을 라이브로 갱신하고, `build()`가 한 장으로 베이크해 둔
   * 타일 텍스처를 파기 후 새 팔레트로 재베이크한다. 캐릭터 엔티티는
   * sortableLayer에 그대로 남는다(가구 Graphics만 교체).
   * init() 전에 불리면 팔레트만 바꿔 둔다 — init()이 그 값을 사용한다.
   */
  setTheme(palette: PixiThemePalette): void {
    this.palette = palette;
    if (!this.started) return;
    this.app.renderer.background.color = palette.background;
    if (this.floorTiles) {
      this.floorLayer.removeChild(this.floorTiles);
      this.floorTiles.cacheAsTexture(false); // 베이크된 캐시 텍스처 명시 해제(GPU 릭 방지)
      this.floorTiles.destroy({ children: true });
      this.floorTiles = undefined;
    }
    for (const g of this.furnitureTiles) {
      this.sortableLayer.removeChild(g);
      g.destroy();
    }
    this.furnitureTiles = [];
    this.buildMapLayers();
  }

  private applyCamera(): void {
    // Measure the canvas's parent directly rather than trusting
    // `app.renderer.width/height`: this method can run from our own
    // ResizeObserver callback, which can fire before Pixi's own `resizeTo`
    // observer has applied the new size to the renderer — and if the parent
    // is momentarily unmeasurable (minimized window, hidden tab -> 0x0), the
    // renderer's stale/negative-going size would otherwise park the camera
    // at a garbage offset that never gets corrected.
    const parent = this.opts.canvas.parentElement;
    if (!parent) return;
    const w = parent.clientWidth;
    const h = parent.clientHeight;
    if (w <= 0 || h <= 0) return; // unmeasurable right now; leave the camera as-is rather than going negative

    // Keep Pixi's renderer in sync in case our observer got here first.
    const view = this.app.renderer;
    if (view.width !== w || view.height !== h) {
      this.app.renderer.resize(w, h);
    }

    const mapPxW = OFFICE_MAP.width * TILE_SIZE;
    const mapPxH = OFFICE_MAP.height * TILE_SIZE;
    const scale = computeIntegerScale(w, h, mapPxW, mapPxH);
    this.worldContainer.scale.set(scale);
    // Center, snapped to integer position to preserve sharpness.
    this.worldContainer.position.set(
      Math.floor((w - mapPxW * scale) / 2),
      Math.floor((h - mapPxH * scale) / 2),
    );
  }

  /**
   * Diff-syncs the live entity set against `profiles` via `OfficeWorld`.
   * Guarded on `started` (preserved from the 3E skeleton): `useOfficeScene`'s
   * `[profiles]`-keyed effect calls this unconditionally on every render —
   * including the very first one, before `init()` has resolved. `useOfficeScene`
   * itself performs the actual initial sync from its `init().then(...)`
   * callback once `started` flips true, so dropping pre-init calls here is
   * safe — but ONLY because that post-init sync reads the hook's
   * `profilesRef` (latest render's profiles), not a mount-time closure
   * capture. With a stale capture, a hydrate that lands mid-init would be
   * dropped here and never replayed → 간헐적 "캐릭터 전원 미표시" 버그.
   */
  syncAgents(profiles: readonly AgentProfile[]): void {
    if (!this.started) return; // init() hasn't finished; nothing to sync into yet
    this.world.syncAgents(profiles);
  }

  /** 캐릭터 머리 위 월드좌표를 화면좌표로 투영해 bus로 발행한다(매 tick).
   * Map 두 개를 재사용해 per-frame 할당을 상수로 유지한다. */
  private publishLabelAnchors(): void {
    this.world.collectLabelAnchors(this.labelAnchorsWorld);
    this.labelAnchorsScreen.clear();
    const scale = this.worldContainer.scale.x;
    const ox = this.worldContainer.position.x;
    const oy = this.worldContainer.position.y;
    for (const [id, p] of this.labelAnchorsWorld) {
      this.labelAnchorsScreen.set(id, worldToScreen(p.x, p.y, scale, ox, oy));
    }
    this.opts.bus.emitLabelAnchorsChanged(this.labelAnchorsScreen);
  }

  destroy(): void {
    this.ro?.disconnect();
    this.ro = undefined;
    if (this.onWake) {
      document.removeEventListener("visibilitychange", this.onWake);
      window.removeEventListener("focus", this.onWake);
      this.onWake = undefined;
    }
    this.world.destroy(); // unconditional: only unsubscribes bus + destroys entities, no Pixi dependency
    if (!this.started) return; // init() never completed -> nothing else to tear down yet
    this.started = false;
    if (this.tickerCallback) this.app.ticker.remove(this.tickerCallback);
    this.app.destroy(true, { children: true, texture: true }); // release GPU resources
  }
}

/** 월드좌표 → 화면좌표(캔버스 px): 카메라 정수 스케일 + 센터링 오프셋. */
export function worldToScreen(
  wx: number,
  wy: number,
  scale: number,
  offsetX: number,
  offsetY: number
): { x: number; y: number } {
  return { x: offsetX + wx * scale, y: offsetY + wy * scale };
}

/** Largest integer scale that fits the map into the viewport. Minimum 1. */
export function computeIntegerScale(viewW: number, viewH: number, mapPxW: number, mapPxH: number): number {
  const s = Math.floor(Math.min(viewW / mapPxW, viewH / mapPxH));
  return Math.max(1, s);
}
