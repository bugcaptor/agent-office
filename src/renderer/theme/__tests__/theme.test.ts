// @vitest-environment jsdom
//
// src/renderer/theme/__tests__/theme.test.ts
//
// 테마 시스템 단위 테스트:
// - 레지스트리 무결성: 모든 테마가 CSS 토큰/타일 팔레트 키를 빠짐없이 정의
// - midnight = 테마 도입 이전의 원본 값 그대로(룩 보존 계약)
// - applyTheme: data-theme + 인라인 커스텀 프로퍼티 + localStorage 영속
// - loadStoredThemeId: 유효값 복원 / 무효·부재 시 daylight 폴백
// - store.setTheme: 상태 갱신 + DOM 적용까지 한 번에

import { afterEach, describe, expect, it, vi } from "vitest";
import {
  CSS_TOKEN_KEYS,
  DEFAULT_THEME_ID,
  THEMES,
  THEME_ORDER,
  TILE_PALETTE_KEYS,
  isThemeId,
  nextThemeId,
} from "../themes";
import { THEME_STORAGE_KEY, applyTheme, loadStoredThemeId } from "../applyTheme";

const ALL_IDS = Object.keys(THEMES) as Array<keyof typeof THEMES>;

// 이 프로젝트의 vitest jsdom 환경은 `localStorage` 전역을 노출하지 않는다
// (오리진 설정상 접근 불가) — 인메모리 스텁으로 대체해 영속 계약(키/값 호출)을
// 검증한다. 모듈 최상단에서 스텁해야 setTheme 테스트의 appStore 동적 import도
// 같은 스텁을 본다.
const storage = new Map<string, string>();
vi.stubGlobal("localStorage", {
  getItem: (k: string) => storage.get(k) ?? null,
  setItem: (k: string, v: string) => void storage.set(k, String(v)),
  removeItem: (k: string) => void storage.delete(k),
  clear: () => storage.clear(),
});

afterEach(() => {
  localStorage.clear();
  const root = document.documentElement;
  delete root.dataset.theme;
  root.removeAttribute("style");
});

