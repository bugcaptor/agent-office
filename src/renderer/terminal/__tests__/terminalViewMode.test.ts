// src/renderer/terminal/__tests__/terminalViewMode.test.ts
//
// 뷰 모드(이슈 #69) 순수 로직: 순환 순서, 타입 가드, localStorage 영속/복원.
// node 환경 — localStorage를 최소 스텁으로 주입해 영속 경로를 검증한다.
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import {
  TERMINAL_VIEW_MODE_STORAGE_KEY,
  isTerminalViewMode,
  loadStoredTerminalViewMode,
  nextTerminalViewMode,
  persistTerminalViewMode,
} from "../terminalViewMode";

describe("nextTerminalViewMode", () => {
  it("windowed↔filled 로 토글한다", () => {
    expect(nextTerminalViewMode("windowed")).toBe("filled");
    expect(nextTerminalViewMode("filled")).toBe("windowed");
  });
});

describe("isTerminalViewMode", () => {
  it("두 값만 참, 나머지는 거짓", () => {
    expect(isTerminalViewMode("windowed")).toBe(true);
    expect(isTerminalViewMode("filled")).toBe(true);
    expect(isTerminalViewMode("fullscreen")).toBe(false);
    expect(isTerminalViewMode("maximized")).toBe(false);
    expect(isTerminalViewMode(null)).toBe(false);
    expect(isTerminalViewMode(undefined)).toBe(false);
  });
});

describe("영속/복원", () => {
  const store = new Map<string, string>();
  const original = (globalThis as { localStorage?: Storage }).localStorage;

  beforeEach(() => {
    store.clear();
    (globalThis as { localStorage?: unknown }).localStorage = {
      getItem: (k: string) => (store.has(k) ? store.get(k)! : null),
      setItem: (k: string, v: string) => void store.set(k, v),
      removeItem: (k: string) => void store.delete(k),
      clear: () => store.clear(),
      key: () => null,
      length: 0,
    };
  });

  afterEach(() => {
    (globalThis as { localStorage?: unknown }).localStorage = original;
  });

  it("persist 후 load가 같은 값을 돌려준다", () => {
    persistTerminalViewMode("filled");
    expect(store.get(TERMINAL_VIEW_MODE_STORAGE_KEY)).toBe("filled");
    expect(loadStoredTerminalViewMode()).toBe("filled");
  });

  it("저장값이 없거나 알 수 없으면 windowed로 폴백", () => {
    expect(loadStoredTerminalViewMode()).toBe("windowed");
    store.set(TERMINAL_VIEW_MODE_STORAGE_KEY, "garbage");
    expect(loadStoredTerminalViewMode()).toBe("windowed");
  });
});

describe("localStorage 부재(node) 안전성", () => {
  const original = (globalThis as { localStorage?: Storage }).localStorage;
  beforeEach(() => {
    delete (globalThis as { localStorage?: unknown }).localStorage;
  });
  afterEach(() => {
    (globalThis as { localStorage?: unknown }).localStorage = original;
  });

  it("load는 windowed, persist는 던지지 않는다", () => {
    expect(loadStoredTerminalViewMode()).toBe("windowed");
    expect(() => persistTerminalViewMode("filled")).not.toThrow();
  });
});
