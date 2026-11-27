// src/renderer/office/world/RemoteFloor.ts
//
// 머신 연동 호스트에서 다른 머신 하나의 층. 같은 맵·같은 캐릭터 엔티티(`OfficeWorld`)를 쓰되, 상태는
// 앱 스토어가 아니라 그 머신이 올린 스냅숏(`link-remote`)에서 온다 — 세션이 돌면 책상에 앉고, 답을 기다리면
// 줄을 서고, 머신이 끊기면 흐리게 보인다. 클릭은 원격 대화 보기를 연다(`onClick`).
//
// 이 층에는 책상 주인 지정·보스 책상·저장소 점검·토의 연출이 없다 — 모두 이 머신의 것이다.
import type { Container } from "pixi.js";
import { createMockOfficeBus, type OfficeBus } from "../bus";
import type { OfficeMap } from "../map/mapData";
import type { AgentProfile } from "../types";
import { OfficeWorld } from "./OfficeWorld";

export interface RemoteFloorAgent {
  profile: AgentProfile;
  /** 세션이 돌고 있다(책상에 앉는다). */
  active: boolean;
  /** 답을 기다린다(보스 책상 앞 줄). */
  pending: boolean;
  /** 머신이 붙어 있다. 끊겼으면 흐리게. */
  online: boolean;
}

/** 오피스 씬이 지금 보일 다른 층. `key`가 바뀌면 다른 머신이다. */
export interface RemoteFloorView {
  key: string;
  agents: readonly RemoteFloorAgent[];
}

/** 끊긴 머신 캐릭터의 불투명도. */
export const OFFLINE_ALPHA = 0.35;

export class RemoteFloor {
  private mock = createMockOfficeBus();
  private world: OfficeWorld;
  private key: string | null = null;

  constructor(o: {
    characterLayer: Container;
    overlayLayer: Container;
    map: OfficeMap;
    onClick: (id: string) => void;
    onHover: (id: string | null, x: number, y: number) => void;
  }) {
    const bus: OfficeBus = {
      ...this.mock,
      emitAgentClicked: (id) => o.onClick(id),
      emitAgentHoverChanged: (id, x, y) => o.onHover(id, x, y),
    };
    this.world = new OfficeWorld({ bus, characterLayer: o.characterLayer, overlayLayer: o.overlayLayer, map: o.map });
    this.world.setHidden(true);
  }

  /** 보일 층으로 맞춘다. null이면 숨긴다(엔티티도 버린다). */
  show(view: RemoteFloorView | null): void {
    if (!view) {
      this.key = null;
      this.world.syncAgents([]);
      this.world.setHidden(true);
      return;
    }
    // 다른 머신으로 바뀌면 자리부터 새로 잡는다(같은 캐릭터 id가 남지 않게 비운다).
    if (this.key !== view.key) this.world.syncAgents([]);
    this.key = view.key;
    this.world.syncAgents(view.agents.map((a) => a.profile));
    for (const a of view.agents) {
      this.mock.triggerSessionStateChanged(a.profile.id, a.active ? "running" : "exited");
      this.mock.triggerNotificationChanged(a.profile.id, a.pending);
      this.world.setAgentAlpha(a.profile.id, a.online ? 1 : OFFLINE_ALPHA);
    }
    this.world.setHidden(false);
  }

  get visible(): boolean {
    return this.key !== null;
  }

  setMap(map: OfficeMap): void {
    this.world.setMap(map);
  }

  setRenderScale(scale: number): void {
    this.world.setRenderScale(scale);
  }

  update(dt: number): void {
    if (this.key !== null) this.world.update(dt);
  }

  collectLabelAnchors(out: Map<string, { x: number; y: number }>): void {
    this.world.collectLabelAnchors(out);
  }

  destroy(): void {
    this.world.destroy();
  }
}
