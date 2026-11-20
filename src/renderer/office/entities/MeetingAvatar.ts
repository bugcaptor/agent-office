// src/renderer/office/entities/MeetingAvatar.ts
//
// 토의 회의실 의자에 앉는 "분신". 본체(CharacterEntity)가 본업 중이라 회의실에 올 수 없을 때 그 의자를
// 대신 채운다 — 본체는 책상에서 일하고, 분신은 회의실에서 말풍선·의사봉을 띄운다.
//
// 본체와 달리 FSM·이동·탕비실·알림 줄이 없다. 의자에 고정돼 idle 프레임만 돌린다. 보일지는 월드가 정한다
// (본체가 그 의자에 앉아 있으면 숨긴다). 본체와 구별되게 살짝 투명하다.
import { Container, Rectangle, Sprite } from "pixi.js";

import type { CharacterAssets } from "../gen/characterFactory";
import type { MeetingSeat } from "../map/mapData";
import { MODERATOR_BADGE_POS, ModeratorBadge, SPEECH_BUBBLE_MEETING_POS, SpeechOverlay } from "./DiscussionOverlay";
import { meetingSeatFeetPx } from "./CharacterEntity";

const ANIM_IDLE_MS = 480; // CharacterEntity와 같은 idle 프레임 주기
const APPARENT_CELL = 16;
/** 분신 표시 투명도. 본체(1)와 나란히 보일 때 어느 쪽이 분신인지 알 수 있을 만큼만. */
export const MEETING_AVATAR_ALPHA = 0.7;

export class MeetingAvatar {
  readonly root = new Container();
  private sprite: Sprite;
  private speech = new SpeechOverlay();
  private badge = new ModeratorBadge();
  private animTimer = 0;
  private frameIdx = 0;

  constructor(
    readonly agentId: string,
    private assets: CharacterAssets,
    readonly seat: MeetingSeat,
  ) {
    const scale = APPARENT_CELL / assets.cellSize;
    this.sprite = new Sprite(assets.idle[0]);
    this.sprite.anchor.set(0.5, 1);
    this.sprite.scale.set(seat.facing === "left" ? -scale : scale, scale);
    this.sprite.alpha = MEETING_AVATAR_ALPHA;
    this.root.addChild(this.sprite);
    this.speech.root.position.set(SPEECH_BUBBLE_MEETING_POS.x, SPEECH_BUBBLE_MEETING_POS.y);
    this.root.addChild(this.speech.root);
    this.badge.root.position.set(MODERATOR_BADGE_POS.x, MODERATOR_BADGE_POS.y);
    this.root.addChild(this.badge.root);

    const feet = meetingSeatFeetPx(seat);
    this.root.position.set(feet.x, feet.y);
    this.root.zIndex = this.root.y;
    this.root.visible = false;

    // 히트 영역은 텍스처 셀(local, pre-scale) 기준 — CharacterEntity와 같은 이유.
    const n = assets.cellSize;
    this.sprite.eventMode = "static";
    this.sprite.cursor = "pointer";
    this.sprite.hitArea = new Rectangle(-n / 2, -n, n, n);
  }

  get cellSize(): number {
    return this.assets.cellSize;
  }

  get visible(): boolean {
    return this.root.visible;
  }

  setVisible(v: boolean): void {
    this.root.visible = v;
  }

  setRole(moderator: boolean, speaking: boolean): void {
    this.badge.setVisible(moderator);
    this.speech.setVisible(speaking);
  }

  onClicked(cb: (id: string) => void): void {
    this.sprite.on("pointertap", () => cb(this.agentId));
  }

  onHover(cb: (id: string | null, x: number, y: number) => void): void {
    this.sprite.on("pointerover", (e) => cb(this.agentId, e.global.x, e.global.y));
    this.sprite.on("pointerout", () => cb(null, 0, 0));
  }

  /** dt: ms. 숨어 있으면 아무것도 하지 않는다. */
  update(dt: number): void {
    if (!this.root.visible) return;
    this.animTimer += dt;
    if (this.animTimer >= ANIM_IDLE_MS) {
      this.animTimer = 0;
      this.frameIdx = (this.frameIdx + 1) % this.assets.idle.length;
      this.sprite.texture = this.assets.idle[this.frameIdx];
    }
    this.speech.update(dt);
  }

  destroy(): void {
    this.speech.destroy();
    this.badge.destroy();
    this.root.destroy({ children: true });
    this.assets.dispose?.();
  }
}
