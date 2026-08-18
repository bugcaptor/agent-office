// src/renderer/theme/themes.ts
//
// 테마 레지스트리 — DOM(CSS 커스텀 프로퍼티)과 Pixi(오피스 씬 팔레트) 색의
// 단일 원천(single source of truth).
//
// - DOM 쪽: `applyTheme()`(./applyTheme.ts)가 각 테마의 `css` 맵을
//   documentElement 인라인 커스텀 프로퍼티로 주입한다. tokens.css의 :root
//   블록은 기본 테마(daylight)의 부트 폴백일 뿐이다.
// - Pixi 쪽: `pixi` 팔레트는 TileRenderer의 타일 색 전부 + 씬 배경색.
//   테마 전환 시 OfficeScene.setTheme()이 타일 텍스처를 재베이크한다.
// - xterm 쪽: `xterm` 팔레트는 터미널 화면(ITheme) 전용. 해석/영속은
//   terminal/theme.ts(resolveXtermTheme)가 맡고, 라이브 재도색은
//   TerminalRegistry.setTheme()이 한다. 유저가 "터미널 색상만 딴 테마로
//   고정"을 고를 수 있으므로 앱 테마와 1:1로 묶여 있지는 않다.
// - 캐릭터 스프라이트 팔레트(office/gen/palette.ts)는 에이전트별 절차
//   생성이므로 테마 대상이 아니다.
import type { ITheme } from "@xterm/xterm";

/** tokens.css가 선언하는 색 토큰 전부(--unit 같은 비색상 토큰 제외). */
export const CSS_TOKEN_KEYS = [
  "--bg-base",
  "--bg-panel",
  "--bg-panel-hi",
  "--border-lite",
  "--border-dark",
  "--accent",
  "--accent-warn",
  "--accent-error",
  "--text",
  "--text-dim",
] as const;
type CssTokenKey = (typeof CSS_TOKEN_KEYS)[number];

/** TileRenderer가 소비하는 타일 팔레트 키 전부(구 PAL 상수의 키셋). */
export const TILE_PALETTE_KEYS = [
  "floorA",
  "floorB",
  "floorDot",
  "wall",
  "wallTop",
  "desk",
  "deskEdge",
  "deskTop",
  "rug",
  "rugEdge",
  "plant",
  "plantPot",
  "counter",
  "counterTop",
  "table",
  "tableTop",
  "laptopLid",
  "laptopBody",
] as const;
type TilePaletteKey = (typeof TILE_PALETTE_KEYS)[number];

/** 타일 색만(0xRRGGBB). TileRenderer의 생성자 인자 타입. */
export type OfficeTilePalette = Record<TilePaletteKey, number>;

/** 타일 색 + 씬 배경색 + 씬 내 텍스트색. OfficeScene이 소비. */
export interface PixiThemePalette extends OfficeTilePalette {
  background: number;
  /** 씬 안에 그리는 텍스트(휴가중 팻말 등) — css `--text`의 미러. */
  text: number;
}

export type ThemeId = "daylight" | "midnight" | "sakura" | "pipboy";

export interface ThemeDef {
  id: ThemeId;
  /** 픽커 버튼/드롭다운에 그대로 노출되는 한국어 라벨. */
  label: string;
  css: Record<CssTokenKey, string>;
  pixi: PixiThemePalette;
  /**
   * 터미널(xterm) 팔레트. 앱 테마를 따르는 게 기본이지만 별도 고정도 가능.
   * `ITheme`은 전 필드가 optional이지만 background/foreground는 필수로 좁힌다 —
   * `--term-bg` 주입(applyTheme)이 배경색을 항상 얻을 수 있어야 하고, 전경 없이
   * 배경만 바뀌면 대비가 깨지기 때문.
   */
  xterm: ITheme & { background: string; foreground: string };
}

