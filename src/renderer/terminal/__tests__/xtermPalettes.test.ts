// src/renderer/terminal/__tests__/xtermPalettes.test.ts
//
// 터미널 전용 팔레트 레지스트리(palettes.ts)의 무결성. 앱 테마 쪽 계약은
// theme/__tests__/theme.test.ts가 보고, 여기서는 그 계약 중 "터미널 화면에만
// 걸리는 것"만 검사한다:
// - 형식(#rrggbb) + ANSI 16색 누락 없음 — 빠지면 xterm이 기본 검정으로 떨어진다
// - 배경/전경 명도가 뒤집히지 않음
// - 앱 테마 id와 팔레트 id가 겹치지 않음(오버라이드 해석이 모호해진다)
//
// 유채색 6색의 "ANSI black 위 배경으로도 3:1" 계약은 여기 적용하지 않는다 —
// 값이 Catppuccin 공식 스킴의 이식이라 우리 취향으로 보정하지 않기 때문.
// 대신 전경 가독성 하한만 지킨다(palettes.ts 상단 주석 참고).
import { describe, expect, it } from "vitest";
import {
  XTERM_PALETTES,
  XTERM_PALETTE_ORDER,
  isXtermPaletteId,
} from "../palettes";
import { THEMES } from "../../theme/themes";

const ALL_IDS = Object.keys(XTERM_PALETTES) as Array<keyof typeof XTERM_PALETTES>;

const ANSI_16 = [
  "black",
  "red",
  "green",
  "yellow",
  "blue",
  "magenta",
  "cyan",
  "white",
  "brightBlack",
  "brightRed",
  "brightGreen",
  "brightYellow",
  "brightBlue",
  "brightMagenta",
  "brightCyan",
  "brightWhite",
] as const;

const relLum = (hex: string) => {
  const lin = (c: number) => (c <= 0.04045 ? c / 12.92 : ((c + 0.055) / 1.055) ** 2.4);
  const n = parseInt(hex.slice(1), 16);
  const [r, g, b] = [(n >> 16) & 0xff, (n >> 8) & 0xff, n & 0xff].map((v) => lin(v / 255));
  return 0.2126 * r + 0.7152 * g + 0.0722 * b;
};
const contrast = (a: string, b: string) => {
  const [hi, lo] = [relLum(a) + 0.05, relLum(b) + 0.05].sort((x, y) => y - x);
  return hi / lo;
};

describe("XTERM_PALETTES 레지스트리 무결성", () => {
  it("ORDER가 레지스트리 전부를 정확히 한 번씩 순회한다", () => {
    expect([...XTERM_PALETTE_ORDER].sort()).toEqual([...ALL_IDS].sort());
  });

  it("id 일치 + 비어있지 않은 라벨", () => {
    for (const id of ALL_IDS) {
      expect(XTERM_PALETTES[id].id).toBe(id);
      expect(XTERM_PALETTES[id].label.length).toBeGreaterThan(0);
    }
  });

  it("앱 테마 id와 팔레트 id가 겹치지 않는다", () => {
    // 겹치면 오버라이드 문자열 하나가 두 레지스트리를 동시에 가리켜
    // xtermPaletteOf의 분기(isThemeId)가 조용히 앱 테마 쪽만 고른다.
    for (const id of ALL_IDS) expect(id in THEMES).toBe(false);
  });

  it("모든 팔레트가 배경/전경/커서 + ANSI 16색을 #rrggbb로 정의한다", () => {
    for (const id of ALL_IDS) {
      const t = XTERM_PALETTES[id].xterm;
      for (const key of ["background", "foreground", "cursor", ...ANSI_16] as const) {
        expect(t[key], `${id} xterm.${key}`).toMatch(/^#[0-9a-f]{6}$/);
      }
    }
  });

  it("배경/전경 명도가 뒤집히지 않고 본문 대비가 충분하다", () => {
    const light: Array<keyof typeof XTERM_PALETTES> = ["catppuccin-latte"];
    for (const id of ALL_IDS) {
      const { background, foreground } = XTERM_PALETTES[id].xterm;
      const darkBg = relLum(background) < relLum(foreground);
      expect(darkBg, `${id} 배경/전경 명도`).toBe(!light.includes(id));
      expect(contrast(foreground, background), `${id} 본문 대비`).toBeGreaterThanOrEqual(4.5);
    }
  });

  it("유채색 6색이 전경으로 읽힌다(vs 터미널 배경)", () => {
    // 공식 스킴 이식이라 앱 테마의 3:1보다 느슨한 하한만 계약으로 둔다 —
    // latte의 노랑/자홍이 2.3:1로 가장 낮다.
    for (const id of ALL_IDS) {
      const t = XTERM_PALETTES[id].xterm;
      for (const key of ["red", "green", "yellow", "blue", "magenta", "cyan"] as const) {
        expect(
          contrast(t[key] as string, t.background),
          `${id} ${key} 전경 대비`
        ).toBeGreaterThanOrEqual(2.3);
      }
    }
  });

  it("isXtermPaletteId는 실재하는 팔레트만 참", () => {
    expect(isXtermPaletteId("catppuccin-mocha")).toBe(true);
    expect(isXtermPaletteId("midnight")).toBe(false);
    expect(isXtermPaletteId("auto")).toBe(false);
    expect(isXtermPaletteId(null)).toBe(false);
  });
});
