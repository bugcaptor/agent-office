// src/renderer/office/entities/AwardFrameOverlay.ts
//
// "이 달의 우수사원" 벽 사진 — 압정으로 꽂은 작은 폴라로이드. 카드(종이+사진칸
// +장식)와 압정, 그 안쪽 콘텐츠(초상 또는 실루엣 폴백)를 이 표시객체 하나가
// 통째로 그린다.
//
// ## 왜 액자가 아니라 폴라로이드인가
//
// 처음엔 28×28 정사각 액자(짙은 테두리 + 매트 + 정면 흉상)를 벽 정중앙(tx8-9)에
// **단 하나만** 걸었다. 눈검증에서 "영정사진 같다 / 수령님 초상화 같다"가 나왔고
// 이유는 도상이 정확히 겹치기 때문이다: 정사각 + 어두운 테두리 + 정면 흉상 +
// 정중앙 단독 배치 + 축하 맥락 표지 0. 보는 사람은 아는 도상으로 읽는다.
//
// 그래서 셋을 동시에 바꿨다.
//
//  1. **형태**: 아래 여백이 넓은 폴라로이드 카드(18×21) — 세로 흉상 액자 비율에서
//     벗어난다. 아래 여백에 작은 별 하나를 찍어 축하 맥락을 준다.
//  2. **크기**: 28×28 → 18×21. 작으면 위압적이지 않다.
//  3. **배치**: 정중앙 간격(tx8-9)을 버리고 오른쪽 간격(tx12-13)으로, 그 안에서도
//     중앙정렬이 아니라 살짝 왼쪽으로 치우쳐 꽂는다. "누가 붙여둔 사진"이 된다.
//     + 압정에 매달려 살짝 기운다(TILT_RAD).
//
// 수상 표현이 이것뿐도 아니다 — `TrophyOverlay`가 수상자 **책상 위**에 트로피를
// 올린다. 벽 사진은 "누가 받았는지"를 알려주는 보조 표시라 작아도 된다.
//
// ## 구조
//
// 압정은 벽에 박힌 것이라 안 기울고, 카드만 압정을 축으로 기운다. 그래서
// root 아래에 회전하는 `card` 컨테이너(pivot=압정 위치)와 회전하지 않는 `pin`을
// 나란히 둔다 — 부모(OfficeScene)는 여전히 `root.position` 하나만 다룬다.
//
// 콘텐츠(초상 텍스처가 있으면 nearest-neighbor로 맞춘 Sprite, 없으면
// hasPortrait:false/로드 실패 시 단색 실루엣)는 동적이다. `OfficeScene`이
// 수상자 유무에 따라 `root.visible`을 토글한다 — 수상자가 없으면 카드·압정까지
// 포함해 전체가 숨는다(빈 액자를 벽에 남기지 않는다).
import { Container, Graphics, Sprite, type Texture } from "pixi.js";

/** theme/themes.ts의 TILE_PALETTE_KEYS 액자 관련 키(frameBorder/frameMat/
 * frameSilhouette) 그대로 — 팔레트 객체(theme.pixi)를 변환 없이 바로 넘길 수
 * 있게 키 이름을 맞췄다. 폴라로이드로 바뀌면서 쓰임만 옮겨 갔다(색 값은 그대로
 * 재사용): frameMat=카드 종이, frameSilhouette=사진칸 바탕/장식,
 * frameBorder=그림자·인물 실루엣·압정. */
export interface AwardFramePalette {
  frameBorder: number;
  frameMat: number;
  frameSilhouette: number;
}

/** 사진칸 좌·우·상 여백(px). 아래 여백은 남는 높이 전부(폴라로이드의 그 넓은 턱). */
const MARGIN_PX = 2;
/** 카드 그림자 오프셋(px). */
const SHADOW_PX = 1;
/** 압정을 축으로 한 기울기(rad, 약 -3.4°). 픽셀아트라 크게 주면 사진 스프라이트에
 * 계단 아티팩트가 보인다 — 격식만 깨질 만큼 아주 조금. */
const TILT_RAD = -0.06;

export class AwardFrameOverlay {
  readonly root = new Container();
  /** 압정을 축으로 기울어지는 카드. 사진 스프라이트도 여기 붙어 같이 기운다. */
  private card = new Container();
  /** 카드 종이 + 그림자 + 사진칸 바탕 + 아래 여백 장식(한 겹으로 함께 재도색). */
  private paper = new Graphics();
  private silhouette = new Graphics();
  /** 벽에 박힌 압정 — 카드와 달리 기울지 않는다. */
  private pin = new Graphics();
  private photo: Sprite | null = null;
  /** 카드(그림자 제외) 크기(월드 px). */
  private cardW: number;
  private cardH: number;
  /** 사진칸 원점 + 크기 — 좌·우·상은 MARGIN_PX, 아래는 남는 만큼 넓다. */
  private photoX: number;
  private photoY: number;
  private photoW: number;
  private photoH: number;

