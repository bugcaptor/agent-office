# 서브시스템 B 상세 설계 — 오피스 씬 & 절차적 픽셀 캐릭터

> 설계: Opus 하위 설계 / 주요 판단: Fable. 계약 정합화 결과는 마스터 플랜(`docs/superpowers/plans/2026-07-06-agent-office.md`)의 "계약 정합화" 절이 우선한다.
> **정합화 반영 사항**: C가 요청한 `generateSpritePreview(seed)`는 씬 메서드가 아니라 `gen/characterFactory.ts`의 순수 함수로 노출한다(§3.5 끝 참조). `SessionState` 타입은 `src/shared/types.ts`의 것을 재사용한다.

대상 경로 루트: `src/renderer/office/`
런타임: **Tauri v2 웹뷰**(WKWebView/WebView2/WebKitGTK), React 셸 안에 마운트되는 PixiJS v8 캔버스. 이 서브시스템은 웹 표준 API만 사용하므로 Electron→Tauri 전환의 영향 없음.
소비 인터페이스: `AgentProfile {id, name, role, seed, ...}`, 이벤트 `agentNotificationChanged(agentId, hasPending)`, `sessionStateChanged(agentId, state)`. 방출: `agentClicked(agentId)`.

---

## 0. 파일 레이아웃 개요

```
src/renderer/office/
  OfficeScene.ts            # Pixi Application 래퍼 (init/resize/destroy/update)
  useOfficeScene.ts         # React 통합 훅 (얇은 브리지)
  OfficeCanvas.tsx          # 캔버스를 호스팅하는 React 컴포넌트 (서브시스템 C 경계)
  types.ts                  # 서브시스템 B 내부 타입 + 소비/방출 계약 타입
  bus.ts                    # 렌더러 내부 이벤트 버스 계약 (A/C ↔ B)
  map/
    mapData.ts              # 하드코딩 타일맵 + 데스크 슬롯 정의
    TileRenderer.ts         # 바닥/벽/데스크 절차적 렌더링
    deskAssignment.ts       # agentId → deskSlot 결정적 배정
  gen/
    prng.ts                 # mulberry32 + 유틸 (hashStringToSeed 등)
    palette.ts              # 팔레트 램프 생성
    parts.ts                # 신체/헤어/의상/액세서리 픽셀 데이터 배열
    compositor.ts           # 파트 합성 → 프레임 → 스프라이트시트 캔버스
    characterFactory.ts     # profile → CharacterAssets (텍스처/애니메이션)
  entities/
    CharacterEntity.ts      # 상태머신 + Pixi 표시객체 소유
    behaviorFsm.ts          # sitting/idle/walking 상태 전이
    ExclamationOverlay.ts   # 머리 위 "!" 바운스 오버레이
  world/
    OfficeWorld.ts          # 엔티티 컬렉션 관리, A/C 이벤트 → 엔티티 반영
    pathing.ts              # 그리드 좌표 ↔ 픽셀 변환, 목적지 선택
  __tests__/
    prng.test.ts
    palette.test.ts
    characterFactory.test.ts
    deskAssignment.test.ts
    behaviorFsm.test.ts
```

핵심 설계 원칙: **`gen/` 이하는 순수 함수** — Pixi/DOM 전역에 의존하지 않고 `OffscreenCanvas`(또는 테스트에서 주입되는 캔버스 팩토리)만 받는다. 결정성/테스트 용이성을 위해서.

---

## 1. PixiJS 씬 아키텍처

### 1.1 레이어 구조

고정 z-order 컨테이너. 캐릭터는 y-정렬(아래쪽일수록 위에 그림)로 데스크와 겹칠 때 자연스러운 깊이감을 준다.

```
stage
 └ worldContainer            (camera transform 적용 대상; scale = zoom, position = pan)
    ├ floorLayer     (Container)  타일 바닥/벽
    ├ sortableLayer  (Container)  데스크 + 캐릭터 (sortableChildren=true, zIndex = worldY)
    └ overlayLayer   (Container)  느낌표/이름표 등 월드 공간 오버레이
```

furnitureLayer와 characterLayer를 **하나의 정렬 컨테이너(`sortableLayer`)**로 합쳐 `sortableChildren = true` + 각 표시객체의 `zIndex = worldY`로 두면 데스크 뒤/앞 판정이 공짜로 된다.

### 1.2 카메라 — MVP: **고정(fixed) + 정수 스케일**, 팬은 후행 확장

- 픽셀 아트 선명도(nearest-neighbor)는 정수 스케일에서만 보장된다. 자유 팬/줌은 서브픽셀 위치를 만들어 흐려짐/떨림을 유발.
- 맵이 작고(20×14 타일) 에이전트 수가 수십 수준이면 팬이 불필요.
- 확장 훅만 남긴다: `setCamera({panX, panY, zoom})`를 두되 MVP 기본 구현은 "맵 전체를 담는 최대 정수 스케일 계산 + 중앙 정렬"만 수행.

```ts
// 뷰포트에 맵을 담는 최대 정수 배율. 최소 1 보장.
function computeIntegerScale(viewW: number, viewH: number, mapPxW: number, mapPxH: number): number {
  const s = Math.floor(Math.min(viewW / mapPxW, viewH / mapPxH));
  return Math.max(1, s);
}
```

### 1.3 OfficeScene 클래스

> (구현 노트: `init()`은 비동기인데, React(특히 StrictMode 이중 마운트)가 `app.init()`이
> resolve되기 전에 effect cleanup(`destroy()`)을 호출할 수 있다. Pixi의
> `Application.renderer`는 `init()` 완료 후에만 할당되므로 그 전에 `app.destroy()`를
> 호출하면 `this.renderer` undefined로 throw한다. 실제 구현은 `destroy()`를 `started`
> 플래그로 가드해 init 이전엔 no-op으로 만들고, `useOfficeScene`의 `disposed` 플래그 +
> init `.then` 콜백이 init 완료 후 실제 teardown을 수행한다. `world.destroy()`는
> `started` 가드보다 먼저, 무조건 실행 — bus 구독 해제만 하고 Pixi 앱에 의존하지
> 않으므로 pre-init destroy 경로에서도 리스너가 새지 않아야 한다. 아래 코드 스니펫은
> 이 가드를 반영해 수정.)

