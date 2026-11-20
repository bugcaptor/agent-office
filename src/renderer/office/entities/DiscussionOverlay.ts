// src/renderer/office/entities/DiscussionOverlay.ts
//
// 토의 연출 두 가지를 그린다(코드 드로잉, 이미지 에셋 없음).
//  - 발언 말풍선: 지금 차례를 받아 말하는 참가자. 생각 말풍선("…" 점)과 헷갈리지 않게 꼬리가 왼쪽
//    아래(머리 쪽)를 향하는 옆 말풍선에 길이가 바뀌는 "말줄" 세 개를 넣는다.
//  - 사회자 표시: 토의가 진행 중인 동안 사회자 머리 왼쪽에 작은 의사봉.
//
// 둘 다 머리 *옆*에 둔다. 본업이 일하는 참가자는 머리 바로 위에 작업 라벨(DOM)이 떠 있어서, 머리 위에
// 그리면 라벨에 덮인다(라벨 아래 변 = 발끝에서 20px 위).
import { Container, Graphics } from "pixi.js";

const LINE_PERIOD_MS = 360; // 말줄 길이가 한 번 바뀌는 주기
// 말줄 세 개의 길이 패턴(px). 주기마다 다음 패턴으로 넘어가 입을 여닫는 듯 보인다.
const LINE_PATTERNS: readonly (readonly number[])[] = [
  [8, 5, 7],
  [6, 8, 4],
  [9, 6, 8],
  [5, 7, 6],
];

/** 발언 말풍선 위치(발끝 기준, px). 머리 오른쪽 위 — 라벨 아래 변(-20)보다 낮다. */
export const SPEECH_BUBBLE_POS = { x: 13, y: -13 } as const;
/** 사회자 의사봉 위치(발끝 기준, px). 머리 왼쪽. */
export const MODERATOR_BADGE_POS = { x: -10, y: -12 } as const;

export class SpeechOverlay {
  readonly root = new Container();
  private lines: Graphics;
  private t = 0;
  private pattern = 0;

  constructor() {
    const bubble = new Graphics();
    // 몸통 14×10 + 머리 쪽(왼쪽 아래)으로 뻗은 꼬리.
    bubble.roundRect(-6, -9, 14, 10, 3).fill(0xffffff).stroke({ width: 1, color: 0x4a4a5a });
    bubble.poly([-5, -1, -1, 0, -8, 4]).fill(0xffffff);
    bubble.moveTo(-5, 0).lineTo(-8, 4).lineTo(-1, 1).stroke({ width: 1, color: 0x4a4a5a });
    this.root.addChild(bubble);
    this.lines = new Graphics();
    this.root.addChild(this.lines);
    this.paintLines();
    this.root.visible = false;
  }

  setVisible(v: boolean): void {
    if (v && !this.root.visible) {
      this.t = 0;
      this.pattern = 0;
      this.paintLines();
    }
    this.root.visible = v;
  }

  get visible(): boolean {
    return this.root.visible;
  }

  /** dt: ms. 숨어 있으면 아무것도 하지 않는다(ThinkingOverlay와 같은 규칙). */
  update(dt: number): void {
    if (!this.root.visible) return;
    this.t += dt;
    if (this.t < LINE_PERIOD_MS) return;
    this.t %= LINE_PERIOD_MS;
    this.pattern = (this.pattern + 1) % LINE_PATTERNS.length;
    this.paintLines();
  }

  private paintLines(): void {
    this.lines.clear();
    LINE_PATTERNS[this.pattern].forEach((w, i) => {
      this.lines.rect(-4, -7 + i * 3, w, 1).fill(0x4a4a5a);
    });
  }

  destroy(): void {
    this.root.destroy({ children: true });
  }
}

/** 사회자 표시: 비스듬한 의사봉(머리 + 자루) 픽셀아트. */
export class ModeratorBadge {
  readonly root = new Container();

  constructor() {
    const g = new Graphics();
    // 그림자 테두리를 먼저 깔아 어떤 바닥색 위에서도 윤곽이 읽힌다.
    g.rect(-4, -5, 7, 4).fill(0x3a2a18); // 머리 테두리
    g.rect(-3, -4, 5, 2).fill(0xd9a24a); // 머리(금빛 나무)
    g.rect(-2, -4, 1, 2).fill(0xf6d58e); // 하이라이트
    g.rect(0, -1, 2, 2).fill(0x3a2a18); // 자루
    g.rect(1, 1, 2, 2).fill(0x3a2a18);
    g.rect(2, 3, 2, 1).fill(0x3a2a18);
    this.root.addChild(g);
    this.root.visible = false;
  }

  setVisible(v: boolean): void {
    this.root.visible = v;
  }

  get visible(): boolean {
    return this.root.visible;
  }

  destroy(): void {
    this.root.destroy({ children: true });
  }
}
