// src/renderer/terminal/theme.ts
//
// 터미널(xterm) 팔레트의 해석/영속 계층 — 색 값 자체는 여기에 없다.
// 실값은 테마 레지스트리(theme/themes.ts)의 `THEMES[id].xterm`이 단일 원천이고,
// 이 모듈은 "어느 테마의 xterm 팔레트를 쓸 것인가"만 결정한다.
//
// - "auto"(기본): 앱 테마를 그대로 따라간다.
// - 특정 ThemeId: 앱 테마와 무관하게 터미널 색만 그 테마로 고정한다
//   (예: 앱은 daylight, 터미널만 midnight).
//
// 순수 로직(스토어/xterm 런타임 의존 없음)이라 appStore와 applyTheme이 모두
// 안전하게 import할 수 있다 — `ITheme`은 type-only import.
import type { ITheme } from "@xterm/xterm";
import { THEMES, isThemeId } from "../theme/themes";
import type { ThemeId } from "../theme/themes";

/** 터미널 색상 선택: "auto"면 앱 테마를 따르고, 그 외엔 해당 테마로 고정. */
export type XtermThemeOverride = ThemeId | "auto";

export const XTERM_THEME_STORAGE_KEY = "agent-office.xterm-theme";

export const DEFAULT_XTERM_THEME_OVERRIDE: XtermThemeOverride = "auto";

export function isXtermThemeOverride(v: unknown): v is XtermThemeOverride {
  return v === "auto" || isThemeId(v);
}

/** 앱 테마 + 오버라이드 → 실제로 xterm에 먹일 팔레트가 속한 테마 id. */
export function effectiveXtermThemeId(
  themeId: ThemeId,
  override: XtermThemeOverride
): ThemeId {
  return override === "auto" ? themeId : override;
}

/** 앱 테마 + 오버라이드 → xterm `ITheme`. */
export function resolveXtermTheme(
  themeId: ThemeId,
  override: XtermThemeOverride
): ITheme {
  return THEMES[effectiveXtermThemeId(themeId, override)].xterm;
}

/** 저장된 선택을 읽는다. 없거나 알 수 없으면 "auto". localStorage 부재(node)도 안전. */
export function loadStoredXtermThemeOverride(): XtermThemeOverride {
  try {
    const raw = localStorage.getItem(XTERM_THEME_STORAGE_KEY);
    return isXtermThemeOverride(raw) ? raw : DEFAULT_XTERM_THEME_OVERRIDE;
  } catch {
    return DEFAULT_XTERM_THEME_OVERRIDE;
  }
}

/** 선택을 localStorage에 영속한다. 저장 불가 환경에서는 조용히 무시. */
export function persistXtermThemeOverride(override: XtermThemeOverride): void {
  try {
    localStorage.setItem(XTERM_THEME_STORAGE_KEY, override);
  } catch {
    // 프라이빗 모드/노드 환경 등 저장 불가 — 적용 자체는 유효하므로 무시.
  }
}

/**
 * 저장된 선택 기준의 터미널 배경색. `applyTheme`이 첫 페인트 전에
 * `--term-bg`로 주입해, 터미널 호스트(터미널.css)가 xterm 캔버스가 그려지기
 * 전에도 같은 색으로 깔리게 한다.
 */
export function storedXtermBackground(themeId: ThemeId): string {
  return xtermBackground(themeId, loadStoredXtermThemeOverride());
}

/** 앱 테마 + 오버라이드 → 터미널 배경색(ThemeDef가 필수로 보장). */
export function xtermBackground(
  themeId: ThemeId,
  override: XtermThemeOverride
): string {
  return THEMES[effectiveXtermThemeId(themeId, override)].xterm.background;
}