```ts
// src/renderer/office/OfficeScene.ts
import { Application, Container, Ticker } from 'pixi.js';
import { OfficeWorld } from './world/OfficeWorld';
import { TileRenderer } from './map/TileRenderer';
import { OFFICE_MAP, TILE_SIZE } from './map/mapData';
import type { OfficeBus } from './bus';

export interface OfficeSceneOptions {
  canvas: HTMLCanvasElement;
  bus: OfficeBus;            // A/C ↔ B 이벤트 브리지
  background?: number;       // 0x1b1b24 기본
}

export class OfficeScene {
  private app: Application;
  private worldContainer: Container;   // 카메라 트랜스폼 대상
  private sortableLayer: Container;     // furniture + characters (zIndex = worldY)
  private floorLayer: Container;
  private overlayLayer: Container;
  private world!: OfficeWorld;
  private tiles!: TileRenderer;
  private ro?: ResizeObserver;
  private opts: OfficeSceneOptions;
  private started = false;

  constructor(opts: OfficeSceneOptions) {
    this.opts = opts;
    this.app = new Application();
    this.worldContainer = new Container();
    this.floorLayer = new Container();
    this.sortableLayer = new Container();
    this.overlayLayer = new Container();
    this.sortableLayer.sortableChildren = true;
  }

  /** 비동기 초기화. React 훅에서 await. */
  async init(): Promise<void> {
    await this.app.init({
      canvas: this.opts.canvas,
      background: this.opts.background ?? 0x1b1b24,
      antialias: false,                 // 픽셀 아트: AA 끔
      roundPixels: true,                // 서브픽셀 렌더 방지
      resolution: 1,                    // 정수 스케일로 선명도 확보하므로 DPR 배율 미사용
      autoDensity: false,
      resizeTo: this.opts.canvas.parentElement ?? undefined,
    });

    this.worldContainer.addChild(this.floorLayer, this.sortableLayer, this.overlayLayer);
    this.app.stage.addChild(this.worldContainer);

    // 맵 렌더
    this.tiles = new TileRenderer(OFFICE_MAP, TILE_SIZE);
    this.floorLayer.addChild(this.tiles.build());   // 정적 바닥
    this.sortableLayer.addChild(...this.tiles.buildFurniture()); // 데스크(zIndex 설정됨)

    // 월드(엔티티 관리자)
    this.world = new OfficeWorld({
      bus: this.opts.bus,
      characterLayer: this.sortableLayer,
      overlayLayer: this.overlayLayer,
      map: OFFICE_MAP,
    });

    this.applyCamera();

    this.app.ticker.add(this.update);
    this.started = true;

    // 부모 크기 변화 감지 → 카메라 재계산 (Pixi resizeTo가 캔버스 픽셀은 맞춰줌)
    const parent = this.opts.canvas.parentElement;
    if (parent) {
      this.ro = new ResizeObserver(() => this.resize());
      this.ro.observe(parent);
    }
  }

  private update = (ticker: Ticker): void => {
    // deltaMS 기반 업데이트 (프레임률 독립)
    this.world.update(ticker.deltaMS);
  };

  resize(): void {
    if (!this.started) return;
    this.applyCamera();
  }

  private applyCamera(): void {
    const view = this.app.renderer;
    const mapPxW = OFFICE_MAP.width * TILE_SIZE;
    const mapPxH = OFFICE_MAP.height * TILE_SIZE;
    const scale = computeIntegerScale(view.width, view.height, mapPxW, mapPxH);
    this.worldContainer.scale.set(scale);
    // 중앙 정렬 (정수 위치로 스냅해 선명도 유지)
    this.worldContainer.position.set(
      Math.floor((view.width - mapPxW * scale) / 2),
      Math.floor((view.height - mapPxH * scale) / 2),
    );
  }

  /** A/C가 프로필 목록을 넘겨주면 호출 (초기 로드/추가/삭제). */
  syncAgents(profiles: readonly AgentProfile[]): void {
    this.world.syncAgents(profiles);
  }

  destroy(): void {
    this.ro?.disconnect();
    this.ro = undefined;
    this.world?.destroy(); // bus 구독 해제 + 엔티티 정리는 Pixi 앱 초기화와 무관하므로 무조건 실행
    if (!this.started) return; // init()이 끝나기 전 호출된 경우 -> 아직 destroy할 Pixi 리소스가 없음
    this.started = false;
    this.app.ticker.remove(this.update);
    this.app.destroy(true, { children: true, texture: true }); // GPU 리소스 해제
  }
}

function computeIntegerScale(viewW: number, viewH: number, mapPxW: number, mapPxH: number): number {
  const s = Math.floor(Math.min(viewW / mapPxW, viewH / mapPxH));
  return Math.max(1, s);
}
```

### 1.4 React 통합 — `useOfficeScene` 훅 + `OfficeCanvas`

훅은 얇게: 캔버스 ref 소유, 씬 생명주기 관리, 프로필 목록 동기화만. React 리렌더가 Pixi를 재생성하지 않도록 `OfficeScene`은 ref에 보관.

```ts
// src/renderer/office/useOfficeScene.ts
import { useEffect, useRef } from 'react';
import { OfficeScene } from './OfficeScene';
import type { OfficeBus } from './bus';
import type { AgentProfile } from './types';

export function useOfficeScene(bus: OfficeBus, profiles: readonly AgentProfile[]) {
  const canvasRef = useRef<HTMLCanvasElement | null>(null);
  const sceneRef = useRef<OfficeScene | null>(null);

  // 마운트: 씬 1회 생성 (StrictMode 이중 마운트 방어 포함)
  useEffect(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;
    let disposed = false;
    const scene = new OfficeScene({ canvas, bus });
    sceneRef.current = scene;
    scene.init().then(() => {
      if (disposed) { scene.destroy(); return; }
      scene.syncAgents(profiles); // 초기 반영
    });
    return () => {
      disposed = true;
      sceneRef.current = null;
      scene.destroy();
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, []); // bus/profiles 변화는 아래 effect에서 처리

  // 프로필 목록 변화 → 씬에 반영 (Pixi 재생성 없이)
  useEffect(() => {
    sceneRef.current?.syncAgents(profiles);
  }, [profiles]);

  return { canvasRef };
}
```

```tsx
// src/renderer/office/OfficeCanvas.tsx  (서브시스템 C와의 경계 컴포넌트)
import { useOfficeScene } from './useOfficeScene';
import type { OfficeBus } from './bus';
import type { AgentProfile } from './types';

export function OfficeCanvas({ bus, profiles }: { bus: OfficeBus; profiles: readonly AgentProfile[] }) {
  const { canvasRef } = useOfficeScene(bus, profiles);
  return (
    <div style={{ position: 'absolute', inset: 0, imageRendering: 'pixelated' }}>
      <canvas ref={canvasRef} style={{ display: 'block', width: '100%', height: '100%' }} />
    </div>
  );
}
```

### 1.5 이벤트 버스 계약 (`bus.ts`)

A(main, IPC 경유)와 C(React)의 이벤트를 B가 구독/방출하는 얇은 계약. 구현체는 C 담당이 주입(zustand 스토어 백킹)하지만 인터페이스는 여기서 못박는다.

```ts
// src/renderer/office/bus.ts
import type { SessionState } from '../../shared/types';

export interface OfficeBus {
  // B가 구독 (A → B)
  onNotificationChanged(cb: (agentId: string, hasPending: boolean) => void): () => void;
  onSessionStateChanged(cb: (agentId: string, state: SessionState) => void): () => void;
  // B가 방출 (B → C/A)
  emitAgentClicked(agentId: string): void;
}
```

---

## 2. 오피스 맵 (타일 + 데스크)

### 2.1 타일 크기 & 좌표계

- **타일 크기 `TILE_SIZE = 16`** 로직 픽셀. MVP는 **캐릭터 16×16** 채택 → 타일과 1:1로 단순.
- 맵 크기: 20×14 타일 = 320×224 로직 픽셀 (고전 콘솔 해상도 감성).

### 2.2 맵 데이터 표현 — 하드코딩 2D 배열

```ts
// src/renderer/office/map/mapData.ts
export const TILE_SIZE = 16;

export const enum Tile {
  Floor = 0,
  Wall = 1,
  DeskTop = 2,     // 책상 상판 (캐릭터가 앉는 위쪽)
  Rug = 3,         // 장식 러그
}

export interface DeskSlot {
  index: number;     // 0..N-1
  // 캐릭터가 앉는 타일(의자 위치) — 그리드 좌표
  seat: { tx: number; ty: number };
  // 바라보는 방향 (데스크가 위에 있으므로 보통 'up')
  facing: 'up' | 'down' | 'left' | 'right';
}

export interface OfficeMap {
  width: number;
  height: number;
  tiles: readonly (readonly Tile[])[]; // [ty][tx]
  desks: readonly DeskSlot[];
}

// F=Floor, W=Wall, D=DeskTop, R=Rug 로 읽기 쉽게 구성 후 숫자로 변환
const L = (row: string): Tile[] =>
  [...row].map(ch => ({ F: Tile.Floor, W: Tile.Wall, D: Tile.DeskTop, R: Tile.Rug }[ch] ?? Tile.Floor));

const GRID: Tile[][] = [
  L('WWWWWWWWWWWWWWWWWWWW'),
  L('WFFFFFFFFFFFFFFFFFFW'),
  L('WFDDFFDDFFDDFFDDFFFW'), // 데스크 상판 행
  L('WFFFFFFFFFFFFFFFFFFW'), // 의자(seat) 행
  L('WFFFFFFFFFFFFFFFFFFW'),
  L('WFDDFFDDFFDDFFDDFFFW'),
  L('WFFFFFFFFFFFFFFFFFFW'),
  L('WFFFFFFFFFFFFFFFFFFW'),
  L('WFDDFFDDFFDDFFDDFFFW'),
  L('WFFFFFFFFFFFFFFFFFFW'),
  L('WFFFFRRRRRRRRFFFFFFW'),
  L('WFFFFRRRRRRRRFFFFFFW'),
  L('WFFFFFFFFFFFFFFFFFFW'),
  L('WWWWWWWWWWWWWWWWWWWW'),
];

// 데스크 상판(ty=2,5,8)의 각 DeskTop 쌍마다 그 아래 타일을 seat으로 생성
function deriveDesks(grid: Tile[][]): DeskSlot[] {
  const desks: DeskSlot[] = [];
  let idx = 0;
  for (let ty = 0; ty < grid.length; ty++) {
    for (let tx = 0; tx < grid[ty].length; tx++) {
      // 데스크 쌍의 왼쪽 타일에서만 슬롯 생성 (오른쪽은 짝)
      if (grid[ty][tx] === Tile.DeskTop && grid[ty][tx - 1] !== Tile.DeskTop) {
        desks.push({ index: idx++, seat: { tx, ty: ty + 1 }, facing: 'up' });
      }
    }
  }
  return desks;
}

export const OFFICE_MAP: OfficeMap = {
  width: GRID[0].length,
  height: GRID.length,
  tiles: GRID,
  desks: deriveDesks(GRID),
};
```

