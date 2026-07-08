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
export function isSessionActive(state: SessionState): boolean {
  return state === "starting" || state === "running";
}

export class OfficeWorld {
  private entities = new Map<string, CharacterEntity>();
  private appearanceKeys = new Map<string, string>();
  private sessionActive = new Map<string, boolean>();
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
  }

  /** Diff the live entity set against `profiles`: destroy dropped agents, create new ones,
   * recreate agents whose appearance key changed, leave the rest untouched. */
  syncAgents(profiles: readonly AgentProfile[]): void {
    const desks = assignDesks(this.o.map, profiles.map((p) => p.id));
    const next = new Set(profiles.map((p) => p.id));

    for (const [id, entity] of this.entities) {
      if (next.has(id)) continue;
      entity.destroy();
      this.entities.delete(id);
      this.appearanceKeys.delete(id);
      this.sessionActive.delete(id);
    }

    // 외형 키가 바뀐 기존 엔티티는 파괴해 아래 생성 루프에서 재생성한다.
    for (const p of profiles) {
      const entity = this.entities.get(p.id);
      if (!entity) continue;
      if (this.appearanceKeys.get(p.id) === appearanceKey(p)) continue;
      entity.destroy();
      this.entities.delete(p.id);
    }

    for (const p of profiles) {
      if (this.entities.has(p.id)) continue;
      const slot = desks.get(p.id);
      if (!slot) continue; // seat shortage: skip for now (planned follow-up: idle wander without a desk)

      const assets = createCharacterAssets(p);
      const rand = mulberry32(hashStringToSeed(p.id) ^ MOVEMENT_RNG_SALT);
      const entity = new CharacterEntity(p.id, assets, slot.seat, this.o.map, rand);
      entity.setSessionActive(this.sessionActive.get(p.id) ?? false);
      entity.onClicked((id) => this.o.bus.emitAgentClicked(id));
      entity.onHover((id, x, y) => this.o.bus.emitAgentHoverChanged(id, x, y));
      this.o.characterLayer.addChild(entity.root);
      this.entities.set(p.id, entity);
      this.appearanceKeys.set(p.id, appearanceKey(p));
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
  }
}
