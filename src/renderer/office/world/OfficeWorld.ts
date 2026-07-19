// src/renderer/office/world/OfficeWorld.ts
//
// World manager: reflects A/C bus events onto entities, and diff-syncs the
// live entity set against the current profile list.
//
// This is the integration point for everything built in earlier tasks:
// - `map/deskAssignment.ts` (3D): deterministic agentId -> seat.
// - `gen/characterFactory.ts` (3C): profile -> Pixi textures.
// - `entities/CharacterEntity.ts` (3G): the per-agent display object, which
//   owns its own FSM/movement/animation/click-hookup/overlay.
// - `bus.ts` (3E): the `OfficeBus` contract — this is the *only* thing B
//   depends on from A/C; no scene-level addAgent/removeAgent/setPending
//   surface exists (frozen) because `syncAgents` below is the single
//   entry point C ever needs to drive.
//
// Determinism (binding requirement): seating comes from `assignDesks`
// (hash-based, order-independent); each entity's movement `rand` stream is
// seeded from `profile.id` via `hashStringToSeed`/`mulberry32` — no
// `Math.random` anywhere in this file.
import type { Container } from "pixi.js";

import type { LabelAnchor, OfficeBus } from "../bus";
import type { AgentProfile } from "../types";
import type { SessionState } from "../../../shared/types";
import type { OfficeMap } from "../map/mapData";
import { assignDesks } from "../map/deskAssignment";
import { createCharacterAssets } from "../gen/characterFactory";
import { detailCellSize } from "../gen/spriteResample";
import { CELL } from "../gen/compositor";
import { getSpriteOverride } from "../gen/spriteOverrides";
import { CharacterEntity } from "../entities/CharacterEntity";
import { mulberry32, hashStringToSeed } from "../gen/prng";

export interface OfficeWorldOptions {
  bus: OfficeBus;
  characterLayer: Container; // sortable layer entities are parented into (zIndex = worldY)
  overlayLayer: Container; // reserved for future non-per-entity overlays; unused for now
  map: OfficeMap;
}

/** Distinct XOR salt so an entity's movement RNG stream never collides with
 * anything else derived from the same agentId (e.g. desk-assignment hashing,
 * which reuses `hashStringToSeed(id)` directly with no salt). */
const MOVEMENT_RNG_SALT = 0x9e3779b9;

/** 라벨 앵커의 머리 위 오프셋(월드 px). ExclamationOverlay(-TILE_SIZE)보다 살짝 위. */
const LABEL_ANCHOR_OFFSET_Y = 20;

/** 엔티티 외형을 결정하는 키 — 바뀌면 재생성한다. archetype, seed 편집, 커스텀 시트
 * 등록/변경/해제(spriteUpdatedAt + 오버라이드 유무) 모두 반영. */
export function appearanceKey(p: AgentProfile): string {
  return `${p.archetype ?? "human"}|${p.seed}|${p.spriteUpdatedAt ?? 0}|${getSpriteOverride(p.id) ? 1 : 0}`;
}

/** "starting"/"running" = actively working (character sits at its desk); "exited"/"disposed" = inactive (heads to the break room). */
function isSessionActive(state: SessionState): boolean {
  return state === "starting" || state === "running";
}

/** 커스텀 시트 프리필터 기본 렌더 스케일. 카메라가 측정되기 전(init~첫 resize
 * 사이)의 폴백 — 대개 곧바로 setRenderScale로 실제 정수 S가 들어온다. */
const DEFAULT_RENDER_SCALE = 3;

export class OfficeWorld {
  private entities = new Map<string, CharacterEntity>();
  private appearanceKeys = new Map<string, string>();
  private sessionActive = new Map<string, boolean>();
  private subagentCounts = new Map<string, number>();
  // 라이브 프로필 스냅샷 — setRenderScale이 커스텀 엔티티를 재프리필터할 때 필요.
  private profiles = new Map<string, AgentProfile>();
  // 카메라 정수 스케일 S. 커스텀 시트를 D=min(N,16·S)로 프리필터하는 기준.
  private renderScale = DEFAULT_RENDER_SCALE;
  // 탕비실 타일 예약(tileKey) — 전 엔티티 공유. 쉬는 캐릭터가 같은 타일에
  // 겹쳐 서지 않게 한다. 엔티티가 예약/해제하고(destroy 포함) 여기서는 소유만.
  private breakReservations = new Set<string>();
  private unsub: Array<() => void> = [];

  constructor(private o: OfficeWorldOptions) {
    this.unsub.push(o.bus.onNotificationChanged((id, hasPending) => this.entities.get(id)?.setPending(hasPending)));
    this.unsub.push(
      o.bus.onSessionStateChanged((agentId, state) => {
        const active = isSessionActive(state);
        this.sessionActive.set(agentId, active);
        this.entities.get(agentId)?.setSessionActive(active);
      }),
    );
    this.unsub.push(
      o.bus.onSubagentCountChanged((agentId, count) => {
        this.subagentCounts.set(agentId, count);
        this.entities.get(agentId)?.setSubagentCount(count);
      }),
    );
  }