위 배치는 3행 × 4쌍 = **데스크 슬롯 12개**. 에이전트가 더 많아지면 `GRID`만 확장.

### 2.3 데스크 배정 — 결정적

에이전트 순서가 바뀌어도 같은 에이전트는 같은 자리에 앉도록, `agentId` 해시 기반으로 슬롯을 배정하되 충돌 시 선형 탐사. 순수 함수.

```ts
// src/renderer/office/map/deskAssignment.ts
import { hashStringToSeed } from '../gen/prng';
import type { OfficeMap, DeskSlot } from './mapData';

/** agentId → deskSlot. 입력 순서 무관, 결정적, 충돌은 선형 탐사로 해결. */
export function assignDesks(map: OfficeMap, agentIds: readonly string[]): Map<string, DeskSlot> {
  const n = map.desks.length;
  const taken = new Array<string | null>(n).fill(null);
  const result = new Map<string, DeskSlot>();
  // id 정렬로 순서 독립성 확보
  const ids = [...agentIds].sort();
  for (const id of ids) {
    const start = hashStringToSeed(id) % n;
    for (let k = 0; k < n; k++) {
      const s = (start + k) % n;
      if (taken[s] === null) { taken[s] = id; result.set(id, map.desks[s]); break; }
    }
  }
  return result; // 슬롯 부족 시 초과 에이전트는 미배정(=자유 배회 상태로 표시)
}
```

### 2.4 타일 렌더링 — 절차적 컬러 사각형 + 1px 디테일

임베디드 타일시트 대신 **코드 드로잉**을 선택. 이유: 아트 파이프라인 없음(아키텍트 결정), 팔레트 스왑 용이, 16×16 단순 타일은 코드로 충분. 정적 바닥 전체는 `cacheAsTexture({ scaleMode: "nearest" })`로 한 장으로 굽는다.

> (구현 노트: `cacheAsTexture(true)`는 내부적으로 기본 `scaleMode: "linear"`를 쓰는데, 픽셀 아트에서는 이게 블러를 유발한다. 실제 구현(`src/renderer/office/map/TileRenderer.ts`)은 `cacheAsTexture({ scaleMode: "nearest" })`로 굽는다.)

```ts
// src/renderer/office/map/TileRenderer.ts
import { Container, Graphics, Texture, Renderer } from 'pixi.js';
import { OfficeMap, Tile, TILE_SIZE } from './mapData';

const PAL = {
  floorA: 0x3a3a4a, floorB: 0x34343f, floorDot: 0x2e2e38,
  wall: 0x22222c, wallTop: 0x3a3a48,
  desk: 0x8a5a34, deskEdge: 0x6b4526, deskTop: 0xa9723f,
  rug: 0x2f5d5b, rugEdge: 0x264b49,
};

export class TileRenderer {
  private textures = new Map<Tile, Texture>();
  constructor(private map: OfficeMap, private tile = TILE_SIZE, private renderer?: Renderer) {}

  /** 정적 바닥+벽 레이어. 체커보드 + 1px 도트 디테일. */
  build(): Container {
    const root = new Container();
    for (let ty = 0; ty < this.map.height; ty++) {
      for (let tx = 0; tx < this.map.width; tx++) {
        const t = this.map.tiles[ty][tx];
        if (t === Tile.DeskTop) continue; // 데스크는 furniture 레이어에서
        const g = this.drawTile(t, tx, ty);
        g.position.set(tx * this.tile, ty * this.tile);
        root.addChild(g);
      }
    }
    root.cacheAsTexture({ scaleMode: "nearest" }); // 정적 → 한 장으로 굽기 (Pixi v8 API; nearest로 픽셀아트 블러 방지)
    return root;
  }

  /** 데스크는 y-sort 대상이라 개별 스프라이트로. zIndex = 하단 y. */
  buildFurniture(): Container[] {
    const out: Container[] = [];
    for (let ty = 0; ty < this.map.height; ty++) {
      for (let tx = 0; tx < this.map.width; tx++) {
        if (this.map.tiles[ty][tx] !== Tile.DeskTop) continue;
        const g = this.drawTile(Tile.DeskTop, tx, ty);
        g.position.set(tx * this.tile, ty * this.tile);
        g.zIndex = (ty + 1) * this.tile; // 데스크 하단 기준 정렬
        out.push(g);
      }
    }
    return out;
  }

  private drawTile(t: Tile, tx: number, ty: number): Graphics {
    const g = new Graphics();
    const s = this.tile;
    switch (t) {
      case Tile.Floor: {
        const checker = (tx + ty) % 2 === 0 ? PAL.floorA : PAL.floorB;
        g.rect(0, 0, s, s).fill(checker);
        // 1px 픽셀 디테일: 코너 도트
        g.rect(1, 1, 1, 1).fill(PAL.floorDot);
        g.rect(s - 2, s - 2, 1, 1).fill(PAL.floorDot);
        break;
      }
      case Tile.Wall:
        g.rect(0, 0, s, s).fill(PAL.wall);
        g.rect(0, 0, s, 3).fill(PAL.wallTop); // 상단 하이라이트 3px
        break;
      case Tile.DeskTop:
        g.rect(0, 0, s, s).fill(PAL.desk);
        g.rect(0, 0, s, 4).fill(PAL.deskTop);      // 상판 밝은 면
        g.rect(0, s - 2, s, 2).fill(PAL.deskEdge); // 하단 그림자
        g.rect(2, 6, s - 4, 1).fill(PAL.deskEdge); // 나뭇결 1px
        break;
      case Tile.Rug:
        g.rect(0, 0, s, s).fill(PAL.rug);
        g.rect(0, 0, s, 1).fill(PAL.rugEdge);
        g.rect(0, 0, 1, s).fill(PAL.rugEdge);
        break;
    }
    return g;
  }
}
```

주: Pixi v8 `Graphics`는 `.rect().fill()` 체이닝 API를 쓴다(v7 `beginFill`은 폐기). `cacheAsTexture`도 v8 명칭.

---

## 3. 절차적 캐릭터 생성기

파이프라인: `profile.seed(문자열) → mulberry32 PRNG → 파트 선택 + 팔레트 램프 → 파트 픽셀 배열을 오프스크린 캔버스에 합성 → 프레임(idle 2 + walk 2) → 스프라이트시트 캔버스 → Pixi Texture(nearest)`.

### 3.1 PRNG (`gen/prng.ts`)

```ts
// src/renderer/office/gen/prng.ts

/** 문자열 → 32bit 시드 (xfnv1a). 결정적. */
export function hashStringToSeed(str: string): number {
  let h = 2166136261 >>> 0;
  for (let i = 0; i < str.length; i++) {
    h ^= str.charCodeAt(i);
    h = Math.imul(h, 16777619);
  }
  return h >>> 0;
}

/** mulberry32: 빠르고 결정적인 32bit PRNG. [0,1) 반환. */
export function mulberry32(seed: number): () => number {
  let a = seed >>> 0;
  return function () {
    a |= 0; a = (a + 0x6d2b79f5) | 0;
    let t = Math.imul(a ^ (a >>> 15), 1 | a);
    t = (t + Math.imul(t ^ (t >>> 7), 61 | t)) ^ t;
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

/** PRNG 헬퍼 묶음. */
export interface Rng {
  next(): number;                    // [0,1)
  int(maxExclusive: number): number; // [0,max)
  range(min: number, max: number): number;
  pick<T>(arr: readonly T[]): T;
  bool(p?: number): boolean;
}

export function makeRng(seed: number): Rng {
  const r = mulberry32(seed);
  return {
    next: r,
    int: (m) => Math.floor(r() * m),
    range: (min, max) => min + r() * (max - min),
    pick: (arr) => arr[Math.floor(r() * arr.length)],
    bool: (p = 0.5) => r() < p,
  };
}
```

### 3.2 팔레트 생성 (`gen/palette.ts`)

HSL로 램프(그림자/기본/하이라이트)를 만들어 대비를 보장. 피부/헤어/의상 각각의 램프를 생성하고, **의상과 피부 사이 최소 명도 대비 제약**을 강제(테스트로 검증).

