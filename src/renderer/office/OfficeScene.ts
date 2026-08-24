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
import { Application, Container, Graphics, Rectangle, Text, type FederatedPointerEvent, type Texture, type Ticker } from "pixi.js";
import { TileRenderer } from "./map/TileRenderer";
import { BOSS_DESK_RECT, TILE_SIZE, type OfficeMap, type TileRect } from "./map/mapData";
import { tileCenterPx } from "./world/pathing";
import { OfficeWorld } from "./world/OfficeWorld";
import { THEMES } from "../theme/themes";
import type { ThemeDef } from "../theme/themes";
import { SCENES } from "./scenes/scenes";
import { awardFrameRectPx } from "./scenes/officeScene";
import type { SceneDef, SceneRender } from "./scenes/sceneTypes";
import type { LabelAnchor, OfficeAwardee, OfficeBus } from "./bus";
import type { AgentProfile } from "./types";
import { awardeeEquals, resolveAwardeeSeat, shouldShowAwardFrame } from "./awardee";
import { TrophyOverlay } from "./entities/TrophyOverlay";
import { AwardFrameOverlay } from "./entities/AwardFrameOverlay";
import { loadAwardPortraitTexture } from "./gen/awardPortraitTexture";
import { tauriApi } from "../ipc/tauriApi";

export interface OfficeSceneOptions {
  canvas: HTMLCanvasElement;
  bus: OfficeBus; // handed straight to this scene's `OfficeWorld` (3H)
  /** 테마(색 축). 기본은 테마 도입 이전 룩(midnight). */
  theme?: ThemeDef;
  /** 풍경(scene 축) — 맵 + 타일 드로잉. 기본은 사무실. */
  scene?: SceneDef;
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
  // 현재 테마·풍경 + 전환 시 파기/재구축해야 하는 표시 객체 추적.
  // (캐릭터 엔티티는 sortableLayer를 공유하므로 레이어 통째 removeChildren은 불가.)
  private theme: ThemeDef;
  private scene: SceneDef;
  /** 현재 씬×테마로 확정된 렌더 바인딩(배경색 + 타일 드로잉). */
  private render: SceneRender;
  private floorTiles?: Container;
  private furnitureTiles: Container[] = [];
  private deskHits: Container[] = [];
  private bossHit?: Container;
  private bossSign?: Container;
  private bossSignBoard?: Graphics;
  private bossSignLabel?: Text;
  private offVacation?: () => void;
  private offHoverGate?: () => void;
  /** 휴가 모드 최신값 — 씬 재구축 후 팻말 표시 상태를 되살리기 위해 보관. */
  private vacationOn = false;
  // "이 달의 우수사원" 오피스 연출(docs/employee-of-the-month-design.md §7).
  private trophyOverlay?: TrophyOverlay;
  private awardFrame?: AwardFrameOverlay;
  private offAwardee?: () => void;
  /** 확정 수상자 최신값 — 씬 재구축 후 표시 상태를 되살리기 위해 보관(bus는 값이
   * 바뀔 때만 발화하므로 재구축 직후엔 replay가 안 온다 — vacationOn과 동일 이유). */
  private awardee: OfficeAwardee | null = null;
  /** 트로피 좌석 계산에 쓰는 최신 로스터 — syncAgents가 채운다. */
  private profiles: readonly AgentProfile[] = [];
  private awardPortraitReq = 0;
  private awardPortraitTexture?: Texture;
  /** awardPortraitTexture가 어느 달의 것인지 — 같은 달이면 재구축 시 IPC를 다시 타지 않는다. */
  private awardPortraitFor?: string;