export const THEMES: Record<ThemeId, ThemeDef> = {
  // 밝고 따뜻한 주간 오피스 — 새 기본 테마.
  daylight: {
    id: "daylight",
    label: "밝음",
    css: {
      "--bg-base": "#f2ede2",
      "--bg-panel": "#fbf6ea",
      "--bg-panel-hi": "#f3ead6",
      "--border-lite": "#fffdf2",
      "--border-dark": "#6b5c44",
      "--accent": "#2f9e44",
      "--accent-warn": "#d9770a",
      "--accent-error": "#d6336c",
      "--text": "#3a3428",
      "--text-dim": "#857a66",
    },
    pixi: {
      background: 0xd9ccb4, // 맵 밖 레터박스: 바닥보다 살짝 어둡게 → 맵이 떠 보인다
      text: 0x3a3428, // = --text
      floorA: 0xe8dcc8, // 밝은 웜 우드 체커
      floorB: 0xe0d3bc,
      floorDot: 0xcfc0a4,
      wall: 0xf3edda, // 크림 벽
      wallTop: 0xfdfaf0,
      desk: 0xc08a4e, // 웜 오크 책상
      deskEdge: 0x96682f,
      deskTop: 0xd9a768,
      rug: 0x9fd3c0, // 민트/틸 러그
      rugEdge: 0x86bfab,
      plant: 0x4f9b5c, // 화분 잎(선명한 그린)
      plantPot: 0xb5713f, // 테라코타 화분
      counter: 0xd8c9a8, // 탕비실 카운터(웜 크림 캐비닛)
      counterTop: 0xf0e6cf, // 카운터 상판(밝은 스톤)
      table: 0xcaa06a, // 탕비실 테이블(밝은 우드)
      tableTop: 0xe0bc8a,
      laptopLid: 0x525a6e, // 랩탑 뚜껑 등판(슬레이트)
      laptopBody: 0x3a4050, // 랩탑 본체/디테일(더 어두운 슬레이트)
    },
    // 밝은 배경의 터미널 — ANSI 16색은 "밝을수록 잘 보인다"가 뒤집히므로
    // 노랑/흰색 계열을 어둡게 보정하고, bright는 더 진하게(대비 강화) 간다.
    xterm: {
      background: "#fbf6ea",
      foreground: "#3a3428",
      cursor: "#2f9e44",
      cursorAccent: "#fbf6ea",
      selectionBackground: "#cfc0a480",
      black: "#2b2620",
      red: "#c03a2e",
      green: "#2f7d3a",
      yellow: "#9a6b00",
      blue: "#1d5fd0",
      magenta: "#a3348f",
      cyan: "#0f7a76",
      white: "#7a7161",
      brightBlack: "#857a66",
      brightRed: "#d94f3d",
      brightGreen: "#3f9c4a",
      brightYellow: "#b8860b",
      brightBlue: "#3b7ddd",
      brightMagenta: "#c04aa8",
      brightCyan: "#14968f",
      brightWhite: "#3a3428",
    },
  },
  // 테마 도입 이전의 기존 룩 — tokens.css/PAL/배경 0x1b1b24를 그대로 보존.
  midnight: {
    id: "midnight",
    label: "미드나이트",
    css: {
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
    },
    pixi: {
      background: 0x1b1b24,
      text: 0xc8d0e0, // = --text
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
      plant: 0x3f6b46, // 화분 잎(어둡고 차분한 그린)
      plantPot: 0x4a3524, // 어두운 갈색 화분
      counter: 0x2c2e3a, // 탕비실 카운터(어두운 무채색 캐비닛)
      counterTop: 0x3d4152,
      table: 0x5a3d24, // 탕비실 테이블(어두운 우드)
      tableTop: 0x6f4d2e,
      laptopLid: 0x5b647e, // 랩탑 뚜껑 등판(어두운 배경 대비 살짝 밝은 슬레이트)
      laptopBody: 0x424a60,
    },
    // 터미널 테마 도입 이전의 유일한 팔레트(구 XTERM_THEME)를 값 그대로 이관.
    // "green-CRT meets modern dark" — 가독성 유지한 레트로.
    xterm: {
      background: "#12131a",
      foreground: "#c8d0e0",
      cursor: "#7cff6b",
      cursorAccent: "#12131a",
      selectionBackground: "#2b3350",
      black: "#1b1d2a",
      red: "#ff5c6a",
      green: "#7cff6b",
      yellow: "#ffd866",
      blue: "#6fb3ff",
      magenta: "#c792ea",
      cyan: "#5be7d6",
      white: "#c8d0e0",
      brightBlack: "#4a5170",
      brightRed: "#ff8791",
      brightGreen: "#a5ff9c",
      brightYellow: "#ffe699",
      brightBlue: "#a0cbff",
      brightMagenta: "#e0b7ff",
      brightCyan: "#8ff4e8",
      brightWhite: "#ffffff",
    },
  },
  // 파스텔 핑크 — 블러시 패널 + 플럼 텍스트.
  sakura: {
    id: "sakura",
    label: "벚꽃",
    css: {
      "--bg-base": "#f5e0e8",
      "--bg-panel": "#fcf0f5",
      "--bg-panel-hi": "#f7e3ec",
      "--border-lite": "#fff8fb",
      "--border-dark": "#7c4a60",
      "--accent": "#d6488c",
      "--accent-warn": "#cf7d22",
      "--accent-error": "#d64550",
      "--text": "#4a2b3c",
      "--text-dim": "#9a7286",
    },
    pixi: {
      background: 0xe9cfda,
      text: 0x4a2b3c, // = --text
      floorA: 0xf6e2ea, // 연분홍 체커 바닥
      floorB: 0xefd7e1,
      floorDot: 0xe2c2d0,
      wall: 0xe3bfce, // 로지 벽
      wallTop: 0xf5e0e9,
      desk: 0xc78a74, // 로즈 브라운 책상
      deskEdge: 0xa2685a,
      deskTop: 0xdea78f,
      rug: 0xafe0cb, // 민트 러그
      rugEdge: 0x94ccb5,
      plant: 0x7cb98a, // 화분 잎(파스텔 그린)
      plantPot: 0xc48a76, // 더스티 로즈 화분
      counter: 0xf0dde5, // 탕비실 카운터(파스텔 핑크 캐비닛)
      counterTop: 0xfbeef4,
      table: 0xc79482, // 탕비실 테이블(로즈 우드)
      tableTop: 0xdcb09c,
      laptopLid: 0x6e5d73, // 랩탑 뚜껑 등판(플럼 그레이)
      laptopBody: 0x504256,
    },
    // 밝은 블러시 배경 + 플럼 전경. daylight와 같은 이유로 노랑/흰색은 어둡게.
    xterm: {
      background: "#fcf0f5",
      foreground: "#4a2b3c",
      cursor: "#d6488c",
      cursorAccent: "#fcf0f5",
      selectionBackground: "#e2c2d080",
      black: "#3f2433",
      red: "#c33a5a",
      green: "#3f8f6a",
      yellow: "#b06a12",
      blue: "#4a63c0",
      magenta: "#c0398c",
      cyan: "#2f8a94",
      white: "#8a6d7c",
      brightBlack: "#9a7286",
      brightRed: "#e05575",
      brightGreen: "#4faa80",
      brightYellow: "#cf8a2a",
      brightBlue: "#6a80d8",
      brightMagenta: "#dd57a6",
      brightCyan: "#40a5b0",
      brightWhite: "#4a2b3c",
    },
  },
  // 폴아웃 Pip-Boy 오마주 — 인광 초록 모노크롬 CRT. 오피스 씬까지 통째로
  // 초록 계열이며, App 루트에 스캔라인 오버레이(.crt-overlay)가 겹친다.
  pipboy: {
    id: "pipboy",
    label: "핍보이",
    css: {
      "--bg-base": "#071209",
      "--bg-panel": "#0c1f10",
      "--bg-panel-hi": "#133019",
      "--border-lite": "#2e6b3a",
      "--border-dark": "#051007",
      "--accent": "#33ff66",
      "--accent-warn": "#d7ff3d",
      "--accent-error": "#ff7a5c",
      "--text": "#8dffa8",
      "--text-dim": "#43a35e",
    },
    // midnight의 명암 구조(바닥 < 벽 < 가구 < 하이라이트)를 그대로 초록으로 옮김.
    pixi: {
      background: 0x071209, // = --bg-base
      text: 0x8dffa8, // = --text
      floorA: 0x143b1f, // 어두운 초록 체커 바닥
      floorB: 0x113318,
      floorDot: 0x0d2814,
      wall: 0x0b2010, // 벽(바닥보다 어둡게 → 씬이 깊어 보인다)
      wallTop: 0x1a4a26,
      desk: 0x1f6b35, // 책상(중간 초록)
      deskEdge: 0x14532a,
      deskTop: 0x2b8a45,
      rug: 0x155040, // 러그(청록으로 살짝 기울인 초록)
      rugEdge: 0x104036,
      plant: 0x39c96a, // 화분 잎(가장 밝은 인광 초록)
      plantPot: 0x1b4a28,
      counter: 0x102a16, // 탕비실 카운터
      counterTop: 0x1c4a28,
      table: 0x1a5c2e, // 탕비실 테이블
      tableTop: 0x24743a,
      laptopLid: 0x2e8a55, // 랩탑(가구보다 한 단계 밝게)
      laptopBody: 0x1d5c39,
    },
    // 완전 단색은 로그 가독성을 죽인다(빨강 에러/노랑 경고 구분 불가) —
    // 색상(hue) 정체성은 남기되 채도를 낮추고 전부 초록 쪽으로 기울인다.
    xterm: {
      background: "#071209",
      foreground: "#66ff99",
      cursor: "#33ff66",
      cursorAccent: "#071209",
      selectionBackground: "#2e6b3a99",
      black: "#0a1c0f",
      red: "#ff6a4d",
      green: "#33ff66",
      yellow: "#c8e04a",
      blue: "#5ba8c4",
      magenta: "#c07fb0",
      cyan: "#45d6c0",
      white: "#8dffa8",
      brightBlack: "#2e6b3a",
      brightRed: "#ff9478",
      brightGreen: "#8dffb0",
      brightYellow: "#e4ff7a",
      brightBlue: "#8ccfe0",
      brightMagenta: "#dda8cc",
      brightCyan: "#86f0e0",
      brightWhite: "#d6ffe2",
    },
  },
};

/** 픽커의 순환 순서(= 기본 테마가 첫 번째). */
export const THEME_ORDER: readonly ThemeId[] = [
  "daylight",
  "midnight",
  "sakura",
  "pipboy",
];

export const DEFAULT_THEME_ID: ThemeId = "daylight";

export function isThemeId(v: unknown): v is ThemeId {
  return typeof v === "string" && v in THEMES;
}

/**
 * THEME_ORDER 기준 다음 테마. BottomBar가 순환 버튼에서 드롭다운으로 바뀌면서
 * UI 소비처는 없어졌지만, 순환 순서 계약(레지스트리 무결성 테스트)과 향후
 * 단축키/CLI 전환을 위해 남겨 둔다.
 */
export function nextThemeId(id: ThemeId): ThemeId {
  const i = THEME_ORDER.indexOf(id);
  return THEME_ORDER[(i + 1) % THEME_ORDER.length];
}