```ts
// src/renderer/office/gen/palette.ts
import type { Rng } from './prng';

export interface Ramp { shadow: number; base: number; light: number; } // 0xRRGGBB
export interface CharacterPalette {
  skin: Ramp;
  hair: Ramp;
  shirt: Ramp;
  pants: Ramp;
  outline: number; // 공통 외곽선
}

function clamp01(v: number): number { return Math.min(1, Math.max(0, v)); }

export function hslToRgb(h: number, s: number, l: number): number {
  h = ((h % 360) + 360) % 360; s = clamp01(s); l = clamp01(l);
  const c = (1 - Math.abs(2 * l - 1)) * s;
  const x = c * (1 - Math.abs(((h / 60) % 2) - 1));
  const m = l - c / 2;
  let r = 0, g = 0, b = 0;
  if (h < 60) [r, g, b] = [c, x, 0];
  else if (h < 120) [r, g, b] = [x, c, 0];
  else if (h < 180) [r, g, b] = [0, c, x];
  else if (h < 240) [r, g, b] = [0, x, c];
  else if (h < 300) [r, g, b] = [x, 0, c];
  else [r, g, b] = [c, 0, x];
  return (Math.round((r + m) * 255) << 16) | (Math.round((g + m) * 255) << 8) | Math.round((b + m) * 255);
}

/** 상대 휘도 (WCAG 근사) — 대비 테스트에 사용. */
export function luminance(rgb: number): number {
  const f = (v: number) => { v /= 255; return v <= 0.03928 ? v / 12.92 : ((v + 0.055) / 1.055) ** 2.4; };
  return 0.2126 * f((rgb >> 16) & 255) + 0.7152 * f((rgb >> 8) & 255) + 0.0722 * f(rgb & 255);
}
export function contrastRatio(a: number, b: number): number {
  const la = luminance(a), lb = luminance(b);
  return (Math.max(la, lb) + 0.05) / (Math.min(la, lb) + 0.05);
}

function ramp(h: number, s: number, l: number, spread = 0.16): Ramp {
  return {
    shadow: hslToRgb(h, s, Math.max(0.06, l - spread)),
    base: hslToRgb(h, s, l),
    light: hslToRgb(h, s, Math.min(0.94, l + spread)),
  };
}

const SKIN_TONES: ReadonlyArray<[number, number, number]> = [
  [28, 0.45, 0.78], [26, 0.5, 0.66], [24, 0.5, 0.52], [20, 0.5, 0.38], [18, 0.45, 0.28],
];

export function generatePalette(rng: Rng): CharacterPalette {
  const [sh, ss, sl] = rng.pick(SKIN_TONES);
  const skin = ramp(sh, ss, sl, 0.1);

  const hairHue = rng.pick([20, 30, 40, 0, 200, 280, 45]); // 갈/금/흑(저채도)/빨강/파랑/보라
  const hairL = rng.range(0.18, 0.6);
  const hair = ramp(hairHue, rng.range(0.25, 0.7), hairL, 0.14);

  // 의상: 피부 대비를 만족할 때까지 재시도 (최대 8회), 실패 시 명도 강제 클램프
  let shirt = ramp(rng.range(0, 360), rng.range(0.4, 0.85), rng.range(0.35, 0.6));
  for (let i = 0; i < 8 && contrastRatio(shirt.base, skin.base) < 1.6; i++) {
    shirt = ramp(rng.range(0, 360), rng.range(0.4, 0.85), rng.range(0.3, 0.62));
  }
  if (contrastRatio(shirt.base, skin.base) < 1.6) {
    // 최종 클램프: 실측 contrastRatio 기반 lightness 스캔으로 대비 >= 1.6을 보장
    // (clampShirtRamp — 아래 참조). hue는 유지, rng 소비는 hue 1회로 기존과 동일.
    shirt = clampShirtRamp(rng.range(0, 360), skin.base, sl > 0.5);
  }
  const pants = ramp(rng.range(0, 360), rng.range(0.2, 0.6), rng.range(0.22, 0.42));

  return { skin, hair, shirt, pants, outline: 0x1a1420 };
}

/**
 * 최종 클램프 (구현 노트: hue만 랜덤이고 s/l이 고정인 "hue 랜덤 + 고정 s/l"
 * 클램프는 명도가 같아도 hue에 따라 상대 휘도가 달라져(WCAG 가중치 R
 * 0.2126 / G 0.7152 / B 0.0722) 대비 1.6을 보장하지 못한다 — 검증된 반례:
 * seed 2274 → 대비 ≈1.52. 실제 구현은 hue는 유지한 채 `contrastRatio`를
 * 재계산하며 l을 피부의 반대 극단(피부가 밝으면 0, 어두우면 1) 방향으로
 * 스캔해 대비 >= 1.6이 될 때까지 조정한다. 흑/백 극단에서 항상 대비가
 * 충족되므로 종료가 보장되며, rng를 쓰지 않아 결정적이다.)
 */
export function clampShirtRamp(hue: number, skinBase: number, skinIsLight: boolean): Ramp {
  const sat = 0.6;
  const step = skinIsLight ? -0.02 : 0.02;
  let l = skinIsLight ? 0.28 : 0.7;
  while (
    contrastRatio(hslToRgb(hue, sat, l), skinBase) < SHIRT_SKIN_MIN_CONTRAST &&
    l > 0 && l < 1
  ) {
    l = Math.min(1, Math.max(0, l + step));
  }
  return ramp(hue, sat, l);
}
```

### 3.3 파트 픽셀 데이터 (`gen/parts.ts`)

16×16 그리드. 각 파트는 **16개 문자열 행**으로 인코딩 — 문자 하나가 픽셀 하나이며, 문자 = 팔레트 슬롯 키. 실제 픽셀 배열을 코드에 그대로 담고 사람이 읽고 편집 가능하다.

문자 → 색상 슬롯 매핑:

```
'.' = 투명
'o' = outline
'S' = skin.shadow  's' = skin.base  'H' = skin.light   (얼굴/손)
'A' = hair.shadow  'a' = hair.base  'B' = hair.light
'C' = shirt.shadow 'c' = shirt.base 'D' = shirt.light
'P' = pants.shadow 'p' = pants.base
'e' = 눈 (outline과 동일 어둠)  'W' = 흰자/하이라이트
```

