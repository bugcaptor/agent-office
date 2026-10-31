// src/renderer/shortcuts/__tests__/accelerator.test.ts
//
// 단축키 판정: 물리 키(code) 기준 일치, AltGr 글자 입력 배제, 쓸 수 없는 키 거르기.
import { describe, expect, it } from "vitest";
import {
  acceleratorFromEvent,
  acceleratorProblem,
  formatAccelerator,
  matchesAccelerator,
  recommendedAgentShortcut,
  recommendedNotifyShortcut,
} from "../accelerator";

function key(code: string, key: string, mods: Partial<Record<"ctrl" | "alt" | "shift" | "meta", boolean>> = {}) {
  return {
    code,
    key,
    ctrlKey: !!mods.ctrl,
    altKey: !!mods.alt,
    shiftKey: !!mods.shift,
    metaKey: !!mods.meta,
  };
}

describe("acceleratorFromEvent / matchesAccelerator", () => {
  it("수식키만 눌린 상태는 단축키가 아니다", () => {
    expect(acceleratorFromEvent(key("AltLeft", "Alt", { alt: true }))).toBeNull();
    expect(acceleratorFromEvent(key("MetaLeft", "Meta", { meta: true }))).toBeNull();
  });

  it("mac은 Option이 글자를 바꿔도(⌥1 = ¡) 물리 키로 맞춘다", () => {
    const e = key("Digit1", "¡", { alt: true, meta: true });
    expect(matchesAccelerator(e, recommendedAgentShortcut(0, true)!, true)).toBe(true);
  });

  it("Windows·Linux에서 Ctrl+Alt+숫자가 다른 글자를 찍으면(AltGr) 단축키로 치지 않는다", () => {
    const acc = "Ctrl+Alt+Digit7";
    expect(matchesAccelerator(key("Digit7", "{", { ctrl: true, alt: true }), acc, false)).toBe(false);
    expect(matchesAccelerator(key("Digit7", "7", { ctrl: true, alt: true }), acc, false)).toBe(true);
    expect(matchesAccelerator(key("KeyJ", "j", { ctrl: true, alt: true }), recommendedNotifyShortcut(false), false)).toBe(true);
  });

  it("수식키가 하나라도 다르면 맞지 않는다", () => {
    const acc = recommendedAgentShortcut(1, false)!;
    expect(matchesAccelerator(key("Digit2", "2", { ctrl: true }), acc, false)).toBe(false);
    expect(matchesAccelerator(key("Digit2", "@", { ctrl: true, alt: true, shift: true }), acc, false)).toBe(false);
    expect(matchesAccelerator(key("Digit2", "2", { ctrl: true, alt: true }), "", false)).toBe(false);
  });
});

describe("recommended", () => {
  it("10번째 에이전트부터는 추천값이 없다", () => {
    expect(recommendedAgentShortcut(8, true)).not.toBeNull();
    expect(recommendedAgentShortcut(9, true)).toBeNull();
  });

  it("추천값은 모두 쓸 수 있는 키다", () => {
    for (const isMac of [true, false]) {
      for (let i = 0; i < 9; i++) expect(acceleratorProblem(recommendedAgentShortcut(i, isMac)!, isMac)).toBeNull();
      expect(acceleratorProblem(recommendedNotifyShortcut(isMac), isMac)).toBeNull();
    }
  });
});

describe("acceleratorProblem", () => {
  it("Ctrl·Alt·⌘ 없이 Shift만이면 needsModifier", () => {
    expect(acceleratorProblem("Shift+KeyA", false)).toBe("needsModifier");
    expect(acceleratorProblem("F5", true)).toBe("needsModifier");
  });

  it("셸이 받는 Ctrl+글자·Alt+글자는 terminalKey", () => {
    expect(acceleratorProblem("Ctrl+KeyR", false)).toBe("terminalKey");
    expect(acceleratorProblem("Ctrl+Shift+KeyR", true)).toBe("terminalKey");
    expect(acceleratorProblem("Alt+KeyB", true)).toBe("terminalKey");
    expect(acceleratorProblem("Ctrl+F5", false)).toBeNull();
  });

  it("앱 탭 키·OS 키는 reserved", () => {
    expect(acceleratorProblem("Meta+Digit3", true)).toBe("reserved");
    expect(acceleratorProblem("Meta+KeyW", true)).toBe("reserved");
    expect(acceleratorProblem("Ctrl+Digit3", false)).toBe("reserved");
    expect(acceleratorProblem("Ctrl+Alt+Delete", false)).toBe("reserved");
    // Windows·Linux의 Win/Super 조합은 OS가 가져간다.
    expect(acceleratorProblem("Alt+Meta+KeyJ", false)).toBe("reserved");
  });
});

describe("formatAccelerator", () => {
  it("mac은 기호, 그 밖은 이름으로 표시한다", () => {
    expect(formatAccelerator("Ctrl+Alt+Shift+Meta+KeyJ", true)).toBe("⌃⌥⇧⌘J");
    expect(formatAccelerator("Ctrl+Alt+Digit1", false)).toBe("Ctrl+Alt+1");
  });
});
