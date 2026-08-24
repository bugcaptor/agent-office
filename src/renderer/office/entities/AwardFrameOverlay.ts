// src/renderer/office/entities/AwardFrameOverlay.ts
//
// "이 달의 우수사원" 액자 — 틀(테두리+매트)과 그 안쪽 콘텐츠(초상 또는 실루엣
// 폴백)를 이 표시객체 하나가 통째로 그린다. 예전에는 틀만 `officeTileDraw`의
// Tile.Wall 케이스가 벽 고정 장식으로 정적으로 구웠는데, 오피스 벽이 한
// 겹(16px)뿐이라 액자가 16px 타일 폭에 묶여 내부 콘텐츠가 8×22px밖에 안
// 남았다(눈검증에서 "문짝처럼 보인다"로 확인). 액자를 타일 드로잉에서 완전히
// 빼내면 16px 격자에서 자유로워진다 — `scenes/officeScene.ts`의
// `awardFrameRectPx()`가 정하는 임의 크기·위치의 외곽 사각형을 받아 테두리→
// 매트→콘텐츠 순으로 안쪽에 겹쳐 그린다.
//
// 콘텐츠(초상 텍스처가 있으면 nearest-neighbor로 맞춘 Sprite, 없으면
// hasPortrait:false/로드 실패 시 단색 실루엣 Graphics)는 여전히 동적이다.
// `OfficeScene`이 수상자 유무에 따라 `root.visible`을 토글한다 — 수상자가
// 없으면 틀·매트까지 포함해 액자 전체가 숨는다(빈 액자를 벽에 남기지 않는다).
import { Container, Graphics, Sprite, type Texture } from "pixi.js";

/** theme/themes.ts의 TILE_PALETTE_KEYS 액자 관련 키(frameBorder/frameMat/
 * frameSilhouette) 그대로 — 팔레트 객체(theme.pixi)를 변환 없이 바로 넘길 수
 * 있게 키 이름을 맞췄다. */
export interface AwardFramePalette {
  frameBorder: number;
  frameMat: number;
  frameSilhouette: number;
}

/** 테두리 두께(px). */
const BORDER_PX = 2;
/** 매트(테두리와 콘텐츠 사이 여백) 두께(px). */
const MAT_PX = 2;

export class AwardFrameOverlay {
  readonly root = new Container();
  private border = new Graphics();
  private mat = new Graphics();
  private silhouette = new Graphics();
  private photo: Sprite | null = null;
  /** 외곽(틀 바깥 경계) 크기(월드 px). */
  private outerW: number;
  private outerH: number;
  /** 콘텐츠(초상/실루엣) 영역 원점 + 크기 — 테두리·매트 두께만큼 안쪽으로 들어간다. */
  private contentX: number;
  private contentY: number;
  private contentW: number;
  private contentH: number;

  constructor(
    pal: AwardFramePalette,
    /** 액자 외곽(틀 바깥 경계) 크기(월드 px) — 위치/크기의 단일 출처는
     * `scenes/officeScene.ts`의 `awardFrameRectPx()`. */
    outerSize: { w: number; h: number },
  ) {
    this.outerW = outerSize.w;
    this.outerH = outerSize.h;
    const inset = BORDER_PX + MAT_PX;
    this.contentX = inset;
    this.contentY = inset;
    this.contentW = outerSize.w - inset * 2;
    this.contentH = outerSize.h - inset * 2;
    this.root.addChild(this.border, this.mat, this.silhouette);
    this.paint(pal);
  }

  /** 틀(테두리+매트)과 실루엣 색을 전부 재도색한다(테마 전환). 사진이 떠 있을
   * 때도 실루엣은 미리 칠해 둔다(사진이 사라지면 즉시 보이도록). */
  paint(pal: AwardFramePalette): void {
    this.border.clear().rect(0, 0, this.outerW, this.outerH).fill(pal.frameBorder);
    this.mat
      .clear()
      .rect(BORDER_PX, BORDER_PX, this.outerW - BORDER_PX * 2, this.outerH - BORDER_PX * 2)
      .fill(pal.frameMat);

    const g = this.silhouette.clear();
    const w = this.contentW;
    const h = this.contentH;
    const x0 = this.contentX;
    const y0 = this.contentY;
    // 머리(원) + 어깨(사다리꼴) — 정사각 콘텐츠(20×20 기준) 안에 들어가는
    // 최소 인물 실루엣. 이전엔 좁고 긴 액자(8×22) 전제로 짠 비율이라 정사각에
    // 맞춰 다시 잡았다: 머리는 살짝 작게, 어깨는 콘텐츠 폭 대부분을 쓴다.
    const headR = Math.min(w, h) * 0.22;
    g.circle(x0 + w / 2, y0 + headR + 2, headR).fill(pal.frameSilhouette);
    g.poly([
      x0 + w * 0.15, y0 + h,
      x0 + w * 0.85, y0 + h,
      x0 + w * 0.7, y0 + h * 0.58,
      x0 + w * 0.3, y0 + h * 0.58,
    ]).fill(pal.frameSilhouette);
  }

  /** 초상 텍스처를 "contain" 방식(비율 유지, nearest 스케일)으로 콘텐츠 영역에
   * 넣는다. 실루엣은 뒤에 남겨 두고 사진을 그 위에 얹는다. */
  showPhoto(texture: Texture): void {
    this.photo?.destroy();
    texture.source.scaleMode = "nearest";
    const sprite = new Sprite(texture);
    sprite.anchor.set(0.5);
    const scale = Math.min(this.contentW / texture.width, this.contentH / texture.height);
    sprite.width = Math.round(texture.width * scale);
    sprite.height = Math.round(texture.height * scale);
    sprite.position.set(
      Math.round(this.contentX + this.contentW / 2),
      Math.round(this.contentY + this.contentH / 2),
    );
    this.root.addChild(sprite);
    this.photo = sprite;
  }

  /** 사진을 내리고 실루엣 폴백으로 되돌린다(초상 없음/로드 실패). */
  showSilhouette(): void {
    if (!this.photo) return;
    this.photo.destroy();
    this.photo = null;
  }

  setVisible(v: boolean): void {
    this.root.visible = v;
  }

  destroy(): void {
    this.photo?.destroy();
    this.photo = null;
    this.root.destroy({ children: true });
  }
}