```ts
// src/renderer/office/gen/parts.ts

/** 16x16, 각 문자열은 16글자. 문자는 팔레트 슬롯 키. */
export type PixelRows = readonly string[];

// ── 신체 베이스 (정면). 머리/몸통/팔/다리. 헤어/의상은 별도 레이어로 덮음.
export const BODY_BASE_FRONT: PixelRows = [
  '................',
  '................',
  '....oooooo......',
  '...osssssso.....',
  '...osHssHso.....', // 이마 하이라이트
  '...osssssso.....',
  '...oseSSeso.....', // 눈(e) + 코 그림자
  '...osssssso.....',
  '....oCCCCo......', // 목→셔츠 시작
  '...oCcccccCo....', // 몸통 셔츠
  '..osCcccccCso...', // 팔(피부 소매 끝은 손)
  '..oHcccccccHo...', // 손(H)
  '...oCcccccCo....',
  '...opppppppo....', // 바지
  '...opp..ppo.....', // 다리 분리
  '...oPP..PPo.....', // 발/신발 그림자
] as const;

// walk용 다리 스왑 프레임 (y=12..15 영역만 유효, 나머지 투명)
//
// > (구현 노트: 16x16 그리드 불변식상 12개 투명 행 + y=12..15의 4행 = 16행이어야
// > 하는데, 이전 판의 배열은 17개 문자열(마지막 전부-투명 행이 잉여)이었다.
// > compositor의 blitLayer는 CELL=16 폭 셀에 dy+y로 그대로 찍으므로 17번째
// > 행(y=16)이 존재하면 다음 프레임 셀로 픽셀이 새는 오프바이원 버그가 된다.
// > src/renderer/office/gen/parts.ts 기준으로 16행으로 수정.)
export const LEGS_WALK_A: PixelRows = [
  '................','................','................','................',
  '................','................','................','................',
  '................','................','................','................',
  '...opppppppo....',
  '...oppp.pppo....', // 왼발 앞
  '...oPP...PPo....',
  '....o....o......',
];
export const LEGS_WALK_B: PixelRows = [
  '................','................','................','................',
  '................','................','................','................',
  '................','................','................','................',
  '...opppppppo....',
  '...opp.ppppo....', // 오른발 앞 (대칭)
  '...oPP...PPo....',
  '......o....o....',
];

// ── 헤어 변종 (MVP 4종). 머리 위/옆을 덮음.
export const HAIR_VARIANTS: Record<string, PixelRows> = {
  short: [
    '................','....AaaaB.......','...AaaaaaB......','...aaaaaaa......',
    '...aa....aa.....','................','................','................',
    '................','................','................','................',
    '................','................','................','................',
  ],
  bob: [
    '................','...AaaaaB.......','..AaaaaaaB......','..aaaaaaaa......',
    '..aa.....aa.....','..a.......a.....','..a.......a.....','................',
    '................','................','................','................',
    '................','................','................','................',
  ],
  spiky: [
    '.....a.a.a......','....AaaaaB......','...AaaaaaaB.....','...aaaaaaa......',
    '...a......a.....','................','................','................',
    '................','................','................','................',
    '................','................','................','................',
  ],
  bald: [
    '................','................','....AaaB........','................',
    '................','................','................','................',
    '................','................','................','................',
    '................','................','................','................',
  ],
};

// ── 의상 변종 (MVP 3종): 몸통 패턴 오버레이 (셔츠 위 디테일).
export const CLOTHES_VARIANTS: Record<string, PixelRows> = {
  plain: EMPTY16(),                     // 베이스 셔츠 그대로
  stripe: overlayRows({ 10: '....D.D.D.....', 11: '....D.D.D.....' }),
  vest:   overlayRows({ 9: '...oCC..CCo....', 10: '...oC....Co....', 11: '...oC....Co....' }),
};

// ── 액세서리 변종 (MVP 3종 + none): 안경/헤드셋/모자.
export const ACCESSORY_VARIANTS: Record<string, PixelRows> = {
  none: EMPTY16(),
  glasses: overlayRows({ 6: '...oeoOeoeo.....' }),
  headset: overlayRows({ 3: '...o......o.....', 6: '..W........W....' }),
  cap:     overlayRows({ 1: '...oDDDDDDo.....', 2: '..oDDDDDDDDo....' }),
};

export function EMPTY16(): PixelRows {
  return Array.from({ length: 16 }, () => '................');
}
function overlayRows(rows: Record<number, string>): PixelRows {
  const out = EMPTY16().slice();
  for (const [y, s] of Object.entries(rows)) out[+y] = (s + '................').slice(0, 16);
  return out;
}

export const BODY_VARIANTS_COUNT = 1;  // MVP: 베이스 1종(팔레트로 다양성 확보)
export const HAIR_KEYS = Object.keys(HAIR_VARIANTS);      // 4
export const CLOTHES_KEYS = Object.keys(CLOTHES_VARIANTS); // 3
export const ACCESSORY_KEYS = Object.keys(ACCESSORY_VARIANTS); // 4
```

조합 다양성: 헤어 4 × 의상 3 × 액세서리 4 = 48 실루엣 × (피부 5 × 헤어색 7 × 셔츠색 연속) → 사실상 무한. MVP로 충분.

### 3.4 합성기 (`gen/compositor.ts`)

문자 → 색상 해석 후 오프스크린 캔버스에 픽셀을 찍는다. 레이어 순서: **body → clothes → hair → accessory**. 프레임 구성: idle 2프레임(bob 0px / 1px), walk 2프레임(다리 스왑 + bob). 스프라이트시트는 가로로 나열.

```ts
// src/renderer/office/gen/compositor.ts
import type { CharacterPalette } from './palette';
import type { PixelRows } from './parts';

export const CELL = 16;                 // 셀(캐릭터) 픽셀 크기
export type FrameName = 'idle0' | 'idle1' | 'walk0' | 'walk1';
export const FRAME_ORDER: FrameName[] = ['idle0', 'idle1', 'walk0', 'walk1'];

/** 캔버스 생성 추상화 — 브라우저는 OffscreenCanvas, 테스트는 주입 가능. */
export type CanvasFactory = (w: number, h: number) => {
  ctx: CanvasRenderingContext2D | OffscreenCanvasRenderingContext2D;
  canvas: HTMLCanvasElement | OffscreenCanvas;
};

export const defaultCanvasFactory: CanvasFactory = (w, h) => {
  const canvas = typeof OffscreenCanvas !== 'undefined'
    ? new OffscreenCanvas(w, h)
    : Object.assign(document.createElement('canvas'), { width: w, height: h });
  const ctx = (canvas as any).getContext('2d')!;
  (ctx as any).imageSmoothingEnabled = false;
  return { ctx, canvas: canvas as any };
};

/** 문자 → 0xRRGGBB 또는 null(투명) */
function resolveChar(ch: string, pal: CharacterPalette): number | null {
  switch (ch) {
    case '.': return null;
    case 'o': return pal.outline;
    case 'S': return pal.skin.shadow; case 's': return pal.skin.base; case 'H': return pal.skin.light;
    case 'A': return pal.hair.shadow; case 'a': return pal.hair.base;  case 'B': return pal.hair.light;
    case 'C': return pal.shirt.shadow;case 'c': return pal.shirt.base; case 'D': return pal.shirt.light;
    case 'P': return pal.pants.shadow;case 'p': return pal.pants.base;
    case 'e': return pal.outline;      // 눈
    case 'W': return 0xffffff;
    default:  return null;
  }
}

/** 한 레이어(PixelRows)를 (dx,dy) 오프셋으로 ctx에 찍음. */
function blitLayer(
  ctx: CanvasRenderingContext2D | OffscreenCanvasRenderingContext2D,
  rows: PixelRows, pal: CharacterPalette, dx: number, dy: number,
): void {
  for (let y = 0; y < rows.length; y++) {
    const row = rows[y];
    for (let x = 0; x < row.length; x++) {
      const c = resolveChar(row[x], pal);
      if (c === null) continue;
      ctx.fillStyle = `#${c.toString(16).padStart(6, '0')}`;
      ctx.fillRect(dx + x, dy + y, 1, 1);
    }
  }
}

export interface CharacterLayers {
  body: PixelRows;
  clothes: PixelRows;
  hair: PixelRows;
  accessory: PixelRows;
  legsWalkA: PixelRows;
  legsWalkB: PixelRows;
}

/** 한 프레임을 (frameX*CELL, 0)에 그림. bob과 다리 스왑을 프레임별로 적용. */
function drawFrame(
  ctx: CanvasRenderingContext2D | OffscreenCanvasRenderingContext2D,
  frame: FrameName, layers: CharacterLayers, pal: CharacterPalette, ox: number,
): void {
  const bob = (frame === 'idle1' || frame === 'walk1') ? 1 : 0; // 1px 흔들림
  blitLayer(ctx, layers.body, pal, ox, bob);
  if (frame === 'walk0') blitLayer(ctx, layers.legsWalkA, pal, ox, bob);
  if (frame === 'walk1') blitLayer(ctx, layers.legsWalkB, pal, ox, bob);
  blitLayer(ctx, layers.clothes, pal, ox, bob);
  blitLayer(ctx, layers.hair, pal, ox, bob);
  blitLayer(ctx, layers.accessory, pal, ox, bob);
}

export interface SpriteSheetResult {
  canvas: HTMLCanvasElement | OffscreenCanvas;
  cell: number;
  frames: FrameName[];        // 인덱스 = frameX
  frameRects: Record<FrameName, { x: number; y: number; w: number; h: number }>;
}

/** 4프레임 가로 스프라이트시트 생성 (64x16). 순수. */
export function composeSpriteSheet(
  layers: CharacterLayers, pal: CharacterPalette, factory: CanvasFactory = defaultCanvasFactory,
): SpriteSheetResult {
  const { ctx, canvas } = factory(CELL * FRAME_ORDER.length, CELL);
  const frameRects = {} as SpriteSheetResult['frameRects'];
  FRAME_ORDER.forEach((f, i) => {
    drawFrame(ctx, f, layers, pal, i * CELL);
    frameRects[f] = { x: i * CELL, y: 0, w: CELL, h: CELL };
  });
  return { canvas, cell: CELL, frames: FRAME_ORDER, frameRects };
}
```

### 3.5 팩토리 — profile → Pixi 애니메이션 (`gen/characterFactory.ts`)

시그니처 체인의 종착점. 여기서만 Pixi에 의존(텍스처화). 순수 부분(`composeSpriteSheet`)과 분리.

```ts
// src/renderer/office/gen/characterFactory.ts
import { Texture, Rectangle } from 'pixi.js';
import { makeRng, hashStringToSeed } from './prng';
import { generatePalette } from './palette';
import {
  BODY_BASE_FRONT, LEGS_WALK_A, LEGS_WALK_B,
  HAIR_VARIANTS, CLOTHES_VARIANTS, ACCESSORY_VARIANTS,
  HAIR_KEYS, CLOTHES_KEYS, ACCESSORY_KEYS,
} from './parts';
import { composeSpriteSheet, FrameName, CanvasFactory, defaultCanvasFactory, CELL } from './compositor';
import type { AgentProfile } from '../types';