describe("THEMES 레지스트리 무결성", () => {
  it("THEME_ORDER가 레지스트리의 모든 테마를 정확히 한 번씩 순회한다", () => {
    expect([...THEME_ORDER].sort()).toEqual([...ALL_IDS].sort());
    expect(THEME_ORDER[0]).toBe(DEFAULT_THEME_ID); // 기본 테마가 순환의 시작점
  });

  it("모든 테마가 id 일치 + 비어있지 않은 한국어 라벨을 가진다", () => {
    for (const id of ALL_IDS) {
      expect(THEMES[id].id).toBe(id);
      expect(THEMES[id].label.length).toBeGreaterThan(0);
    }
  });

  it("모든 테마의 css 맵이 CSS 토큰 키 전부를 #rrggbb 값으로 정의한다", () => {
    for (const id of ALL_IDS) {
      for (const key of CSS_TOKEN_KEYS) {
        expect(THEMES[id].css[key], `${id} ${key}`).toMatch(/^#[0-9a-f]{6}$/);
      }
    }
  });

  it("모든 테마의 pixi 팔레트가 타일 키 전부 + background를 유효한 0xRRGGBB로 정의한다", () => {
    for (const id of ALL_IDS) {
      for (const key of [...TILE_PALETTE_KEYS, "background"] as const) {
        const v = THEMES[id].pixi[key];
        expect(Number.isInteger(v), `${id} ${key}`).toBe(true);
        expect(v).toBeGreaterThanOrEqual(0x000000);
        expect(v).toBeLessThanOrEqual(0xffffff);
      }
    }
  });

  it("midnight은 테마 도입 이전의 원본 값을 그대로 보존한다(오피스 리디자인으로 추가된 타일 팔레트 키 제외)", () => {
    // 원본: tokens.css(구 :root) + TileRenderer.PAL(구 상수) + 배경 0x1b1b24.
    // Phase A(오피스 리디자인)에서 plant/counter/table 등 신규 키가 추가되었으므로
    // 원본 키셋만 부분 일치(toMatchObject)로 검증하고, 값 자체는 그대로 보존한다.
    expect(THEMES.midnight.css).toEqual({
      "--bg-base": "#12131a",
      "--bg-panel": "#1e2130",
      "--bg-panel-hi": "#2a2e42",
      "--border-lite": "#4a5170",
      "--border-dark": "#0a0b12",
      "--accent": "#7cff6b",
      "--accent-warn": "#ffd866",
      "--accent-error": "#ff5c6a",
      "--text": "#c8d0e0",
      "--text-dim": "#8a93b0",
    });
    expect(THEMES.midnight.pixi).toMatchObject({
      background: 0x1b1b24,
      floorA: 0x3a3a4a,
      floorB: 0x34343f,
      floorDot: 0x2e2e38,
      wall: 0x22222c,
      wallTop: 0x3a3a48,
      desk: 0x8a5a34,
      deskEdge: 0x6b4526,
      deskTop: 0xa9723f,
      rug: 0x2f5d5b,
      rugEdge: 0x264b49,
    });
  });

  it("isThemeId / nextThemeId", () => {
    expect(isThemeId("daylight")).toBe(true);
    expect(isThemeId("neon")).toBe(false);
    expect(isThemeId(null)).toBe(false);
    // 3개 테마 순환: 시작점으로 되돌아온다
    let id = DEFAULT_THEME_ID;
    const seen = new Set([id]);
    for (let i = 0; i < THEME_ORDER.length - 1; i++) seen.add((id = nextThemeId(id)));
    expect(seen.size).toBe(THEME_ORDER.length);
    expect(nextThemeId(id)).toBe(DEFAULT_THEME_ID);
  });
});

describe("applyTheme", () => {
  it("documentElement에 data-theme와 모든 CSS 토큰을 주입하고 localStorage에 영속한다", () => {
    applyTheme("sakura");
    const root = document.documentElement;
    expect(root.dataset.theme).toBe("sakura");
    for (const key of CSS_TOKEN_KEYS) {
      expect(root.style.getPropertyValue(key)).toBe(THEMES.sakura.css[key]);
    }
    expect(localStorage.getItem(THEME_STORAGE_KEY)).toBe("sakura");
  });

  it("재적용 시 이전 테마 값을 전부 덮어쓴다", () => {
    applyTheme("midnight");
    applyTheme("daylight");
    const root = document.documentElement;
    expect(root.dataset.theme).toBe("daylight");
    expect(root.style.getPropertyValue("--bg-base")).toBe(THEMES.daylight.css["--bg-base"]);
    expect(localStorage.getItem(THEME_STORAGE_KEY)).toBe("daylight");
  });
});

describe("loadStoredThemeId", () => {
  it("저장된 유효 id를 복원한다", () => {
    localStorage.setItem(THEME_STORAGE_KEY, "midnight");
    expect(loadStoredThemeId()).toBe("midnight");
  });

  it("부재/무효 값이면 daylight로 폴백한다", () => {
    expect(loadStoredThemeId()).toBe(DEFAULT_THEME_ID);
    localStorage.setItem(THEME_STORAGE_KEY, "neon");
    expect(loadStoredThemeId()).toBe(DEFAULT_THEME_ID);
  });
});

describe("store.setTheme", () => {
  it("상태 갱신 + DOM 적용 + 영속을 한 번에 수행한다", async () => {
    const { useAppStore } = await import("../../store/appStore");
    useAppStore.getState().setTheme("sakura");
    expect(useAppStore.getState().theme).toBe("sakura");
    expect(document.documentElement.dataset.theme).toBe("sakura");
    expect(localStorage.getItem(THEME_STORAGE_KEY)).toBe("sakura");
    // 원복(모듈 스토어는 테스트 파일 내에서 공유되므로 기본값으로 되돌린다)
    useAppStore.getState().setTheme(DEFAULT_THEME_ID);
  });
});
