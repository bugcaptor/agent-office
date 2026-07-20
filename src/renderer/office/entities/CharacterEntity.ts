// src/renderer/office/entities/CharacterEntity.ts
//
// Pixi display object + behavior/movement/animation owner for a single
// character.
//
// Wires together:
// - `gen/characterFactory.ts`: pre-baked idle/walk textures.
// - `map/mapData.ts`: tile size, walkability grid.
// - `entities/behaviorFsm.ts` + `world/pathing.ts`: pure state
//   transition + grid<->pixel/break-target helpers, both injected with a
//   `rand: () => number` seam (no `Math.random`) and driven by an
//   externally-supplied `dt` (no real timers) so this class stays testable
//   with a fake Pixi-free-ish setup (real `Container`/`Sprite`/`Graphics`,
//   since those construct fine without a live renderer — only `Application`
//   needs one).
//
// Manual frame swap (not `AnimatedSprite`): only 2 frames per state, so a
// plain texture swap on a timer is simpler and keeps animation speed control
// (different periods for idle vs walk) trivial.
//
// Behavior (Phase B office redesign): a character sits at its desk only
// while its session is "active" (starting/running, per `setSessionActive`)
// or a notification is pending; otherwise it gets up (after a short linger)
// and hangs out in the break room, occasionally strolling to another spot
// in it, until the session re-activates (or a notification arrives), at
// which point it heads straight back to its desk — even pre-empting an
// in-progress stroll toward the break room.

import { Container, Rectangle, Sprite } from "pixi.js";

import type { CharacterAssets } from "../gen/characterFactory";
import { OfficeMap, QUEUE_SLOTS, TILE_SIZE } from "../map/mapData";
import { GridPos, pickBreakTarget, tileCenterPx, tileKey } from "../world/pathing";
import { BehaviorState, stepBehavior } from "./behaviorFsm";
import { ExclamationOverlay } from "./ExclamationOverlay";
import { MiniAgentsOverlay } from "./MiniAgentsOverlay";
import { ThinkingOverlay } from "./ThinkingOverlay";

const WALK_SPEED = 28; // px/sec
const ANIM_IDLE_MS = 480; // idle frame swap period
const ANIM_WALK_MS = 140;
const ARRIVE_EPS_PX = 0.5;
const APPARENT_CELL = 16; // 겉보기 셀 크기(px). 실제 텍스처 셀 N을 이 크기로 스케일.

/** 착석 시 발 위치를 좌석 타일 하단보다 이만큼 내린다(px). 좌석이 책상
 * *위쪽* 타일이므로(맵 데이터 계약), 남쪽의 책상(zIndex = 책상 하단 y)이
 * 캐릭터 다리를 가려 "책상 뒤에 앉은" 모습이 된다. */
export const SEAT_SINK_PX = 6;

// "..." 생각 말풍선 점멸 주기(캐릭터마다 seeded rand로 편차를 둔다).
const THINK_HIDDEN_MIN_MS = 4000;
const THINK_HIDDEN_MAX_MS = 7000;
const THINK_VISIBLE_MIN_MS = 2000;
const THINK_VISIBLE_MAX_MS = 2500;

type TargetKind = "seat" | "break" | "queue";

export class CharacterEntity {
  readonly root = new Container(); // added to the scene's sortable layer; zIndex = worldY
  private sprite: Sprite;
  private overlay: ExclamationOverlay;
  private thinkOverlay: ThinkingOverlay;
  private miniOverlay: MiniAgentsOverlay;
  private state: BehaviorState = "sitting";
  private stateTimer = 0;
  private animTimer = 0;
  private frameIdx = 0;
  private targetPx: { x: number; y: number } | null = null;
  private targetKind: TargetKind | null = null;
  private hasPending = false;
  private sessionActive = false;
  private spriteScale = 1; // 16 / cellSize, 좌우 반전과 결합할 배율 크기.
  // 이 캐릭터가 breakReservations에 넣어 둔 탕비실 타일 키(예약 중이면 non-null).
  private reservedBreakKey: string | null = null;
  // 보스 책상 줄 슬롯(월드가 배정). null = 줄에 서 있지 않음.
  private queueSlot: number | null = null;

  // "..." 말풍선 점멸 사이클 상태 (dt 누적 기반, Date.now 사용 안 함).
  private thinkHidden = true;
  private thinkPhaseTimer = 0;
  private thinkPhaseDurationMs = 0; // 0 = 다음 평가 시 새로 굴려야 함

