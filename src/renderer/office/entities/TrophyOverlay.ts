// src/renderer/office/entities/TrophyOverlay.ts
//
// "이 달의 우수사원" 트로피 표시. ExclamationOverlay와 같은 자기완결 표시객체
// 패턴 — 부모(OfficeScene)가 좌표/visible/zIndex만 다룬다.
//
// 그림은 **🏆 글리프를 13px로 구운 스프라이트**가 1순위고, 굽기가 실패하면
// (이모지 폰트 없음 / 2d 컨텍스트 없음 / 테스트 환경) 아래의 절차적 도트
// 드로잉으로 떨어진다. 12×12px 안에 컵·손잡이·기둥·받침을 손으로 다 읽히게
// 그리는 건 도트 예산이 빡빡해 눈검증을 세 번 물렸고(안 보임 → 너무 넓음 →
// 금색 일변도), 글리프를 크게 그려 줄이면 그 실루엣이 통째로 살아난다.
// 굽는 쪽 설명은 `gen/emojiTexture.ts`.
//
// 절차적 폴백의 색은 테마 팔레트 축(theme/themes.ts의 trophy* 키)이라
// `paint()`로 테마 전환 시 재도색한다(파기 없이). 이모지 스프라이트는 원색
// 고정이라 테마를 안 탄다 — 그 판단 근거도 `gen/emojiTexture.ts`에 적었다.
import { Container, Graphics, Sprite } from "pixi.js";
import { bakeEmojiTexture, EMOJI_OUTLINE_PX } from "../gen/emojiTexture";

/** theme/themes.ts의 TILE_PALETTE_KEYS 트로피 축(trophy*) 그대로 — 팔레트
 * 객체(theme.pixi)를 변환 없이 바로 넘길 수 있게 키 이름을 맞췄다. */
export interface TrophyPalette {
  trophyCup: number;
  trophyCupShine: number;
  trophyBase: number;
  trophyRibbon: number;
}

/** 구울 글리프와 크기(px). 16px 타일 위에 좌우 여유를 남기는 최대치가 13이다
 * (16px로 구우면 손잡이가 책상 타일 밖으로 나간다 — 눈검증 확인). 외곽선까지
 * 하면 15px이라 타일 안에 아슬아슬하게 들어간다. */
export const TROPHY_EMOJI = "🏆";
export const TROPHY_EMOJI_PX = 13;

/** 외곽선 색을 만들 때 팔레트 색에 곱하는 비율. `trophyBase`를 그대로 두르면
 * 웜톤 책상과 명도가 비슷해 테두리가 아니라 얼룩으로 보인다(눈검증) — 이만큼
 * 어둡게 눌러야 네 테마 전부에서 윤곽선으로 읽힌다. */
const OUTLINE_DARKEN = 0.45;

/** 24bit RGB를 채널별로 눌러 어둡게. 순수. */
function darken(color: number, factor: number): number {
  const r = Math.round(((color >> 16) & 0xff) * factor);
  const g = Math.round(((color >> 8) & 0xff) * factor);
  const b = Math.round((color & 0xff) * factor);
  return (r << 16) | (g << 8) | b;
}

export class TrophyOverlay {
  readonly root = new Container();
  private g = new Graphics();
  /** 이모지 스프라이트. 굽기에 실패하면 null이고 그때만 `g`가 보인다. */
  private sprite: Sprite | null = null;

  constructor(pal: TrophyPalette) {
    this.root.addChild(this.g);
    this.paint(pal);
  }

  /** 이모지 스프라이트를 쓰고 있는지(테스트·진단용). */
  get usesEmoji(): boolean {
    return this.sprite !== null;
  }

  /** 테마 전환 진입점 — 폴백 재도색 + 이모지 텍스처(외곽선 색이 테마 축이다) 교체. */
  paint(pal: TrophyPalette): void {
    this.paintFallback(pal);
    this.syncSprite(pal);
  }