export interface CharacterAssets {
  base: Texture;                         // 시트 전체 (nearest)
  frames: Record<FrameName, Texture>;    // 프레임별 서브텍스처
  idle: Texture[];                       // [idle0, idle1]
  walk: Texture[];                       // [walk0, walk1]
  descriptor: { hair: string; clothes: string; accessory: string; }; // 디버그/프로필 표시
}

/** seed 문자열(보통 profile.seed || profile.id) → 결정적 파트 선택 */
export function selectLayers(seed: string) {
  const rng = makeRng(hashStringToSeed(seed));
  const pal = generatePalette(rng);
  const hairKey = rng.pick(HAIR_KEYS);
  const clothesKey = rng.pick(CLOTHES_KEYS);
  const accKey = rng.pick(ACCESSORY_KEYS);
  return {
    pal,
    descriptor: { hair: hairKey, clothes: clothesKey, accessory: accKey },
    layers: {
      body: BODY_BASE_FRONT,
      clothes: CLOTHES_VARIANTS[clothesKey],
      hair: HAIR_VARIANTS[hairKey],
      accessory: ACCESSORY_VARIANTS[accKey],
      legsWalkA: LEGS_WALK_A,
      legsWalkB: LEGS_WALK_B,
    },
  };
}

/** 순수 시트 생성(테스트에서 픽셀 비교에 사용). */
export function generateSheet(seed: string, factory: CanvasFactory = defaultCanvasFactory) {
  const { pal, layers, descriptor } = selectLayers(seed);
  return { sheet: composeSpriteSheet(layers, pal, factory), descriptor };
}

/** Pixi 텍스처까지. 렌더러 컨텍스트 필요. */
export function createCharacterAssets(profile: AgentProfile): CharacterAssets {
  const seed = profile.seed || profile.id;
  const { sheet, descriptor } = generateSheet(seed);
  const base = Texture.from(sheet.canvas as any);
  base.source.scaleMode = 'nearest';     // Pixi v8: 픽셀 선명도
  const frames = {} as Record<FrameName, Texture>;
  for (const f of sheet.frames) {
    const r = sheet.frameRects[f];
    frames[f] = new Texture({ source: base.source, frame: new Rectangle(r.x, r.y, r.w, r.h) });
  }
  return {
    base, frames, descriptor,
    idle: [frames.idle0, frames.idle1],
    walk: [frames.walk0, frames.walk1],
  };
}

/**
 * (정합화: C의 프로필 다이얼로그 라이브 프리뷰용 — 순수, Pixi 비의존)
 * idle0 프레임만 잘라 scale배 확대한 PNG dataURL 반환.
 */
export function generateSpritePreview(seed: string, scale = 6): string {
  const { sheet } = generateSheet(seed);
  const out = document.createElement('canvas');
  out.width = CELL * scale; out.height = CELL * scale;
  const ctx = out.getContext('2d')!;
  ctx.imageSmoothingEnabled = false;
  ctx.drawImage(sheet.canvas as CanvasImageSource, 0, 0, CELL, CELL, 0, 0, CELL * scale, CELL * scale);
  return out.toDataURL('image/png');
}
```

---

## 4. 캐릭터 거동 (상태머신 + 이동 + 오버레이 + 히트테스트)

### 4.1 상태머신 (`entities/behaviorFsm.ts`)

3상태: `sitting`(데스크 착석, 기본), `idleWander`(자리 근처 서성임 대기), `walking`(목적지 이동). MVP는 **대부분 sitting**, 낮은 확률로 잠깐 wander 후 복귀 → "생동감". 순수 전이 함수로 테스트 가능.

```ts
// src/renderer/office/entities/behaviorFsm.ts
export type BehaviorState = 'sitting' | 'idleWander' | 'walking';

export interface FsmContext {
  atDesk: boolean;        // 현재 데스크 좌석에 있는가
  hasPending: boolean;    // 알림 대기 (있으면 자리 지킴)
  timerMs: number;        // 현재 상태 경과
  rand: number;           // [0,1) 이번 틱 난수
}
export interface FsmResult {
  next: BehaviorState;
  // walking 진입 시 목적지 요청 플래그
  requestWanderTarget?: boolean;
  requestReturnToDesk?: boolean;
}

const SIT_MIN_MS = 6000;         // 최소 착석 시간
const WANDER_CHANCE_PER_SEC = 0.06;
const WANDER_MAX_MS = 4000;

export function stepBehavior(state: BehaviorState, c: FsmContext, dtMs: number): FsmResult {
  switch (state) {
    case 'sitting': {
      // 알림 대기 중엔 자리 고정
      if (c.hasPending) return { next: 'sitting' };
      if (c.timerMs < SIT_MIN_MS) return { next: 'sitting' };
      // 초당 확률 → 이번 틱 확률
      const p = 1 - Math.pow(1 - WANDER_CHANCE_PER_SEC, dtMs / 1000);
      if (c.rand < p) return { next: 'walking', requestWanderTarget: true };
      return { next: 'sitting' };
    }
    case 'walking': {
      // 도착 판정은 이동 컨트롤러가 하고 도착 시 상태 종료를 부름
      return { next: 'walking' };
    }
    case 'idleWander': {
      if (c.hasPending || c.timerMs > WANDER_MAX_MS) {
        return { next: 'walking', requestReturnToDesk: true };
      }
      return { next: 'idleWander' };
    }
  }
}
```

착석↔배회 흐름: `sitting → walking(→wander target) → 도착 → idleWander → walking(→return desk) → 도착 → sitting`. 도착 이벤트는 이동 컨트롤러가 소유.

### 4.2 이동 — 그리드 목표 + 자유 보간 (`world/pathing.ts`)

```ts
// src/renderer/office/world/pathing.ts
import { OfficeMap, Tile, TILE_SIZE } from '../map/mapData';

export interface GridPos { tx: number; ty: number; }
export const tileCenterPx = (p: GridPos) => ({
  x: p.tx * TILE_SIZE + TILE_SIZE / 2,
  y: p.ty * TILE_SIZE + TILE_SIZE / 2,
});
export const isWalkable = (m: OfficeMap, tx: number, ty: number): boolean =>
  ty >= 0 && ty < m.height && tx >= 0 && tx < m.width && m.tiles[ty][tx] === Tile.Floor;

/** seat 주변 걷기 가능한 임의 타일(배회 목적지) 선택. 결정성 불필요(런타임). */
export function pickWanderTarget(m: OfficeMap, near: GridPos, rand: () => number, radius = 3): GridPos {
  for (let i = 0; i < 12; i++) {
    const tx = near.tx + Math.round((rand() * 2 - 1) * radius);
    const ty = near.ty + Math.round((rand() * 2 - 1) * radius);
    if (isWalkable(m, tx, ty)) return { tx, ty };
  }
  return near; // 실패 시 제자리
}
```

이동 실행은 `CharacterEntity.update`에서 목표 픽셀로 `speed*dt`만큼 접근, 도착 임계(<0.5px)면 도착 처리.

### 4.3 CharacterEntity (`entities/CharacterEntity.ts`)

Pixi 표시객체(캐릭터 스프라이트 + 오버레이)와 상태를 소유. `AnimatedSprite` 대신 수동 프레임 교체(2프레임이라 단순, 애니 속도 제어 쉬움).

```ts
// src/renderer/office/entities/CharacterEntity.ts
import { Container, Sprite } from 'pixi.js';
import type { CharacterAssets } from '../gen/characterFactory';
import { OfficeMap, TILE_SIZE } from '../map/mapData';
import { tileCenterPx, pickWanderTarget, GridPos } from '../world/pathing';
import { stepBehavior, BehaviorState } from './behaviorFsm';
import { ExclamationOverlay } from './ExclamationOverlay';

const WALK_SPEED = 28;      // px/sec
const ANIM_IDLE_MS = 480;   // idle 프레임 교체 주기
const ANIM_WALK_MS = 140;

export class CharacterEntity {
  readonly root = new Container();       // sortableLayer에 추가됨. zIndex = worldY
  private sprite: Sprite;
  private overlay: ExclamationOverlay;
  private state: BehaviorState = 'sitting';
  private stateTimer = 0;
  private animTimer = 0;
  private frameIdx = 0;
  private targetPx: { x: number; y: number } | null = null;
  private hasPending = false;
  private rand: () => number;

