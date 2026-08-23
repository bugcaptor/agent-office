// src/renderer/terminal/palettes.ts
//
// 터미널 전용 색 팔레트 레지스트리 — 앱 테마(theme/themes.ts)와 짝이 없는,
// "터미널 색상" 셀렉터에서만 고를 수 있는 팔레트들.
//
// 앱 테마의 xterm 팔레트는 CSS 토큰/픽시 타일과 한 벌로 디자인된 것이라
// 테마마다 하나뿐이다. 반면 여기 있는 팔레트는 터미널 화면에만 적용되므로
// 씬·패널 색을 만들 필요가 없다 — 그래서 ThemeDef가 아니라 ITheme만 든
// 가벼운 레코드로 따로 둔다. 해석/영속은 theme.ts가 두 레지스트리를 함께 본다.
//
// 값은 Catppuccin 공식 Windows Terminal 포트(catppuccin/windows-terminal)의
// 스킴을 그대로 옮긴 것이다. 정체성이 곧 존재 이유이므로 우리 취향으로
// 보정하지 않는다 — 앱 테마에 걸린 대비 계약(유채색 6색이 ANSI black 위
// 배경으로도 3:1)은 여기 적용하지 않는다. 특히 latte는 유채색을 세그먼트
// 배경으로 쓰는 프롬프트(agnoster류)에서 글자 대비가 낮다(빨강 1.15:1).
// 밝은 배경을 원하면서 그런 프롬프트를 쓴다면 앱 테마의 "밝음"이 낫다.
import type { ITheme } from "@xterm/xterm";

/** 배경/전경은 필수 — theme.ts가 `--term-bg`를 항상 얻을 수 있어야 한다. */
export type XtermPalette = ITheme & { background: string; foreground: string };

export interface XtermPaletteDef {
  id: XtermPaletteId;
  /** 셀렉터에 그대로 노출되는 라벨. */
  label: string;
  xterm: XtermPalette;
}

export type XtermPaletteId =
  | "catppuccin-latte"
  | "catppuccin-frappe"
  | "catppuccin-macchiato"
  | "catppuccin-mocha";

export const XTERM_PALETTES: Record<XtermPaletteId, XtermPaletteDef> = {
  // 유일한 밝은 플레이버.
  "catppuccin-latte": {
    id: "catppuccin-latte",
    label: "카푸치노 라떼",
    xterm: {
      background: "#eff1f5",
      foreground: "#4c4f69",
      cursor: "#dc8a78",
      cursorAccent: "#eff1f5",
      selectionBackground: "#acb0be",
      black: "#5c5f77",
      red: "#d20f39",
      green: "#40a02b",
      yellow: "#df8e1d",
      blue: "#1e66f5",
      magenta: "#ea76cb",
      cyan: "#179299",
      white: "#acb0be",
      brightBlack: "#acb0be",
      brightRed: "#d20f39",
      brightGreen: "#40a02b",
      brightYellow: "#df8e1d",
      brightBlue: "#1e66f5",
      brightMagenta: "#ea76cb",
      brightCyan: "#179299",
      brightWhite: "#bcc0cc",
    },
  },
  // 어두운 세 플레이버는 frappe → macchiato → mocha 순으로 더 어두워진다.
  "catppuccin-frappe": {
    id: "catppuccin-frappe",
    label: "카푸치노 프라페",
    xterm: {
      background: "#303446",
      foreground: "#c6d0f5",
      cursor: "#f2d5cf",
      cursorAccent: "#303446",
      selectionBackground: "#626880",
      black: "#51576d",
      red: "#e78284",
      green: "#a6d189",
      yellow: "#e5c890",
      blue: "#8caaee",
      magenta: "#f4b8e4",
      cyan: "#81c8be",
      white: "#b5bfe2",
      brightBlack: "#626880",
      brightRed: "#e78284",
      brightGreen: "#a6d189",
      brightYellow: "#e5c890",
      brightBlue: "#8caaee",
      brightMagenta: "#f4b8e4",
      brightCyan: "#81c8be",
      brightWhite: "#a5adce",
    },
  },
  "catppuccin-macchiato": {
    id: "catppuccin-macchiato",
    label: "카푸치노 마키아토",
    xterm: {
      background: "#24273a",
      foreground: "#cad3f5",
      cursor: "#f4dbd6",
      cursorAccent: "#24273a",
      selectionBackground: "#5b6078",
      black: "#494d64",
      red: "#ed8796",
      green: "#a6da95",
      yellow: "#eed49f",
      blue: "#8aadf4",
      magenta: "#f5bde6",
      cyan: "#8bd5ca",
      white: "#b8c0e0",
      brightBlack: "#5b6078",
      brightRed: "#ed8796",
      brightGreen: "#a6da95",
      brightYellow: "#eed49f",
      brightBlue: "#8aadf4",
      brightMagenta: "#f5bde6",
      brightCyan: "#8bd5ca",
      brightWhite: "#a5adcb",
    },
  },
  "catppuccin-mocha": {
    id: "catppuccin-mocha",
    label: "카푸치노 모카",
    xterm: {
      background: "#1e1e2e",
      foreground: "#cdd6f4",
      cursor: "#f5e0dc",
      cursorAccent: "#1e1e2e",
      selectionBackground: "#585b70",
      black: "#45475a",
      red: "#f38ba8",
      green: "#a6e3a1",
      yellow: "#f9e2af",
      blue: "#89b4fa",
      magenta: "#f5c2e7",
      cyan: "#94e2d5",
      white: "#bac2de",
      brightBlack: "#585b70",
      brightRed: "#f38ba8",
      brightGreen: "#a6e3a1",
      brightYellow: "#f9e2af",
      brightBlue: "#89b4fa",
      brightMagenta: "#f5c2e7",
      brightCyan: "#94e2d5",
      brightWhite: "#a6adc8",
    },
  },
};

/** 셀렉터 표시 순서(밝은 것부터). */
export const XTERM_PALETTE_ORDER: readonly XtermPaletteId[] = [
  "catppuccin-latte",
  "catppuccin-frappe",
  "catppuccin-macchiato",
  "catppuccin-mocha",
];

export function isXtermPaletteId(v: unknown): v is XtermPaletteId {
  return typeof v === "string" && v in XTERM_PALETTES;
}
