// src/renderer/terminal/__tests__/xtermTheme.test.ts
//
// 터미널 색상 선택(theme.ts)의 순수 로직: 타입 가드, auto/고정 해석,
// localStorage 영속/복원. terminalViewMode.test.ts와 같은 패턴(node 환경 +
// 최소 localStorage 스텁)으로 쓴다.
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { THEMES } from "../../theme/themes";
import {
  DEFAULT_XTERM_THEME_OVERRIDE,
  XTERM_THEME_STORAGE_KEY,
  effectiveXtermThemeId,
  isXtermThemeOverride,
  loadStoredXtermThemeOverride,
  persistXtermThemeOverride,
  resolveXtermTheme,
  storedXtermBackground,
  xtermBackground,
} from "../theme";

describe("isXtermThemeOverride", () => {
  it("auto와 실재하는 테마 id만 참", () => {
    expect(isXtermThemeOverride("auto")).toBe(true);
    expect(isXtermThemeOverride("pipboy")).toBe(true);
    expect(isXtermThemeOverride("daylight")).toBe(true);
    expect(isXtermThemeOverride("neon")).toBe(false);
    expect(isXtermThemeOverride("")).toBe(false);
    expect(isXtermThemeOverride(null)).toBe(false);
    expect(isXtermThemeOverride(undefined)).toBe(false);
  });
});

describe("해석(auto / 고정)", () => {
  it("auto면 앱 테마를 그대로 따른다", () => {
    expect(effectiveXtermThemeId("sakura", "auto")).toBe("sakura");
    expect(resolveXtermTheme("sakura", "auto")).toBe(THEMES.sakura.xterm);
    expect(xtermBackground("sakura", "auto")).toBe(THEMES.sakura.xterm.background);
  });

  it("고정이면 앱 테마와 무관하게 그 테마의 팔레트를 쓴다", () => {
    expect(effectiveXtermThemeId("daylight", "midnight")).toBe("midnight");
    expect(resolveXtermTheme("daylight", "midnight")).toBe(THEMES.midnight.xterm);
    expect(xtermBackground("daylight", "pipboy")).toBe(THEMES.pipboy.xterm.background);
  });

  it("모든 테마가 auto 경로에서 자기 팔레트로 해석된다", () => {
    for (const id of Object.keys(THEMES) as Array<keyof typeof THEMES>) {
      expect(resolveXtermTheme(id, "auto")).toBe(THEMES[id].xterm);
    }
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
    persistXtermThemeOverride("pipboy");
    expect(store.get(XTERM_THEME_STORAGE_KEY)).toBe("pipboy");
    expect(loadStoredXtermThemeOverride()).toBe("pipboy");

    persistXtermThemeOverride("auto");
    expect(loadStoredXtermThemeOverride()).toBe("auto");
  });

  it("저장값이 없거나 알 수 없으면 auto로 폴백", () => {
    expect(loadStoredXtermThemeOverride()).toBe(DEFAULT_XTERM_THEME_OVERRIDE);
    store.set(XTERM_THEME_STORAGE_KEY, "garbage");
    expect(loadStoredXtermThemeOverride()).toBe("auto");
  });

  it("storedXtermBackground는 저장된 선택을 반영한다", () => {
    expect(storedXtermBackground("daylight")).toBe(THEMES.daylight.xterm.background);
    persistXtermThemeOverride("midnight");
    expect(storedXtermBackground("daylight")).toBe(THEMES.midnight.xterm.background);
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

  it("load는 auto, persist는 던지지 않는다", () => {
    expect(loadStoredXtermThemeOverride()).toBe("auto");
    expect(() => persistXtermThemeOverride("pipboy")).not.toThrow();
  });
});
