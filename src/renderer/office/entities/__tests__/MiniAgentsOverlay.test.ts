import { describe, expect, it } from "vitest";
import { BufferImageSource, Texture, type Sprite } from "pixi.js";
import { MiniAgentsOverlay } from "../MiniAgentsOverlay";

const tex = (): Texture =>
  new Texture({
    source: new BufferImageSource({ resource: new Uint8Array([255, 255, 255, 255]), width: 1, height: 1, label: "t" }),
    label: "t",
  });

const visibleCount = (o: MiniAgentsOverlay): number =>
  o.root.children.filter((c) => c.visible).length;

describe("MiniAgentsOverlay", () => {
  it("항상 3개 스프라이트를 미리 만들고, 초기 표시 수는 0", () => {
    const o = new MiniAgentsOverlay(tex(), 1);
    expect(o.root.children.length).toBe(3);
    expect(visibleCount(o)).toBe(0);
  });

  it("setCount(2)는 2개만 보이게 한다", () => {
    const o = new MiniAgentsOverlay(tex(), 1);
    o.setCount(2);
    expect(visibleCount(o)).toBe(2);
  });

  it("setCount는 3에서 캡한다", () => {
    const o = new MiniAgentsOverlay(tex(), 1);
    o.setCount(7);
    expect(visibleCount(o)).toBe(3);
  });

  it("setCount(0)/음수는 모두 숨긴다", () => {
    const o = new MiniAgentsOverlay(tex(), 1);
    o.setCount(3);
    o.setCount(-4);
    expect(visibleCount(o)).toBe(0);
  });

  it("스케일은 부모 spriteScale의 절반, alpha 0.75", () => {
    const o = new MiniAgentsOverlay(tex(), 2);
    const s = o.root.children[0] as Sprite;
    expect(s.scale.x).toBeCloseTo(1); // 2 * 0.5
    expect(s.alpha).toBeCloseTo(0.75);
  });

  it("update(dt)는 보이는 미니의 y를 밥(bob)으로 흔든다", () => {
    const o = new MiniAgentsOverlay(tex(), 1);
    o.setCount(1);
    const s = o.root.children[0] as Sprite;
    const y0 = s.y;
    o.update(300); // 주기 1200ms의 1/4 → sin 최대 근처로 이동
    expect(s.y).not.toBeCloseTo(y0);
  });

  it("destroy()는 공유 텍스처를 파괴하지 않는다(부모 소유 — texture:false)", () => {
    const sharedTexture = tex();
    const o = new MiniAgentsOverlay(sharedTexture, 1);
    o.destroy();
    expect(sharedTexture.destroyed).toBe(false);
  });
});