  constructor(
    pal: AwardFramePalette,
    /** 폴라로이드 카드 크기(월드 px) — 위치/크기의 단일 출처는
     * `scenes/officeScene.ts`의 `awardFrameRectPx()`. */
    outerSize: { w: number; h: number },
  ) {
    this.cardW = outerSize.w;
    this.cardH = outerSize.h;
    this.photoX = MARGIN_PX;
    this.photoY = MARGIN_PX;
    this.photoW = outerSize.w - MARGIN_PX * 2;
    // 사진칸은 정사각. 남는 높이가 곧 폴라로이드의 아래 턱이다.
    this.photoH = this.photoW;

    // 압정 위치를 축으로 카드만 기운다(압정은 벽에 박힌 것이라 안 기운다).
    const pinX = Math.round(outerSize.w / 2);
    this.card.pivot.set(pinX, 0);
    this.card.position.set(pinX, 0);
    this.card.rotation = TILT_RAD;
    this.card.addChild(this.paper, this.silhouette);
    this.root.addChild(this.card, this.pin);
    this.paint(pal);
  }

  /** 카드·실루엣·압정 색을 전부 재도색한다(테마 전환). 사진이 떠 있을 때도
   * 실루엣은 미리 칠해 둔다(사진이 사라지면 즉시 보이도록). */
  paint(pal: AwardFramePalette): void {
    const w = this.cardW;
    const h = this.cardH;

    this.paper
      .clear()
      // 그림자(살짝 오른쪽 아래로) — 벽에서 떠 있는 느낌을 준다.
      .rect(SHADOW_PX, SHADOW_PX, w, h)
      .fill({ color: pal.frameBorder, alpha: 0.35 })
      // 카드 종이
      .rect(0, 0, w, h)
      .fill(pal.frameMat)
      // 사진칸 바탕(초상이 없을 때 그대로 보인다)
      .rect(this.photoX, this.photoY, this.photoW, this.photoH)
      .fill(pal.frameSilhouette);

    // 아래 턱 중앙의 작은 별(십자 3px) — 축하 맥락 표지. 이것 하나로 "영정"
    // 해석이 막힌다.
    const starX = Math.round(w / 2);
    const starY = Math.round(this.photoY + this.photoH + (h - this.photoY - this.photoH) / 2);
    this.paper
      .rect(starX - 1, starY, 3, 1)
      .fill(pal.frameSilhouette)
      .rect(starX, starY - 1, 1, 3)
      .fill(pal.frameSilhouette);

    // 압정: 십자 3×3 머리(모서리를 비워 둥근 머리로 읽힌다) + 꼭대기 1px
    // 하이라이트. 카드 위로 1px 삐져나와 벽에 박힌 모양이 된다. 꽉 찬 사각형은
    // 압정이 아니라 "종이에 붙은 탭"으로 보여서 십자로 판다.
    this.pin
      .clear()
      .rect(starX, -1, 1, 3)
      .fill(pal.frameBorder)
      .rect(starX - 1, 0, 3, 1)
      .fill(pal.frameBorder)
      .rect(starX, -1, 1, 1)
      .fill(pal.frameMat);

    const g = this.silhouette.clear();
    const x0 = this.photoX;
    const y0 = this.photoY;
    const pw = this.photoW;
    const ph = this.photoH;
    // 머리(원) + 어깨(사다리꼴) — 정사각 사진칸에 들어가는 최소 인물 실루엣.
    // 사진칸 바탕(frameSilhouette)과 대비되게 짙은 색(frameBorder)으로 찍는다.
    const headR = Math.min(pw, ph) * 0.22;
    g.circle(x0 + pw / 2, y0 + headR + 2, headR).fill(pal.frameBorder);
    g.poly([
      x0 + pw * 0.15, y0 + ph,
      x0 + pw * 0.85, y0 + ph,
      x0 + pw * 0.7, y0 + ph * 0.58,
      x0 + pw * 0.3, y0 + ph * 0.58,
    ]).fill(pal.frameBorder);
  }

  /** 초상 텍스처를 "contain" 방식(비율 유지, nearest 스케일)으로 사진칸에
   * 넣는다. 실루엣은 뒤에 남겨 두고 사진을 그 위에 얹는다(카드에 붙으므로
   * 카드와 같이 기운다). */
  showPhoto(texture: Texture): void {
    this.photo?.destroy();
    texture.source.scaleMode = "nearest";
    const sprite = new Sprite(texture);
    sprite.anchor.set(0.5);
    const scale = Math.min(this.photoW / texture.width, this.photoH / texture.height);
    sprite.width = Math.round(texture.width * scale);
    sprite.height = Math.round(texture.height * scale);
    sprite.position.set(
      Math.round(this.photoX + this.photoW / 2),
      Math.round(this.photoY + this.photoH / 2),
    );
    this.card.addChild(sprite);
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