  constructor(opts: OfficeSceneOptions) {
    this.opts = opts;
    this.theme = opts.theme ?? THEMES.midnight;
    this.scene = opts.scene ?? SCENES.office;
    this.render = this.scene.resolve(this.theme);
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
      map: this.scene.map,
    });
  }

  /** 현재 풍경의 타일 맵 — 좌석/보스 책상/줄 슬롯/라운지의 단일 출처. */
  private get map(): OfficeMap {
    return this.scene.map;
  }

  /** 현재 맵의 보스 책상 사각형(맵이 안 들고 있으면 오피스 기본값). */
  private get bossDeskRect(): TileRect {
    return this.map.bossDesk ?? BOSS_DESK_RECT;
  }

  /** Async init. Awaited from the React hook. */
  async init(): Promise<void> {
    await this.app.init({
      canvas: this.opts.canvas,
      background: this.render.background, // init 전 setTheme()/setScene()가 왔어도 최신 값이 반영된다
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
    this.buildBossDesk();
    this.buildAwardDisplays();

    this.applyCamera();
    this.started = true;

    // 오버레이(터미널 창 등)가 캔버스를 덮고 있을 때 뒤의 캐릭터가 hover되는
    // 것을 차단. 유닛 테스트의 FakeApplication renderer에는 events가 없어
    // 옵셔널 체이닝으로 건너뛴다.
    const events = this.app.renderer.events;
    if (events?.features) {
      this.offHoverGate = bindHoverGate(events.features, this.opts.canvas);
    }

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

  /** 현재 씬×테마 드로잉으로 정적 바닥/벽 레이어 + 가구를 (재)구축한다.
   *
   * `addChildAt(…, 0)`으로 floorLayer 맨 아래에 꽂는다(append가 아니라) —
   * 액자 오버레이(`buildAwardDisplays`)도 이제 floorLayer에 있고, `repaint()`는
   * 데스크/보스 히트영역·액자를 그대로 둔 채 이 메서드만 다시 불러 floorTiles를
   * 교체한다. append였다면 새 floorTiles가 기존 액자보다 나중에 추가돼 그 위를
   * 덮어버린다(벽 타일에 액자가 가려짐) — 맨 아래 고정이면 순서와 무관하게
   * 항상 바닥에 깔린다.
   */
  private buildMapLayers(): void {
    const tiles = new TileRenderer(this.map, TILE_SIZE, this.render.drawTile);
    this.floorTiles = tiles.build();
    this.floorLayer.addChildAt(this.floorTiles, 0);
    this.furnitureTiles = tiles.buildFurniture();
    this.sortableLayer.addChild(...this.furnitureTiles);
  }

  /**
   * 데스크 슬롯마다 데스크 쌍(2x1 타일)을 덮는 보이지 않는 히트영역을
   * floorLayer(최하단)에 만든다 — 캐릭터·가구보다 아래라서 캐릭터 클릭이
   * 항상 우선하고, 빈 책상 클릭만 여기로 떨어진다. 테마 전환과는 무관하고
   * (색이 없다), 풍경 전환에서는 좌석 좌표가 바뀌므로 재구축한다.
   */
  private buildDeskHitAreas(): void {
    for (const desk of this.map.desks) {
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
      this.deskHits.push(hit);
    }
  }

  /** 보스 책상: 클릭 히트영역(휴가 토글) + "휴가중" 표지판(휴가 모드일 때만 표시). */
  private buildBossDesk(): void {
    const rect = this.bossDeskRect;
    const hit = new Container();
    hit.position.set(rect.x * TILE_SIZE, rect.y * TILE_SIZE);
    hit.eventMode = "static";
    hit.cursor = "pointer";
    hit.hitArea = new Rectangle(0, 0, TILE_SIZE * rect.w, TILE_SIZE * rect.h);
    hit.on("pointertap", () => this.opts.bus.emitBossDeskClicked());
    this.floorLayer.addChild(hit); // 데스크 히트영역과 동일 레이어(캐릭터 클릭 우선)
    this.bossHit = hit;

    // 책상 위 텐트 카드(/휴가중/\): 앞면 평행사변형 + 능선을 공유하는 뒤판 삼각형.
    const sign = new Container();
    const board = new Graphics();
    board.position.x = -2.25; // 뒤판 포함 전체 폭(-7~11.5)의 중심을 앵커(책상 중앙)에 정렬
    sign.addChild(board);
    this.bossSignBoard = board;
    // 글씨는 월드 배율에서 비정수 리샘플링으로 깨져, applyCamera가 1/scale로 상쇄한다.
    const label = new Text({
      text: "휴가중",
      style: { fontFamily: "DungGeunMo", fontSize: 11, fill: this.theme.pixi.text },
      resolution: 2,
    });
    label.anchor.set(0.5, 0.5);
    label.position.set(1 - 2.25, -3.5);
    sign.addChild(label);
    this.bossSignLabel = label;
    this.paintBossSign();
    const p = tileCenterPx({ tx: rect.x, ty: rect.y + rect.h - 1 });
    sign.position.set(p.x, p.y);
    // 풍경을 바꿔도 휴가 팻말이 사라지지 않도록 최신 휴가 상태로 복원한다
    // (bus는 값이 *바뀔 때만* 발화하므로 재구축 직후엔 아무 이벤트도 안 온다).
    sign.visible = this.vacationOn;
    // 글씨는 월드 배율을 1/scale로 상쇄한다(applyCamera와 같은 규칙) — 재구축
    // 직후 카메라가 다시 측정되지 못하는 경우에도 배율이 어긋나지 않게 여기서 건다.
    label.scale.set(1 / (this.worldContainer.scale.x || 1));
    this.overlayLayer.addChild(sign);
    this.bossSign = sign;

    this.offVacation = this.opts.bus.onVacationModeChanged((on) => {
      this.vacationOn = on;
      if (this.bossSign) this.bossSign.visible = on;
    });
  }

  /**
   * "이 달의 우수사원" 표시객체(책상 트로피 + 벽 액자)를 만들고 bus를
   * 구독한다. 좌표/가시성은 최신 awardee + profiles로 결정되므로, 구독 전에
   * 한 번 `updateAwardDisplays()`로 최신 상태를 먼저 복원한다 — 휴가 팻말과
   * 같은 이유(bus replay는 subscribe 시점에만 오고, 값이 안 바뀌었으면
   * 재구축 후에도 다시 안 온다).
   */
  private buildAwardDisplays(): void {
    const trophy = new TrophyOverlay(this.theme.pixi);
    trophy.setVisible(false);
    this.sortableLayer.addChild(trophy.root);
    this.trophyOverlay = trophy;

    const rect = awardFrameRectPx();
    const frame = new AwardFrameOverlay(this.theme.pixi, rect);
    frame.root.position.set(rect.x, rect.y);
    frame.setVisible(false);
    // floorLayer에 둔다(overlayLayer가 아니라) — 벽 타일이 이미 floorLayer에
    // 있어 그 위에 자연히 겹치고, sortableLayer의 캐릭터·가구는 전부 이
    // 레이어보다 위라 "벽에 걸린 그림 앞을 사람이 지나간다"가 z-order 계산
    // 없이 그냥 성립한다.
    this.floorLayer.addChild(frame.root);
    this.awardFrame = frame;

    this.updateAwardDisplays();
    this.offAwardee = this.opts.bus.onAwardeeChanged((awardee) => {
      if (awardeeEquals(this.awardee, awardee)) return; // 같은 값 재발화는 무시 — 재조회/재도색 없음
      this.awardee = awardee;
      this.updateAwardDisplays();
    });
  }

  /** 트로피 좌석 + 액자 표시 여부를 최신 awardee/profiles로 재계산한다. */
  private updateAwardDisplays(): void {
    this.updateTrophy();
    this.updateAwardFrame();
  }

  /** 수상자 좌석(책상) 위치로 트로피를 옮긴다. 좌석이 없으면(프로필 삭제/책상
   * 부족) 숨긴다. 책상 쌍의 오른쪽 타일(랩탑이 없는 쪽 — officeTileDraw의
   * DeskTop 케이스는 왼쪽 타일에만 랩탑을 그린다) 위에 얹는다. */
  private updateTrophy(): void {
    if (!this.trophyOverlay) return;
    const seat = resolveAwardeeSeat(this.map, this.awardee?.agentId ?? null, this.profiles);
    if (!seat) {
      this.trophyOverlay.setVisible(false);
      return;
    }
    const p = tileCenterPx({ tx: seat.tx + 1, ty: seat.ty + 1 });
    this.trophyOverlay.root.position.set(p.x, p.y - 2); // 책상 상판 위에 살짝 얹힌 높이
    this.trophyOverlay.root.zIndex = this.trophyOverlay.root.y; // 캐릭터/가구와 동일한 y-sort 규칙
    this.trophyOverlay.setVisible(true);
  }

  /** 액자(틀+매트+콘텐츠 전체) 표시 여부. `AwardFrameOverlay`가 틀까지 통째로
   * 그리므로 `root.visible` 하나로 전체를 켜고 끈다 — 수상자가 없으면 틀도
   * 벽에 남기지 않는다. 현재는 오피스 씬에만 액자 아트가 있어 다른 풍경에서는
   * 항상 숨긴다. */
  private updateAwardFrame(): void {
    if (!this.awardFrame) return;
    const show = this.scene.id === "office" && shouldShowAwardFrame(this.awardee);
    this.awardFrame.setVisible(show);
    if (!show) return;
    if (this.awardPortraitTexture && this.awardPortraitFor === this.awardee?.month) {
      this.awardFrame.showPhoto(this.awardPortraitTexture); // 캐시 재적용 — 재구축돼도 IPC 재요청 없음
    } else {
      this.awardFrame.showSilhouette();
      this.loadAwardPortrait();
    }
  }

  /** 확정 수상자의 초상을 비동기 로드해 액자에 반영한다. 초상이 없거나 로드에
   * 실패하면 실루엣 폴백을 유지한다. 그 사이 수상자가 바뀌면(reqId/month 불일치)
   * 도착한 텍스처를 버린다(stale 반영 방지). */
  private loadAwardPortrait(): void {
    const awardee = this.awardee;
    if (!awardee) return;
    this.disposePortraitTexture();
    if (!awardee.hasPortrait) return;
    const reqId = ++this.awardPortraitReq;
    void tauriApi
      .loadAwardPortrait(awardee.month)
      .then((b64) => (b64 ? loadAwardPortraitTexture(b64) : null))
      .then((texture) => {
        if (reqId !== this.awardPortraitReq || this.awardee?.month !== awardee.month) {
          texture?.destroy(true); // stale — 이 사이 수상자가 바뀌었다
          return;
        }
        if (!texture) return; // 디코드 실패 — 실루엣 유지
        this.awardPortraitTexture = texture;
        this.awardPortraitFor = awardee.month;
        this.awardFrame?.showPhoto(texture);
      })
      .catch((err) => console.warn("OfficeScene: 수상자 초상 로드 실패", err));
  }

  private disposePortraitTexture(): void {
    this.awardPortraitTexture?.destroy(true);
    this.awardPortraitTexture = undefined;
    this.awardPortraitFor = undefined;
  }

  /**
   * 테마 전환: 배경색을 라이브로 갱신하고, `build()`가 한 장으로 베이크해 둔
   * 타일 텍스처를 파기 후 새 색으로 재베이크한다. 맵(지오메트리)은 그대로라
   * 히트영역·보스 책상·월드는 손대지 않는다. 캐릭터 엔티티도 sortableLayer에
   * 그대로 남는다(가구 Graphics만 교체).
   * init() 전에 불리면 값만 바꿔 둔다 — init()이 그 값을 사용한다.
   */
  setTheme(theme: ThemeDef): void {
    this.theme = theme;
    this.render = this.scene.resolve(theme);
    if (!this.started) return;
    this.repaint();
  }

  /**
   * 풍경 전환: 색뿐 아니라 **지오메트리 전체**가 바뀐다. 재도색(repaint)에
   * 더해 좌석 히트영역·보스 책상(히트영역+팻말)·월드 맵을 다시 만들고,
   * 카메라와 라벨 앵커를 새 맵 기준으로 갱신한다.
   * init() 전에 불리면 값만 바꿔 둔다 — init()이 그 씬으로 처음부터 짓는다.
   */
  setScene(scene: SceneDef): void {
    if (scene === this.scene) return;
    this.scene = scene;
    this.render = scene.resolve(this.theme);
    // 월드는 Pixi 렌더러에 의존하지 않으므로 init 전에도 안전하게 전파한다
    // (init 전이면 엔티티가 없어 사실상 맵 참조 교체뿐).
    this.world.setMap(scene.map);
    if (!this.started) return;
    // 파기 → 재도색 → 재구축 순서. 먼저 파기해야 repaint가 곧 버릴 팻말을
    // 헛되이 칠하지 않는다(paintBossSign은 팻말이 없으면 no-op).
    this.teardownHitAreas();
    this.repaint();
    this.buildDeskHitAreas();
    this.buildBossDesk();
    this.buildAwardDisplays();
    this.applyCamera(); // 맵 크기가 달라질 수 있고, 팻말 글씨 배율도 여기서 다시 건다
    this.publishLabelAnchors(); // 순간이동한 캐릭터의 라벨을 즉시 따라오게
  }

  /** 색만 갱신: 배경 + 바닥/가구 재베이크 + 텐트 카드 재도색. */
  private repaint(): void {
    this.app.renderer.background.color = this.render.background;
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
    this.paintBossSign();
    this.trophyOverlay?.paint(this.theme.pixi);
    this.awardFrame?.paint(this.theme.pixi);
  }

  /** 좌석/보스 히트영역과 휴가 팻말을 파기한다(풍경 전환 전용). */
  private teardownHitAreas(): void {
    for (const hit of this.deskHits) {
      this.floorLayer.removeChild(hit);
      hit.destroy();
    }
    this.deskHits = [];
    if (this.bossHit) {
      this.floorLayer.removeChild(this.bossHit);
      this.bossHit.destroy();
      this.bossHit = undefined;
    }
    this.offVacation?.(); // buildBossDesk가 다시 구독한다 — 중복 구독 방지
    this.offVacation = undefined;
    if (this.bossSign) {
      this.overlayLayer.removeChild(this.bossSign);
      this.bossSign.destroy({ children: true });
      this.bossSign = undefined;
      this.bossSignBoard = undefined;
      this.bossSignLabel = undefined;
    }
    this.offAwardee?.(); // buildAwardDisplays가 다시 구독한다 — 중복 구독 방지
    this.offAwardee = undefined;
    if (this.trophyOverlay) {
      this.sortableLayer.removeChild(this.trophyOverlay.root);
      this.trophyOverlay.destroy();
      this.trophyOverlay = undefined;
    }
    if (this.awardFrame) {
      this.floorLayer.removeChild(this.awardFrame.root);
      this.awardFrame.destroy(); // 텍스처는 안 건드린다(awardPortraitTexture는 캐시로 남아 재구축 후 재적용됨)
      this.awardFrame = undefined;
    }
  }

  /** 텐트 카드를 현재 테마로 (재)도색 — 타일 재베이크(repaint)와 동기.
   * 씬 팔레트가 아니라 **테마** 팔레트를 쓴다(씬 안 텍스트/명패는 테마 축). */
  private paintBossSign(): void {
    if (!this.bossSignBoard || !this.bossSignLabel) return;
    this.bossSignBoard
      .clear()
      .poly([9, -7, 11.5, 0, 7, 0])
      .fill(this.theme.pixi.deskEdge)
      .poly([-7, 0, 7, 0, 9, -7, -5, -7])
      .fill(this.theme.pixi.counterTop);
    this.bossSignLabel.style.fill = this.theme.pixi.text;
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

    const mapPxW = this.map.width * TILE_SIZE;
    const mapPxH = this.map.height * TILE_SIZE;
    const scale = computeIntegerScale(w, h, mapPxW, mapPxH);
    this.worldContainer.scale.set(scale);
    // 커스텀 고해상 시트를 이 정수 스케일에 맞춰 프리필터(이슈 #47). S가 바뀔
    // 때만 커스텀 엔티티 텍스처를 재생성한다(내부에서 no-op 가드).
    this.world.setRenderScale(scale);
    this.bossSignLabel?.scale.set(1 / scale);
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
    this.profiles = profiles; // 트로피 좌석 계산(updateTrophy)이 최신 로스터를 참조
    this.world.syncAgents(profiles);
    this.updateTrophy(); // 로스터/책상 배정이 바뀌면 수상자 좌석도 바뀔 수 있다
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
    this.offVacation?.();
    this.offAwardee?.();
    this.trophyOverlay?.destroy();
    this.trophyOverlay = undefined;
    this.awardFrame?.destroy();
    this.awardFrame = undefined;
    this.disposePortraitTexture();
    this.offHoverGate?.();
    this.world.destroy(); // unconditional: only unsubscribes bus + destroys entities, no Pixi dependency
    if (!this.started) return; // init() never completed -> nothing else to tear down yet
    this.started = false;
    if (this.tickerCallback) this.app.ticker.remove(this.tickerCallback);
    this.app.destroy(true, { children: true, texture: true }); // release GPU resources
  }
}

/**
 * Pixi v8 EventSystem은 pointermove를 캔버스가 아닌 document(캡처)에 등록하므로,
 * 캔버스를 덮은 DOM(터미널 오버레이·다이얼로그 등) 위에서 마우스를 움직여도
 * 씬 히트테스트가 계속 돌아 뒤의 캐릭터에 pointerover가 발화한다. 포인터가
 * 실제로 캔버스 위에 있을 때만 `features.move`를 켜서 이를 차단한다.
 * EventsTicker의 합성 pointermove(정지 커서 밑으로 캐릭터가 걸어올 때)도 같은
 * `features.move` 게이트를 지나므로 함께 정리된다. 캔버스 pointerleave 처리는
 * `features.click` 게이트라 계속 살아 있어, 커서가 오버레이로 넘어가는 순간
 * 기존 hover 상태는 정상 해제된다. 초기값은 false — 첫 pointerenter 전에는
 * 포인터 위치를 모르니 안전하게 차단해 두면, 캔버스 위에서의 첫 이동이 바로
 * enter를 발화해 스스로 풀린다.
 */
export function bindHoverGate(features: { move: boolean }, canvas: HTMLElement): () => void {
  const enter = () => {
    features.move = true;
  };
  const leave = () => {
    features.move = false;
  };
  features.move = false;
  canvas.addEventListener("pointerenter", enter);
  canvas.addEventListener("pointerleave", leave);
  return () => {
    canvas.removeEventListener("pointerenter", enter);
    canvas.removeEventListener("pointerleave", leave);
  };
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