  constructor(
    readonly agentId: string,
    private assets: CharacterAssets,
    private seat: GridPos,
    private map: OfficeMap,
    private rand: () => number,
    // 월드가 소유한 탕비실 타일 예약 집합(tileKey). 모든 캐릭터가 공유해
    // 쉬는 타일이 겹치지 않게 한다. 미주입 시(단독 테스트 등) 예약 없이 동작.
    private breakReservations?: Set<string>,
  ) {
    this.sprite = new Sprite(assets.idle[0]);
    this.spriteScale = APPARENT_CELL / this.assets.cellSize;
    this.sprite.scale.set(this.spriteScale); // 텍스처 셀 N -> 겉보기 16px
    this.sprite.anchor.set(0.5, 1); // feet-aligned, for y-sort
    this.root.addChild(this.sprite);

    this.overlay = new ExclamationOverlay();
    this.overlay.root.position.set(0, -TILE_SIZE); // above the head
    this.root.addChild(this.overlay.root);
    this.overlay.setVisible(false);

    this.thinkOverlay = new ThinkingOverlay();
    this.thinkOverlay.root.position.set(0, -TILE_SIZE - 2); // a couple px higher than the "!" badge
    this.root.addChild(this.thinkOverlay.root);
    this.thinkOverlay.setVisible(false);

    this.miniOverlay = new MiniAgentsOverlay(this.assets.idle[0], this.spriteScale);
    this.miniOverlay.root.position.set(0, -TILE_SIZE); // 머리 위(기존 오버레이와 동일 높이)
    this.root.addChild(this.miniOverlay.root);

    // Seated placement (feet sunk toward the desk so it overlaps the legs).
    const p = tileCenterPx(seat);
    this.root.position.set(p.x, p.y + TILE_SIZE / 2 + SEAT_SINK_PX);
    this.root.zIndex = this.root.y;

    // Click hit test: eventMode + explicit hit area (the 16x16 sprite is
    // small, so widen it a touch beyond the anchor-derived default bounds).
    // NOTE: Pixi v8 hitArea is evaluated in the sprite's *local* (pre-scale)
    // coordinate space (EventBoundary.hitPruneFn applies the inverse world
    // transform, i.e. undoes `sprite.scale`, before calling `contains`). So
    // this must be sized to the texture's own cell size N (not the apparent
    // 16px on-screen size) or a high-res custom sheet (cellSize=N>16, scaled
    // down by `spriteScale` = 16/N) would end up with a ~1x1px on-screen
    // click/hover target.
    const n = this.assets.cellSize;
    this.sprite.eventMode = "static";
    this.sprite.cursor = "pointer";
    this.sprite.hitArea = new Rectangle(-n / 2, -n, n, n);
  }

  /** 현재 렌더 셀 크기(텍스처 셀 px). 커스텀 시트 S-적응 재생성 판단용. */
  get cellSize(): number {
    return this.assets.cellSize;
  }

  /**
   * 외형은 그대로 두고 텍스처 에셋만 교체한다(커스텀 고해상 시트를 카메라 정수
   * 스케일 S에 맞춰 재프리필터할 때 — 이슈 #47). FSM/이동/애니메이션 상태를
   * 보존하므로 창 리사이즈로 S가 바뀌어도 캐릭터가 튀거나 초기화되지 않는다.
   */
  replaceAssets(next: CharacterAssets): void {
    const prev = this.assets;
    this.assets = next;
    this.spriteScale = APPARENT_CELL / next.cellSize;
    const flip = this.sprite.scale.x < 0 ? -1 : 1;
    this.sprite.scale.set(flip * this.spriteScale, this.spriteScale);
    // 진행 중이던 상태의 현재 프레임을 새 텍스처 셋에서 다시 집는다.
    const frames = this.state === "walking" ? next.walk : next.idle;
    this.sprite.texture = frames[this.frameIdx % frames.length];
    // 히트 영역은 텍스처 셀(local, pre-scale) 기준이라 새 cellSize를 따른다.
    const n = next.cellSize;
    this.sprite.hitArea = new Rectangle(-n / 2, -n, n, n);
    this.miniOverlay.setBase(next.idle[0], this.spriteScale);
    prev.dispose?.();
  }

  onClicked(cb: (id: string) => void): void {
    this.sprite.on("pointertap", () => cb(this.agentId));
  }

  /** pointerover -> (agentId, 화면좌표), pointerout -> (null, 0, 0). */
  onHover(cb: (id: string | null, x: number, y: number) => void): void {
    this.sprite.on("pointerover", (e) => cb(this.agentId, e.global.x, e.global.y));
    this.sprite.on("pointerout", () => cb(null, 0, 0));
  }