  /** Diff the live entity set against `profiles`: destroy dropped agents, create new ones,
   * recreate agents whose appearance key changed, leave the rest untouched. */
  syncAgents(profiles: readonly AgentProfile[]): void {
    // 수동 지정(assignedDeskIndex)은 우선 배정되고, 지정된 책상은 자동
    // 배정 풀에서 빠진다 — 지정된 적 없는 책상만 자동 선점 대상.
    const manual = new Map<string, number>();
    for (const p of profiles) {
      if (typeof p.assignedDeskIndex === "number") manual.set(p.id, p.assignedDeskIndex);
    }
    const desks = assignDesks(this.o.map, profiles.map((p) => p.id), manual);
    const next = new Set(profiles.map((p) => p.id));
    // setRenderScale 재프리필터가 참조할 최신 프로필 스냅샷.
    this.profiles = new Map(profiles.map((p) => [p.id, p]));

    for (const [id, entity] of this.entities) {
      if (next.has(id)) continue;
      entity.destroy();
      this.entities.delete(id);
      this.appearanceKeys.delete(id);
      this.sessionActive.delete(id);
      this.subagentCounts.delete(id);
    }

    // 외형 키가 바뀐 기존 엔티티는 파괴해 아래 생성 루프에서 재생성한다.
    for (const p of profiles) {
      const entity = this.entities.get(p.id);
      if (!entity) continue;
      if (this.appearanceKeys.get(p.id) === appearanceKey(p)) continue;
      entity.destroy();
      this.entities.delete(p.id);
    }

    // 좌석 변화 반영: 슬롯을 잃은 기존 엔티티는 파괴(sessionActive 캐시는
    // 유지 — 에이전트 자체는 살아 있어 나중에 자리가 나면 재생성된다),
    // 슬롯이 바뀐 엔티티는 새 좌석으로 걸어가게 한다(setSeat은 동일 타일이면
    // no-op이라 매 sync 호출해도 안전).
    for (const p of profiles) {
      const entity = this.entities.get(p.id);
      if (!entity) continue;
      const slot = desks.get(p.id);
      if (!slot) {
        entity.destroy();
        this.entities.delete(p.id);
        this.appearanceKeys.delete(p.id);
        continue;
      }
      entity.setSeat(slot.seat);
    }

    for (const p of profiles) {
      if (this.entities.has(p.id)) continue;
      const slot = desks.get(p.id);
      if (!slot) continue; // seat shortage: skip for now (planned follow-up: idle wander without a desk)

      const assets = createCharacterAssets(p, this.renderScale);
      const rand = mulberry32(hashStringToSeed(p.id) ^ MOVEMENT_RNG_SALT);
      const entity = new CharacterEntity(p.id, assets, slot.seat, this.o.map, rand, this.breakReservations);
      entity.setSessionActive(this.sessionActive.get(p.id) ?? false);
      entity.setSubagentCount(this.subagentCounts.get(p.id) ?? 0);
      entity.onClicked((id) => this.o.bus.emitAgentClicked(id));
      entity.onHover((id, x, y) => this.o.bus.emitAgentHoverChanged(id, x, y));
      this.o.characterLayer.addChild(entity.root);
      this.entities.set(p.id, entity);
      this.appearanceKeys.set(p.id, appearanceKey(p));
    }
  }

  /**
   * 카메라 정수 스케일 S 반영(이슈 #47). 커스텀 고해상 시트를 가진 엔티티만
   * D=min(N,16·S)로 재프리필터해 텍스처를 교체한다(FSM/이동 상태는 보존).
   * 절차 생성 스프라이트(항상 16px)와 목표 해상도 D가 그대로인 엔티티는 건너뛴다.
   */
  setRenderScale(scale: number): void {
    const s = Math.max(1, Math.round(scale));
    if (s === this.renderScale) return;
    this.renderScale = s;
    for (const [id, entity] of this.entities) {
      const override = getSpriteOverride(id);
      if (!override) continue; // 커스텀만 대상(절차 생성은 스케일 무관)
      const p = this.profiles.get(id);
      if (!p) continue;
      const n = (override as { height?: number }).height ?? CELL;
      if (detailCellSize(n, s) === entity.cellSize) continue; // 목표 해상도 불변
      entity.replaceAssets(createCharacterAssets(p, s));
    }
  }

  /** 라벨 앵커(머리 위, 월드좌표)를 out에 채운다. out은 호출자가 재사용(per-frame 할당 최소화). */
  collectLabelAnchors(out: Map<string, LabelAnchor>): void {
    out.clear();
    for (const [id, e] of this.entities) {
      out.set(id, { x: e.root.x, y: e.root.y - LABEL_ANCHOR_OFFSET_Y });
    }
  }

  /** dt: ms, ticker-supplied. Forwarded verbatim to every live entity. */
  update(dt: number): void {
    for (const entity of this.entities.values()) entity.update(dt);
  }

  destroy(): void {
    this.unsub.forEach((u) => u());
    this.unsub = [];
    for (const entity of this.entities.values()) entity.destroy();
    this.entities.clear();
    this.appearanceKeys.clear();
    this.sessionActive.clear();
    this.subagentCounts.clear();
    this.profiles.clear();
  }
}
