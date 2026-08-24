// src/renderer/office/entities/TrophyOverlay.ts
//
// "이 달의 우수사원" 트로피 표시(절차적 드로잉, 에셋 없음). ExclamationOverlay와
// 같은 자기완결 표시객체 패턴 — 부모(OfficeScene)가 좌표/visible/zIndex만
// 다루고, 이 클래스는 16px 그리드에 맞춘 작은 컵 모양(받침·기둥·컵·손잡이)만
// 안다. 색은 테마 팔레트 축(theme/themes.ts의 trophy* 키)이라 `paint()`로
// 테마 전환 시 재도색한다(파기 없이).
import { Container, Graphics } from "pixi.js";

/** theme/themes.ts의 TILE_PALETTE_KEYS 트로피 축(trophy*) 그대로 — 팔레트
 * 객체(theme.pixi)를 변환 없이 바로 넘길 수 있게 키 이름을 맞췄다. */
export interface TrophyPalette {
  trophyCup: number;
  trophyCupShine: number;
  trophyBase: number;
}

export class TrophyOverlay {
  readonly root = new Container();
  private g = new Graphics();

  constructor(pal: TrophyPalette) {
    this.root.addChild(this.g);
    this.paint(pal);
  }

  /** 테마 전환 시 색만 다시 칠한다(형태는 고정). */
  paint(pal: TrophyPalette): void {
    const g = this.g.clear();
    // 받침(바닥 블록)
    g.rect(-5, 4, 10, 2).fill(pal.trophyBase);
    // 기둥
    g.rect(-1, 0, 2, 4).fill(pal.trophyBase);
    // 컵(볼) 몸체
    g.rect(-4, -6, 8, 6).fill(pal.trophyCup);
    // 손잡이(좌/우)
    g.rect(-6, -5, 2, 3).fill(pal.trophyCup);
    g.rect(4, -5, 2, 3).fill(pal.trophyCup);
    // 광택 하이라이트(1~2px)
    g.rect(-3, -5, 2, 2).fill(pal.trophyCupShine);
  }

  setVisible(v: boolean): void {
    this.root.visible = v;
  }

  destroy(): void {
    this.root.destroy({ children: true });
  }
}
