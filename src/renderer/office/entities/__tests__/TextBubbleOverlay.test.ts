// src/renderer/office/entities/__tests__/TextBubbleOverlay.test.ts
//
// 동료 대화 말풍선(docs/agent-talk-design.md §7). ThinkingOverlay.test.ts의
// 관례를 따른다 — 진짜 `Container`/`Graphics`/`Text`로 만들고(렌더러 없이도
// 생성·변형은 된다), dt만으로 결정적으로 검증한다(실타이머·Math.random 없음).
//
// 커버리지:
// - bubbleText: 개행·제어문자 정리 + 40자 절단, 빈 결과.
// - 표시/숨김 수명: show가 수명을 채우고 update(dt)가 깎아 스스로 숨는다.
// - 겹침: 나중 show가 앞의 것을 교체하고 수명을 다시 채운다.
// - 1/S 배율 상쇄, destroy.

import { describe, expect, it } from "vitest";
import type { Text } from "pixi.js";
import {
  HEAR_DURATION_MS,
  SAY_DURATION_MS,
  TALK_BUBBLE_MAX_CHARS,
  TextBubbleOverlay,
  bubbleText,
  bubbleTextWidth,
} from "../TextBubbleOverlay";

/** children 순서는 [bubble(Graphics), label(Text)]. */
const labelOf = (o: TextBubbleOverlay): Text => o.root.children[1] as Text;

describe("bubbleText: 신뢰 불가 본문 정리", () => {
  it("개행·탭·제어문자를 공백으로 접고 앞뒤를 자른다", () => {
    expect(bubbleText("  안녕\n\t하세요\r\n  ")).toBe("안녕 하세요");
    expect(bubbleText("a\u0001b\u0002c")).toBe("a b c");
  });

  it("폭 0 / 방향 제어 문자는 아예 제거한다", () => {
    expect(bubbleText("a\u200bb\u202ec")).toBe("abc");
  });

  it("상한을 넘으면 말줄임표로 자른다(상한 이하는 그대로)", () => {
    const long = "가".repeat(TALK_BUBBLE_MAX_CHARS + 10);
    const cut = bubbleText(long);
    expect(cut).toBe("가".repeat(TALK_BUBBLE_MAX_CHARS) + "…");
    expect(bubbleText("가".repeat(TALK_BUBBLE_MAX_CHARS))).toHaveLength(TALK_BUBBLE_MAX_CHARS);
  });

  it("공백뿐인 본문은 빈 문자열", () => {
    expect(bubbleText(" \n\t ")).toBe("");
  });
});

describe("bubbleTextWidth: 결정적 폭 어림", () => {
  it("한글은 전각(1배), ASCII는 반각(0.5배)으로 센다", () => {
    expect(bubbleTextWidth("가나", 10)).toBe(20);
    expect(bubbleTextWidth("ab", 10)).toBe(10);
  });
});

describe("TextBubbleOverlay: 표시 수명", () => {
  it("처음엔 숨어 있고, show가 본문을 넣고 띄운다", () => {
    const o = new TextBubbleOverlay();
    expect(o.root.visible).toBe(false);
    o.show("안녕하세요", "say");
    expect(o.root.visible).toBe(true);
    expect(labelOf(o).text).toBe("안녕하세요");
  });

  it("정리 결과가 비면 띄우지 않는다", () => {
    const o = new TextBubbleOverlay();
    o.show("   \n  ", "say");
    expect(o.root.visible).toBe(false);
  });

  it("발신(say) 수명이 다하면 스스로 숨는다", () => {
    const o = new TextBubbleOverlay();
    o.show("hi", "say");
    o.update(SAY_DURATION_MS - 1);
    expect(o.root.visible).toBe(true);
    o.update(1);
    expect(o.root.visible).toBe(false);
  });

  it("수신(hear) 표시는 발신보다 짧다", () => {
    const o = new TextBubbleOverlay();
    o.show("하나 →", "hear");
    o.update(HEAR_DURATION_MS);
    expect(o.root.visible).toBe(false);
    expect(HEAR_DURATION_MS).toBeLessThan(SAY_DURATION_MS);
  });

  it("숨은 동안 update는 no-op(다시 살아나지 않는다)", () => {
    const o = new TextBubbleOverlay();
    o.show("hi", "say");
    o.setVisible(false);
    o.update(10);
    expect(o.root.visible).toBe(false);
  });
});

describe("TextBubbleOverlay: 겹침", () => {
  it("나중 show가 앞의 것을 교체하고 수명을 다시 채운다", () => {
    const o = new TextBubbleOverlay();
    o.show("첫 번째", "say");
    o.update(SAY_DURATION_MS - 100);
    o.show("두 번째", "say");
    expect(labelOf(o).text).toBe("두 번째");
    // 앞의 것이 만료됐을 시점을 지나도 살아 있어야 한다.
    o.update(200);
    expect(o.root.visible).toBe(true);
  });
});

describe("TextBubbleOverlay: 배율·정리", () => {
  it("setRenderScale은 루트를 1/S로 상쇄한다", () => {
    const o = new TextBubbleOverlay();
    o.setRenderScale(4);
    expect(o.root.scale.x).toBeCloseTo(0.25);
    o.setRenderScale(0); // 하한 1로 물린다
    expect(o.root.scale.x).toBeCloseTo(1);
  });

  it("destroy가 루트를 파괴한다", () => {
    const o = new TextBubbleOverlay();
    o.destroy();
    expect(o.root.destroyed).toBe(true);
  });
});
