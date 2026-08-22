// src/renderer/office/entities/TextBubbleOverlay.ts
//
// 동료 대화(docs/agent-talk-design.md §7) 말풍선. ThinkingOverlay 패턴을 그대로
// 따른다 — Pixi Graphics + dt 구동 + setVisible/update/destroy, 소유·파괴는
// `CharacterEntity`.
//
// ThinkingOverlay와 다른 두 가지:
// 1) **글씨가 들어간다.** 글씨는 월드 배율에서 비정수 리샘플링으로 깨지므로,
//    OfficeScene의 휴가 팻말과 같은 규칙으로 루트 배율을 1/S로 상쇄한다
//    (`setRenderScale`). 그래서 이 안의 좌표·크기는 전부 **화면 px**다.
// 2) **수명이 있다.** show()가 남은 시간을 채우고 update(dt)가 깎아 0에서
//    스스로 숨는다(외부 타이머·Date.now 없음). 말풍선이 겹치면 마지막 것으로
//    교체된다 — show()가 남은 시간을 다시 채우기만 하면 되므로 큐가 없다.
//
// 폭 계산은 Pixi `Text` 계측(`.width`)을 쓰지 않는다 — CanvasTextMetrics가
// document/canvas를 요구해서 node 환경 테스트에서 터진다. 대신 글자 폭을
// 결정적으로 어림한다(한글·전각 1배, 그 외 0.5배).

import { Container, Graphics, Text } from "pixi.js";
import type { TalkBubbleTone } from "../bus";

/** 본문 표시 상한(글자). 넘치면 말줄임표로 자른다. */
export const TALK_BUBBLE_MAX_CHARS = 40;

const FONT_SIZE = 11; // 휴가 팻말과 같은 크기(화면 px)
const PAD_X = 4;
const PAD_Y = 3;
const TAIL_W = 4;
const TAIL_H = 4;
const MIN_BOX_W = 14;

/** 발신 말풍선 수명(ms). 설계 §7의 "3~5초" 중간값. */
export const SAY_DURATION_MS = 4500;
/** 수신 도착 표시 수명(ms). 발신보다 짧게 — 도착만 알리는 이펙트다. */
export const HEAR_DURATION_MS = 2000;

/** 톤별 색(테마 비의존 — ExclamationOverlay/ThinkingOverlay와 같은 관례). */
const TONE_COLORS: Record<TalkBubbleTone, { fill: number; stroke: number; text: number }> = {
  // 발신: 생각 말풍선과 같은 흰 바탕(같은 캐릭터의 같은 계열 표시).
  say: { fill: 0xffffff, stroke: 0x555555, text: 0x333333 },
  // 수신: "!" 뱃지와 같은 노란 계열 — 발신 말풍선과 한눈에 구분된다.
  hear: { fill: 0xffe9a8, stroke: 0x8a5a00, text: 0x3a2600 },
};

/**
 * 신뢰 불가 본문을 표시용 한 줄로 정리한다. 개행·탭·제어문자를 공백으로
 * 바꾸고, 연속 공백을 접고, `max`자를 넘으면 말줄임표로 자른다.
 * 결과가 비면 빈 문자열 — 호출자는 이때 말풍선을 띄우지 않는다.
 */
export function bubbleText(raw: string, max = TALK_BUBBLE_MAX_CHARS): string {
  const flat = raw
    // C0/C1 제어문자(개행·탭 포함) → 공백.
    .replace(/[\u0000-\u001f\u007f-\u009f]/g, " ")
    // 폭 0 / 방향 제어 문자는 제거 — 보이지 않으면서 글자 수만 먹는다.
    .replace(/[\u200b-\u200f\u2028\u2029\u202a-\u202e\u2066-\u2069\ufeff]/g, "")
    .replace(/\s+/g, " ")
    .trim();
  if (flat.length <= max) return flat;
  return `${flat.slice(0, max)}…`;
}