  setPending(v: boolean): void {
    // 착석 고정 조건이 풀리는 순간부터 SIT_LINGER_MS를 새로 재기 시작해야
    // 한다 — 리셋하지 않으면 누적된 stateTimer 때문에 즉시 일어나 버린다.
    if (this.state === "sitting" && this.hasPending && !v) this.stateTimer = 0;
    this.hasPending = v;
    this.overlay.setVisible(v);
    // Priority: the exclamation badge always wins over the thinking bubble.
    if (v) this.thinkOverlay.setVisible(false);
  }

  setSessionActive(v: boolean): void {
    if (this.state === "sitting" && this.sessionActive && !v) this.stateTimer = 0;
    this.sessionActive = v;
  }

  /** 활성 서브에이전트 수 반영(0~3 미니 표시). */
  setSubagentCount(n: number): void {
    this.miniOverlay.setCount(n);
  }

  /** 좌석(책상 지정) 변경. 앉아 있거나 자리로 걸어가는 중이면 즉시 새
   * 좌석으로 걸어간다. 휴식 중이면 그대로 두고, 복귀 시점에 새 좌석을 쓴다. */
  setSeat(seat: GridPos): void {
    if (seat.tx === this.seat.tx && seat.ty === this.seat.ty) return;
    this.seat = seat;
    if (this.state === "sitting" || (this.state === "walking" && this.targetKind === "seat")) {
      this.setSeatTarget();
      this.state = "walking";
      this.stateTimer = 0;
    }
  }

  /** 보스 책상 줄 슬롯 배정/해제(월드가 호출). null = 줄에서 빠짐. */
  setQueueSlot(slot: number | null): void {
    if (slot === this.queueSlot) return;
    this.queueSlot = slot;
    if (slot !== null) {
      this.setQueueTarget(slot);
      this.state = "walking";
      this.stateTimer = 0;
    } else if (this.state === "queueing" || (this.state === "walking" && this.targetKind === "queue")) {
      // 이동 중 해제도 즉시 자리로 리타깃 — FSM은 queueing 상태에서만 복귀를 처리한다.
      this.setSeatTarget();
      this.state = "walking";
      this.stateTimer = 0;
    }
  }

  /** dt: ms, ticker-supplied (never a real timer / Date.now internally). */
  update(dt: number): void {
    this.stateTimer += dt;
    const r = this.rand();
    const res = stepBehavior(
      this.state,
      {
        hasPending: this.hasPending,
        sessionActive: this.sessionActive,
        shouldQueue: this.queueSlot !== null,
        timerMs: this.stateTimer,
        rand: r,
      },
      dt,
    );

    if (res.next !== this.state) {
      let accepted = true;
      if (res.requestBreakTarget || res.requestBreakWander) {
        accepted = this.setBreakTarget();
      } else if (res.requestReturnToDesk) {
        this.setSeatTarget();
      } else if (res.requestQueueSlot) {
        this.setQueueTarget(this.queueSlot!); // shouldQueue=true일 때만 오므로 non-null
      }
      if (accepted) {
        this.state = res.next;
        this.stateTimer = 0;
      }
      // else: no reachable break-room tile (e.g. a misconfigured/tiny map)
      // — stay in the current state and let a future tick retry.
    }

    // Responsiveness: a session activating (or a notification arriving)
    // mid-stroll toward the break room pre-empts the stroll — retarget to
    // the seat (or the queue slot, if one is assigned) immediately rather
    // than finishing the walk.
    if (this.state === "walking" && this.targetKind === "break" && (this.sessionActive || this.hasPending)) {
      if (this.queueSlot !== null) this.setQueueTarget(this.queueSlot);
      else this.setSeatTarget();
    }

    if (this.targetPx) this.moveToward(dt);
    this.animate(dt);
    this.updateThinkingOverlay(dt);
    this.root.zIndex = this.root.y; // y-sort refresh
    this.overlay.update(dt);
    this.miniOverlay.update(dt);

    // Belt-and-suspenders: never let a character leave the map rect.
    const mapPxW = this.map.width * TILE_SIZE;
    const mapPxH = this.map.height * TILE_SIZE;
    this.root.x = Math.min(Math.max(this.root.x, 0), mapPxW);
    this.root.y = Math.min(Math.max(this.root.y, 0), mapPxH);
  }

  destroy(): void {
    this.releaseBreakTile();
    this.overlay.destroy();
    this.thinkOverlay.destroy();
    this.miniOverlay.destroy();
    this.root.destroy({ children: true });
    this.assets.dispose?.(); // 커스텀 다운스케일 프레임 텍스처/소스 해제(누수 방지)
  }

