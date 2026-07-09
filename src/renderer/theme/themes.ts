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
// - 캐릭터 스프라이트 팔레트(office/gen/palette.ts)는 에이전트별 절차
//   생성이므로 테마 대상이 아니다.

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
export type CssTokenKey = (typeof CSS_TOKEN_KEYS)[number];

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
export type TilePaletteKey = (typeof TILE_PALETTE_KEYS)[number];

/** 타일 색만(0xRRGGBB). TileRenderer의 생성자 인자 타입. */
export type OfficeTilePalette = Record<TilePaletteKey, number>;

/** 타일 색 + 씬 배경색. OfficeScene이 소비. */
export interface PixiThemePalette extends OfficeTilePalette {
  background: number;
}

export type ThemeId = "daylight" | "midnight" | "sakura";

export interface ThemeDef {
  id: ThemeId;
  /** 픽커 버튼에 그대로 노출되는 한국어 라벨. */
  label: string;
  css: Record<CssTokenKey, string>;
  pixi: PixiThemePalette;
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
  },
};

/** 픽커의 순환 순서(= 기본 테마가 첫 번째). */
export const THEME_ORDER: readonly ThemeId[] = ["daylight", "midnight", "sakura"];

export const DEFAULT_THEME_ID: ThemeId = "daylight";

export function isThemeId(v: unknown): v is ThemeId {
  return typeof v === "string" && v in THEMES;
}

/** 픽커 버튼용: THEME_ORDER 기준 다음 테마. */
export function nextThemeId(id: ThemeId): ThemeId {
  const i = THEME_ORDER.indexOf(id);
  return THEME_ORDER[(i + 1) % THEME_ORDER.length];
}
