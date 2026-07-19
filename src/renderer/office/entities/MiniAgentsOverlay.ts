// src/renderer/office/entities/MiniAgentsOverlay.ts
//
// 부모 캐릭터 머리 옆에 떠다니는 "미니 서브에이전트" 표시. 부모 스프라이트
// 텍스처를 그대로 재사용(복제 아님)해 축소 클론으로 그린다. 카운트 기반:
// setCount(n)이 min(n,3)마리를 보이게 하고, update(dt)가 sin 밥으로 흔든다.
// ThinkingOverlay 패턴(자식 Container + dt 구동 + 캐릭터가 소유/파괴)을 따른다.

import { Container, Sprite, type Texture } from "pixi.js";

const SLOT_X = [-11, 11, -15]; // 머리 옆 고정 슬롯
const MAX_MINIS = SLOT_X.length;
const SLOT_BASE_Y = [0, -1, 2]; // 미니마다 살짝 다른 기준 높이(머리 라인 근처)
const MINI_SCALE_FACTOR = 0.5; // 부모 spriteScale 대비
const MINI_ALPHA = 0.75;
const BOB_AMPLITUDE_PX = 1.5;
const BOB_PERIOD_MS = 1200;
const BOB_PHASE_STEP = (Math.PI * 2) / 3; // 미니 간 위상차

export class MiniAgentsOverlay {
  readonly root = new Container();
  private minis: Sprite[];
  private count = 0;
  private t = 0;

  constructor(texture: Texture, spriteScale: number) {
    this.minis = SLOT_X.map((x, i) => {
      const s = new Sprite(texture);
      s.anchor.set(0.5, 1); // feet-aligned, 부모와 동일
      s.scale.set(spriteScale * MINI_SCALE_FACTOR);
      s.alpha = MINI_ALPHA;
      s.position.set(x, SLOT_BASE_Y[i]);
      s.visible = false;
      this.root.addChild(s);
      return s;
    });
  }

  /** 부모 텍스처/배율 교체(커스텀 시트 S-적응 재생성 시). 텍스처는 부모 소유. */
  setBase(texture: Texture, spriteScale: number): void {
    this.minis.forEach((s) => {
      s.texture = texture;
      s.scale.set(spriteScale * MINI_SCALE_FACTOR);
    });
  }

  setCount(n: number): void {
    const clamped = Math.max(0, Math.min(MAX_MINIS, Math.floor(n)));
    this.count = clamped;
    this.minis.forEach((s, i) => {
      s.visible = i < clamped;
    });
  }

  /** dt: ms. 보이는 미니만 sin 밥으로 흔든다(숨김이면 계산 생략). */
  update(dt: number): void {
    if (this.count === 0) return;
    this.t += dt;
    for (let i = 0; i < this.count; i++) {
      const phase = (this.t / BOB_PERIOD_MS) * Math.PI * 2 - i * BOB_PHASE_STEP;
      this.minis[i].y = SLOT_BASE_Y[i] + Math.sin(phase) * BOB_AMPLITUDE_PX;
    }
  }

  destroy(): void {
    // 텍스처는 부모 소유 → destroy에서 파괴 금지(children Container만 정리).
    this.root.destroy({ children: true, texture: false });
  }
}