  constructor(
    readonly agentId: string,
    private assets: CharacterAssets,
    private seat: GridPos,             // 배정된 좌석
    private map: OfficeMap,
    rand: () => number,
  ) {
    this.rand = rand;
    this.sprite = new Sprite(assets.idle[0]);
    this.sprite.anchor.set(0.5, 1);    // 발 기준 정렬 (y-sort용)
    this.root.addChild(this.sprite);

    this.overlay = new ExclamationOverlay();
    this.overlay.root.position.set(0, -TILE_SIZE);  // 머리 위
    this.root.addChild(this.overlay.root);
    this.overlay.setVisible(false);

    // 좌석 착석 배치
    const p = tileCenterPx(seat);
    this.root.position.set(p.x, p.y + TILE_SIZE / 2); // 발이 좌석 하단
    this.root.zIndex = this.root.y;

    // 클릭 히트테스트
    this.sprite.eventMode = 'static';
    this.sprite.cursor = 'pointer';
  }

  onClicked(cb: (id: string) => void): void {
    this.sprite.on('pointertap', () => cb(this.agentId));
  }

  setPending(v: boolean): void {
    this.hasPending = v;
    this.overlay.setVisible(v);
  }

  /** dt: ms */
  update(dt: number): void {
    this.stateTimer += dt;
    const r = this.rand();
    const res = stepBehavior(this.state, {
      atDesk: this.targetPx === null && this.state === 'sitting',
      hasPending: this.hasPending, timerMs: this.stateTimer, rand: r,
    }, dt);

    if (res.next !== this.state) { this.state = res.next; this.stateTimer = 0; }
    if (res.requestWanderTarget) this.setTarget(pickWanderTarget(this.map, this.seat, this.rand));
    if (res.requestReturnToDesk) this.setTarget(this.seat);

    if (this.targetPx) this.moveToward(dt);
    this.animate(dt);
    this.root.zIndex = this.root.y;   // y-sort 갱신
    this.overlay.update(dt);
  }

  private setTarget(g: GridPos): void {
    const p = tileCenterPx(g);
    this.targetPx = { x: p.x, y: p.y + TILE_SIZE / 2 };
  }

  private moveToward(dt: number): void {
    const t = this.targetPx!;
    const dx = t.x - this.root.x, dy = t.y - this.root.y;
    const dist = Math.hypot(dx, dy);
    const step = (WALK_SPEED * dt) / 1000;
    if (dist <= step || dist < 0.5) {
      this.root.position.set(Math.round(t.x), Math.round(t.y));
      this.targetPx = null;
      // 도착: walking 종료 → wander 중이면 idleWander, 좌석 복귀면 sitting
      this.state = this.isAtSeat() ? 'sitting' : 'idleWander';
      this.stateTimer = 0;
      return;
    }
    this.sprite.scale.x = dx < 0 ? -1 : 1; // 진행 방향 flip
    this.root.x += (dx / dist) * step;
    this.root.y += (dy / dist) * step;
  }

  private isAtSeat(): boolean {
    const p = tileCenterPx(this.seat);
    return Math.abs(this.root.x - p.x) < 1 && Math.abs(this.root.y - (p.y + TILE_SIZE / 2)) < 1;
  }

  private animate(dt: number): void {
    const walking = this.state === 'walking';
    const frames = walking ? this.assets.walk : this.assets.idle;
    this.animTimer += dt;
    const period = walking ? ANIM_WALK_MS : ANIM_IDLE_MS;
    if (this.animTimer >= period) {
      this.animTimer = 0;
      this.frameIdx = (this.frameIdx + 1) % frames.length;
    }
    this.sprite.texture = frames[this.frameIdx % frames.length];
  }

  destroy(): void {
    this.overlay.destroy();
    this.root.destroy({ children: true });
  }
}
```

### 4.4 느낌표 오버레이 (`entities/ExclamationOverlay.ts`)

"!" 를 코드 드로잉 텍스처로 굽고, sin 바운스. 노란 말풍선 느낌.

```ts
// src/renderer/office/entities/ExclamationOverlay.ts
import { Container, Graphics } from 'pixi.js';

export class ExclamationOverlay {
  readonly root = new Container();
  private mark: Graphics;
  private t = 0;

  constructor() {
    this.mark = new Graphics();
    // 노란 원형 배경 + "!" (픽셀 아트풍)
    this.mark.circle(0, 0, 6).fill(0xffcc33).stroke({ width: 1, color: 0x8a5a00 });
    this.mark.rect(-1, -4, 2, 5).fill(0x3a2600); // 느낌표 몸통
    this.mark.rect(-1, 3, 2, 2).fill(0x3a2600);  // 점
    this.root.addChild(this.mark);
  }
  setVisible(v: boolean): void { this.root.visible = v; if (v) this.t = 0; }
  update(dt: number): void {
    if (!this.root.visible) return;
    this.t += dt;
    // 상하 바운스, 주기 ~600ms
    this.mark.y = Math.round(Math.sin(this.t / 600 * Math.PI * 2) * 2 - 2);
  }
  destroy(): void { this.root.destroy({ children: true }); }
}
```

### 4.5 클릭 히트테스트 & 이벤트 배선

- `sprite.eventMode = 'static'` + `pointertap` → Pixi v8 히트영역(스프라이트 bounds). 16×16 스프라이트는 작으니, 필요 시 `sprite.hitArea = new Rectangle(-8,-16,16,16)`로 명시 확대.
- `OfficeWorld`가 각 엔티티의 `onClicked`를 `bus.emitAgentClicked(id)`에 연결.

### 4.6 월드 관리자 (`world/OfficeWorld.ts`)

A/C 이벤트를 엔티티에 반영하고, 프로필 목록 diff로 엔티티 생성/삭제.

```ts
// src/renderer/office/world/OfficeWorld.ts
import { Container } from 'pixi.js';
import type { OfficeBus } from '../bus';
import type { AgentProfile } from '../types';
import { OfficeMap } from '../map/mapData';
import { assignDesks } from '../map/deskAssignment';
import { createCharacterAssets } from '../gen/characterFactory';
import { CharacterEntity } from '../entities/CharacterEntity';
import { mulberry32, hashStringToSeed } from '../gen/prng';

export interface OfficeWorldOptions {
  bus: OfficeBus; characterLayer: Container; overlayLayer: Container; map: OfficeMap;
}
export class OfficeWorld {
  private entities = new Map<string, CharacterEntity>();
  private unsub: Array<() => void> = [];
  constructor(private o: OfficeWorldOptions) {
    this.unsub.push(o.bus.onNotificationChanged((id, has) => this.entities.get(id)?.setPending(has)));
    this.unsub.push(o.bus.onSessionStateChanged((id, st) => {/* MVP: 상태별 색/애니 후행 확장 */}));
  }
  syncAgents(profiles: readonly AgentProfile[]): void {
    const desks = assignDesks(this.o.map, profiles.map(p => p.id));
    const next = new Set(profiles.map(p => p.id));
    // 삭제
    for (const [id, e] of this.entities) if (!next.has(id)) { e.destroy(); this.entities.delete(id); }
    // 추가
    for (const p of profiles) {
      if (this.entities.has(p.id)) continue;
      const slot = desks.get(p.id); if (!slot) continue; // 좌석 부족 시 스킵(후행: 대기 배회)
      const assets = createCharacterAssets(p);
      const rand = mulberry32(hashStringToSeed(p.id) ^ 0x9e3779b9); // 거동용 별도 스트림
      const e = new CharacterEntity(p.id, assets, slot.seat, this.o.map, rand);
      e.onClicked(id => this.o.bus.emitAgentClicked(id));
      this.o.characterLayer.addChild(e.root);
      this.entities.set(p.id, e);
    }
  }
  update(dt: number): void { for (const e of this.entities.values()) e.update(dt); }
  destroy(): void { this.unsub.forEach(u => u()); for (const e of this.entities.values()) e.destroy(); this.entities.clear(); }
}
```

### 4.7 타입 계약 (`types.ts`)

```ts
// src/renderer/office/types.ts
export interface AgentProfile {
  id: string;
  name: string;
  role: string;
  seed: string;      // 생성 시드 (없으면 id로 대체)
  [k: string]: unknown;
}
```

---

## 5. 결정성 & 테스트 (vitest)

핵심: `gen/`은 순수. `defaultCanvasFactory`를 테스트에서 **@napi-rs/canvas 기반 팩토리**로 주입. `getImageData` 기반 픽셀 비교로 검증.

```ts
// src/renderer/office/__tests__/characterFactory.test.ts
import { describe, it, expect } from 'vitest';
import { generateSheet, selectLayers } from '../gen/characterFactory';
import { contrastRatio } from '../gen/palette';
import { createTestCanvasFactory, sheetToPixels } from './helpers';

