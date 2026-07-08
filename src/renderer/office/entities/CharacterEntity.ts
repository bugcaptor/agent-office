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
import { OfficeMap, TILE_SIZE } from "../map/mapData";
import { GridPos, pickBreakTarget, tileCenterPx } from "../world/pathing";
import { BehaviorState, stepBehavior } from "./behaviorFsm";
import { ExclamationOverlay } from "./ExclamationOverlay";
import { ThinkingOverlay } from "./ThinkingOverlay";

const WALK_SPEED = 28; // px/sec
const ANIM_IDLE_MS = 480; // idle frame swap period
const ANIM_WALK_MS = 140;
const ARRIVE_EPS_PX = 0.5;
const APPARENT_CELL = 16; // 겉보기 셀 크기(px). 실제 텍스처 셀 N을 이 크기로 스케일.

// "..." 생각 말풍선 점멸 주기(캐릭터마다 seeded rand로 편차를 둔다).
const THINK_HIDDEN_MIN_MS = 4000;
const THINK_HIDDEN_MAX_MS = 7000;
const THINK_VISIBLE_MIN_MS = 2000;
const THINK_VISIBLE_MAX_MS = 2500;

type TargetKind = "seat" | "break";

export class CharacterEntity {
  readonly root = new Container(); // added to the scene's sortable layer; zIndex = worldY
  private sprite: Sprite;
  private overlay: ExclamationOverlay;
  private thinkOverlay: ThinkingOverlay;
  private state: BehaviorState = "sitting";
  private stateTimer = 0;
  private animTimer = 0;
  private frameIdx = 0;
  private targetPx: { x: number; y: number } | null = null;
  private targetKind: TargetKind | null = null;
  private hasPending = false;
  private sessionActive = false;
  private spriteScale = 1; // 16 / cellSize, 좌우 반전과 결합할 배율 크기.

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

    // Seated placement.
    const p = tileCenterPx(seat);
    this.root.position.set(p.x, p.y + TILE_SIZE / 2); // feet at the seat tile's bottom edge
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

  /** dt: ms, ticker-supplied (never a real timer / Date.now internally). */
  update(dt: number): void {
    this.stateTimer += dt;
    const r = this.rand();
    const res = stepBehavior(
      this.state,
      {
        hasPending: this.hasPending,
        sessionActive: this.sessionActive,
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
    // the seat immediately rather than finishing the walk.
    if (this.state === "walking" && this.targetKind === "break" && (this.sessionActive || this.hasPending)) {
      this.setSeatTarget();
    }

    if (this.targetPx) this.moveToward(dt);
    this.animate(dt);
    this.updateThinkingOverlay(dt);
    this.root.zIndex = this.root.y; // y-sort refresh
    this.overlay.update(dt);

    // Belt-and-suspenders: never let a character leave the map rect.
    const mapPxW = this.map.width * TILE_SIZE;
    const mapPxH = this.map.height * TILE_SIZE;
    this.root.x = Math.min(Math.max(this.root.x, 0), mapPxW);
    this.root.y = Math.min(Math.max(this.root.y, 0), mapPxH);
  }

  destroy(): void {
    this.overlay.destroy();
    this.thinkOverlay.destroy();
    this.root.destroy({ children: true });
  }

  private setSeatTarget(): void {
    const p = tileCenterPx(this.seat);
    this.targetPx = { x: p.x, y: p.y + TILE_SIZE / 2 };
    this.targetKind = "seat";
  }

  /** Returns false (and leaves targetPx/targetKind untouched) if no reachable break-room tile was found. */
  private setBreakTarget(): boolean {
    const p = pickBreakTarget(this.map, this.rand);
    if (!p) return false;
    this.targetPx = { x: p.x, y: p.y + TILE_SIZE / 2 };
    this.targetKind = "break";
    return true;
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
      // Arrival: walking ends -> sitting for the seat leg, breakIdle for a
      // break-room leg (initial or a stroll).
      this.state = this.targetKind === "seat" ? "sitting" : "breakIdle";
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