/** 전각(한글·한자·가나·전각기호·이모지)인가 — 폭 어림의 유일한 분기. */
function isWide(ch: string): boolean {
  const c = ch.codePointAt(0) ?? 0;
  return (
    (c >= 0x1100 && c <= 0x115f) || // 한글 자모
    (c >= 0x2e80 && c <= 0xa4cf) || // CJK 부수~이수 부호
    (c >= 0xac00 && c <= 0xd7a3) || // 한글 음절
    (c >= 0xf900 && c <= 0xfaff) || // CJK 호환 한자
    (c >= 0xfe30 && c <= 0xfe6f) || // CJK 호환 형태
    (c >= 0xff00 && c <= 0xff60) || // 전각 ASCII
    (c >= 0xffe0 && c <= 0xffe6) ||
    c >= 0x1f000 // 이모지 평면
  );
}

/**
 * 말풍선 본문 폭(화면 px)을 결정적으로 어림한다. Pixi 계측을 쓰지 않는 이유는
 * 파일 헤더 주석 참고. 픽셀 폰트라 실제 폭과 몇 px 어긋나도 눈에 띄지 않는다.
 */
export function bubbleTextWidth(text: string, fontSize = FONT_SIZE): number {
  let w = 0;
  for (const ch of text) w += isWide(ch) ? fontSize : fontSize * 0.5;
  return w;
}

export class TextBubbleOverlay {
  readonly root = new Container();
  private bubble: Graphics;
  private label: Text;
  private remainingMs = 0;

  constructor() {
    this.bubble = new Graphics();
    this.root.addChild(this.bubble);

    this.label = new Text({
      text: "",
      style: { fontFamily: "DungGeunMo", fontSize: FONT_SIZE, fill: TONE_COLORS.say.text },
      resolution: 2, // 휴가 팻말과 같은 값 — 1/S 상쇄 후에도 또렷하게
    });
    this.label.anchor.set(0.5, 0.5);
    this.root.addChild(this.label);

    this.root.visible = false;
  }

  /**
   * 카메라 정수 스케일 S 반영. 루트를 1/S로 축소해 말풍선이 확대·축소와
   * 무관하게 같은 화면 크기로 보이게 한다(OfficeScene의 휴가 팻말과 같은 규칙).
   */
  setRenderScale(scale: number): void {
    const s = Math.max(1, Math.round(scale));
    this.root.scale.set(1 / s);
  }

  /**
   * 말풍선을 띄운다. 이미 떠 있으면 **마지막 것으로 교체**하고 수명을 다시
   * 채운다(큐잉 없음). 정리 결과가 빈 문자열이면 띄우지 않고 숨긴다.
   */
  show(rawText: string, tone: TalkBubbleTone): void {
    const text = bubbleText(rawText);
    if (text === "") {
      this.setVisible(false);
      return;
    }
    const c = TONE_COLORS[tone];
    this.label.text = text;
    this.label.style.fill = c.text;

    const boxW = Math.max(MIN_BOX_W, bubbleTextWidth(text) + PAD_X * 2);
    const boxH = FONT_SIZE + PAD_Y * 2;
    // 바닥(y=0)이 머리 쪽, 꼬리는 그 아래로 내려간다.
    this.bubble.clear();
    this.bubble
      .roundRect(-boxW / 2, -boxH, boxW, boxH, 3)
      .fill(c.fill)
      .stroke({ width: 1, color: c.stroke });
    this.bubble.poly([-TAIL_W / 2, 0, TAIL_W / 2, 0, 0, TAIL_H]).fill(c.fill);
    this.label.position.set(0, -boxH / 2);

    this.remainingMs = tone === "say" ? SAY_DURATION_MS : HEAR_DURATION_MS;
    this.root.visible = true;
  }

  setVisible(v: boolean): void {
    this.root.visible = v;
    if (!v) this.remainingMs = 0;
  }

  /** dt: ms. 수명을 깎아 0이 되면 스스로 숨는다. 숨은 동안은 no-op. */
  update(dt: number): void {
    if (!this.root.visible) return;
    this.remainingMs -= dt;
    if (this.remainingMs <= 0) this.setVisible(false);
  }

  destroy(): void {
    this.root.destroy({ children: true });
  }
}