  private setSeatTarget(): void {
    this.releaseBreakTile();
    const p = tileCenterPx(this.seat);
    this.targetPx = { x: p.x, y: p.y + TILE_SIZE / 2 + SEAT_SINK_PX };
    this.targetKind = "seat";
  }

  /** Returns false (and leaves targetPx/targetKind untouched) if no reachable break-room tile was found. */
  private setBreakTarget(): boolean {
    const p = pickBreakTarget(this.map, this.rand, this.breakReservations);
    if (!p) return false;
    // 도착 전(목적지 선정 시점)부터 예약해, 같은 타일로 동시에 걸어가는
    // 경합까지 막는다. 이전 예약(산책 전 타일)은 이때 해제.
    this.releaseBreakTile();
    this.reservedBreakKey = tileKey(Math.floor(p.x / TILE_SIZE), Math.floor(p.y / TILE_SIZE));
    this.breakReservations?.add(this.reservedBreakKey);
    this.targetPx = { x: p.x, y: p.y + TILE_SIZE / 2 };
    this.targetKind = "break";
    return true;
  }

  private setQueueTarget(slot: number): void {
    this.releaseBreakTile();
    const p = tileCenterPx(QUEUE_SLOTS[slot]);
    this.targetPx = { x: p.x, y: p.y + TILE_SIZE / 2 };
    this.targetKind = "queue";
  }

  private releaseBreakTile(): void {
    if (this.reservedBreakKey === null) return;
    this.breakReservations?.delete(this.reservedBreakKey);
    this.reservedBreakKey = null;
  }

  private moveToward(dt: number): void {
    const t = this.targetPx!;
    const dx = t.x - this.root.x;
    const dy = t.y - this.root.y;
    const dist = Math.hypot(dx, dy);
    const step = (WALK_SPEED * dt) / 1000;
    if (dist <= step || dist < ARRIVE_EPS_PX) {
      this.root.position.set(Math.round(t.x), Math.round(t.y));
      this.targetPx = null;
      // Arrival: walking ends -> sitting for the seat leg, queueing for the
      // boss-desk queue leg, breakIdle for a break-room leg (initial or a
      // stroll).
      this.state = this.targetKind === "seat" ? "sitting" : this.targetKind === "queue" ? "queueing" : "breakIdle";
      this.targetKind = null;
      this.stateTimer = 0;
      return;
    }
    this.sprite.scale.x = dx < 0 ? -this.spriteScale : this.spriteScale; // 진행 방향으로 반전(배율 유지)
    this.root.x += (dx / dist) * step;
    this.root.y += (dy / dist) * step;
  }

  /** dt-accumulator driven "..." bubble cadence: hidden ~4-7s, visible ~2-2.5s, while sitting+active+!pending. */
  private updateThinkingOverlay(dt: number): void {
    const gate = this.state === "sitting" && this.sessionActive && !this.hasPending;
    if (!gate) {
      this.thinkOverlay.setVisible(false);
      this.thinkHidden = true;
      this.thinkPhaseTimer = 0;
      this.thinkPhaseDurationMs = 0; // force a fresh roll next time the gate opens
      return;
    }

    this.thinkPhaseTimer += dt;
    if (this.thinkPhaseDurationMs <= 0) {
      this.thinkPhaseDurationMs = this.randRange(THINK_HIDDEN_MIN_MS, THINK_HIDDEN_MAX_MS);
    }
    if (this.thinkPhaseTimer >= this.thinkPhaseDurationMs) {
      this.thinkPhaseTimer = 0;
      this.thinkHidden = !this.thinkHidden;
      this.thinkOverlay.setVisible(!this.thinkHidden);
      this.thinkPhaseDurationMs = this.thinkHidden
        ? this.randRange(THINK_HIDDEN_MIN_MS, THINK_HIDDEN_MAX_MS)
        : this.randRange(THINK_VISIBLE_MIN_MS, THINK_VISIBLE_MAX_MS);
    }
    this.thinkOverlay.update(dt);
  }

  private randRange(min: number, max: number): number {
    return min + this.rand() * (max - min);
  }

  private animate(dt: number): void {
    const walking = this.state === "walking";
    const frames = walking ? this.assets.walk : this.assets.idle;
    this.animTimer += dt;
    const period = walking ? ANIM_WALK_MS : ANIM_IDLE_MS;
    if (this.animTimer >= period) {
      this.animTimer = 0;
      this.frameIdx = (this.frameIdx + 1) % frames.length;
    }
    this.sprite.texture = frames[this.frameIdx % frames.length];
  }
}