  /**
   * 절차적 폴백 도트 드로잉. 이모지 스프라이트가 떠 있으면 화면에는 안 보이지만
   * 그래도 칠해 둔다 — 폴백이 드러나는 경로가 생겨도 색이 옛 테마로 남지 않게.
   *
   * 이 모양 자체도 눈검증을 두 번 반영한 것이다.
   *  - "좌우로 너무 넓다": 폭 12px(컵 8 + 손잡이 2+2) → **8px**(컵 6 + 손잡이
   *    1+1). 16px 타일 위 소품이라 반 칸을 넘어가면 책상을 다 덮는다.
   *  - "금색밖에 없어서 구분이 안 감": 컵·기둥·받침이 전부 같은 황동 계열이라
   *    실루엣이 한 덩어리로 뭉쳤다. 목 리본과 받침 명패 두 군데에 포인트 색
   *    (`trophyRibbon`)을 넣어 컵/기둥/받침 경계를 색으로 끊는다.
   */
  private paintFallback(pal: TrophyPalette): void {
    const g = this.g.clear();
    // 컵·손잡이 외곽선 패스: 같은 실루엣을 1px 키워 짙은 받침색으로 먼저 깐다.
    // 밝은 테마의 책상(웜 오크)과 골드 컵은 명도가 비슷해 그냥 얹으면 상판에
    // 묻힌다 — 1px 테두리가 어느 테마에서든 컵의 윤곽을 세운다. 받침·기둥은
    // 원래 짙은 색(trophyBase)이라 이미 대비가 서므로 두르지 않는다(두르면
    // 아래쪽이 뭉툭한 덩어리로 보인다).
    g.rect(-4, -7, 8, 7).fill(pal.trophyBase); // 컵
    g.rect(-5, -6, 3, 5).fill(pal.trophyBase); // 왼쪽 손잡이
    g.rect(2, -6, 3, 5).fill(pal.trophyBase); // 오른쪽 손잡이

    // 컵(볼) 몸체
    g.rect(-3, -6, 6, 5).fill(pal.trophyCup);
    // 손잡이(좌/우) — 1px씩. 2px이면 컵보다 손잡이가 눈에 먼저 든다.
    g.rect(-4, -5, 1, 3).fill(pal.trophyCup);
    g.rect(3, -5, 1, 3).fill(pal.trophyCup);
    // 광택 하이라이트
    g.rect(-2, -5, 1, 2).fill(pal.trophyCupShine);
    // 목 리본 — 컵과 기둥 사이를 색으로 끊는다.
    g.rect(-2, -1, 4, 1).fill(pal.trophyRibbon);
    // 기둥
    g.rect(-1, 0, 2, 2).fill(pal.trophyBase);
    // 받침
    g.rect(-3, 2, 6, 3).fill(pal.trophyBase);
    // 받침 명패 — 두 번째 포인트 색. 받침이 단색 블록으로 안 보이게 한다.
    g.rect(-2, 3, 4, 1).fill(pal.trophyRibbon);
  }

  /**
   * 이모지 스프라이트를 (재)생성·교체한다. 외곽선 색이 테마 축(trophyBase)이라
   * 테마가 바뀌면 다른 텍스처가 필요하다 — 굽기는 (문자, 크기, 색)으로 캐시되므로
   * 테마를 왕복해도 다시 굽지 않는다. 굽기가 실패하면 폴백을 그대로 보여 준다.
   */
  private syncSprite(pal: TrophyPalette): void {
    const texture = bakeEmojiTexture(
      TROPHY_EMOJI,
      TROPHY_EMOJI_PX,
      darken(pal.trophyBase, OUTLINE_DARKEN),
    );
    if (!texture) {
      this.g.visible = true;
      return;
    }
    if (!this.sprite) {
      const sprite = new Sprite(texture);
      // 바닥 기준 정렬 — 폴백의 받침 바닥(y=+5)에 맞춰 책상에 얹는다.
      // 텍스처는 외곽선만큼 사방으로 크므로 그 몫을 더해야 글리프 바닥이 +5다.
      sprite.anchor.set(0.5, 1);
      sprite.position.set(0, 5 + EMOJI_OUTLINE_PX);
      this.root.addChild(sprite);
      this.sprite = sprite;
    } else {
      this.sprite.texture = texture;
    }
    this.g.visible = false; // 폴백은 칠해는 두되 가린다
  }

  setVisible(v: boolean): void {
    this.root.visible = v;
  }

  destroy(): void {
    // 텍스처는 캐시(gen/emojiTexture)가 들고 있으므로 파기하지 않는다 —
    // 씬 재구축 때 다시 굽지 않고 그대로 재사용한다.
    this.sprite = null;
    this.root.destroy({ children: true });
  }
}