describe('character generator determinism', () => {
  it('same seed → identical pixels', () => {
    const f = createTestCanvasFactory();
    const a = sheetToPixels(generateSheet('agent-alpha', f).sheet);
    const b = sheetToPixels(generateSheet('agent-alpha', f).sheet);
    expect(a).toEqual(b);                       // Uint8ClampedArray 동일
  });

  it('different seed → different pixels (very high probability)', () => {
    const f = createTestCanvasFactory();
    const a = sheetToPixels(generateSheet('agent-alpha', f).sheet);
    const b = sheetToPixels(generateSheet('agent-omega', f).sheet);
    expect(a).not.toEqual(b);
  });

  it('layer selection is deterministic and in-range', () => {
    const s1 = selectLayers('seed-123').descriptor;
    const s2 = selectLayers('seed-123').descriptor;
    expect(s1).toEqual(s2);
    expect(['short','bob','spiky','bald']).toContain(s1.hair);
  });

  it('palette contrast: shirt vs skin >= 1.6', () => {
    for (const seed of ['a','b','c','d','e','f','g','h']) {
      const { pal } = selectLayers(seed);
      expect(contrastRatio(pal.shirt.base, pal.skin.base)).toBeGreaterThanOrEqual(1.6);
    }
  });

  it('outline present & non-empty frame (character actually drawn)', () => {
    const f = createTestCanvasFactory();
    const px = sheetToPixels(generateSheet('seed-x', f).sheet);
    // 알파 채널이 0이 아닌 픽셀이 최소 N개 이상 (빈 시트 아님)
    let opaque = 0; for (let i = 3; i < px.length; i += 4) if (px[i] > 0) opaque++;
    expect(opaque).toBeGreaterThan(60);
  });
});
```

```ts
// src/renderer/office/__tests__/deskAssignment.test.ts
import { describe, it, expect } from 'vitest';
import { assignDesks } from '../map/deskAssignment';
import { OFFICE_MAP } from '../map/mapData';

describe('desk assignment', () => {
  it('is order-independent and stable', () => {
    const ids = ['x','y','z','p','q'];
    const a = assignDesks(OFFICE_MAP, ids);
    const b = assignDesks(OFFICE_MAP, [...ids].reverse());
    for (const id of ids) expect(a.get(id)!.index).toBe(b.get(id)!.index);
  });
  it('assigns unique seats without collision', () => {
    const ids = Array.from({ length: OFFICE_MAP.desks.length }, (_, i) => `a${i}`);
    const m = assignDesks(OFFICE_MAP, ids);
    const seats = new Set([...m.values()].map(d => d.index));
    expect(seats.size).toBe(ids.length);
  });
});
```

```ts
// src/renderer/office/__tests__/behaviorFsm.test.ts
import { describe, it, expect } from 'vitest';
import { stepBehavior } from '../entities/behaviorFsm';

describe('behavior fsm', () => {
  it('pending notification keeps character sitting', () => {
    const r = stepBehavior('sitting', { atDesk: true, hasPending: true, timerMs: 999999, rand: 0 }, 16);
    expect(r.next).toBe('sitting');
  });
  it('idleWander returns to desk after timeout', () => {
    const r = stepBehavior('idleWander', { atDesk: false, hasPending: false, timerMs: 5000, rand: 0.9 }, 16);
    expect(r.next).toBe('walking');
    expect(r.requestReturnToDesk).toBe(true);
  });
  it('does not wander before minimum sit time', () => {
    const r = stepBehavior('sitting', { atDesk: true, hasPending: false, timerMs: 100, rand: 0 }, 16);
    expect(r.next).toBe('sitting');
  });
});
```

테스트 헬퍼(`__tests__/helpers.ts`)는 `@napi-rs/canvas`(순수 네이티브·빠름)로 `CanvasFactory`를 구현하고 `getContext('2d').getImageData(...)`로 픽셀을 뽑는다. vitest 환경은 `node`(Pixi 미로딩 경로만 테스트)로 두어 GPU 의존 제거.

---

## 6. 구현 작업 분해 (순서대로, 독립 테스트 가능)

**T1 — PRNG & 팔레트 코어** · `gen/prng.ts`, `gen/palette.ts`
- 산출: `hashStringToSeed`, `mulberry32`, `makeRng`, `generatePalette`, `contrastRatio`, 대비 최종 클램프.
- 검증: `__tests__/prng.test.ts`(동일 시드 동일 시퀀스, 분포 sanity), `palette.test.ts`(대비 ≥1.6, 램프 명도 순서).

**T2 — 파트 데이터 & 합성기** · `gen/parts.ts`, `gen/compositor.ts`
- 산출: 픽셀 배열들, `composeSpriteSheet`(순수, `CanvasFactory` 주입), `resolveChar`.
- 검증: 시트 픽셀 opaque 수/투명 처리 단위 테스트.

**T3 — 캐릭터 팩토리** · `gen/characterFactory.ts` + `__tests__/helpers.ts`
- 산출: `selectLayers`, `generateSheet`, `createCharacterAssets`(Pixi Texture), `generateSpritePreview`(dataURL).
- 검증: `characterFactory.test.ts`(§5 전체 — 결정성/차이/대비/비어있지 않음).

**T4 — 맵 데이터 & 데스크 배정** · `map/mapData.ts`, `map/deskAssignment.ts`
- 산출: `OFFICE_MAP`, `deriveDesks`, `assignDesks`.
- 검증: `deskAssignment.test.ts`(순서 독립·무충돌).

**T5 — 타일 렌더러** · `map/TileRenderer.ts`
- 산출: `build`(정적 캐시), `buildFurniture`(y-sort).
- 검증: 시각(수동).

**T6 — 씬 골격 & React 브리지** · `OfficeScene.ts`, `useOfficeScene.ts`, `OfficeCanvas.tsx`, `bus.ts`, `types.ts`
- 산출: Application init/resize/destroy, 카메라 정수 스케일, 레이어, 훅.
- 검증: 빈 오피스(맵만) 렌더 수동 확인, StrictMode 이중 마운트 무누수(destroy 호출 확인).

**T7 — 거동 FSM** · `entities/behaviorFsm.ts`, `world/pathing.ts`
- 산출: `stepBehavior`, 좌표 변환/배회 타겟.
- 검증: `behaviorFsm.test.ts`.

**T8 — 캐릭터 엔티티 & 오버레이** · `entities/CharacterEntity.ts`, `entities/ExclamationOverlay.ts`
- 산출: 스프라이트 애니, 이동 보간, flip, 클릭 배선, 느낌표 바운스.
- 검증: 수동(캐릭터 착석·간헐 배회·클릭 로그·느낌표 토글).

**T9 — 월드 통합** · `world/OfficeWorld.ts`
- 산출: 프로필 diff 생성/삭제, 버스 구독(`onNotificationChanged`→`setPending`), `emitAgentClicked` 배선.
- 검증: 목 버스로 알림 토글→느낌표, 클릭→이벤트 방출 통합 테스트.

**T10 — 마감/후행 확장 훅**
- `sessionStateChanged` 시각화(상태별 틴트/작은 아이콘), 좌석 부족 시 대기 배회, 카메라 팬 활성화. 전부 기존 시그니처 무변경 확장.

---

## 핵심 결정 요약

- 카메라: **고정 + 정수 스케일**(픽셀 선명도), 팬은 시그니처만 남긴 후행 확장.
- 타일: **절차적 코드 드로잉 + 1px 디테일**, 정적 바닥은 `cacheAsTexture`로 1장 캐시, 데스크는 y-sort 개별 스프라이트.
- 캐릭터: 16×16, 파트=문자 픽셀 배열(사람이 편집 가능), 레이어 순서 body→clothes→hair→accessory, idle 2 + walk 2 프레임(bob offset + 다리 스왑), `nearest` 텍스처.
- 결정성: `profile.seed → xfnv1a → mulberry32`가 팔레트·파트·좌석까지 지배. `gen/`은 Pixi/DOM 비의존 순수 코드(캔버스 팩토리 주입)로 vitest 가능.
- 경계: `OfficeBus` 인터페이스 하나로 A/C와 격리 — B는 `onNotificationChanged/onSessionStateChanged` 구독, `emitAgentClicked` 방출만.
