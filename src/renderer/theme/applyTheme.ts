// src/renderer/theme/applyTheme.ts
//
// 테마 적용/영속 — DOM 부수효과 계층. 레지스트리(themes.ts)는 순수 데이터로
// 남겨 두고, documentElement/localStorage를 만지는 코드는 전부 여기로 모은다.
//
// 플래시 방지: main.tsx가 첫 render() 전에 동기적으로
// `applyTheme(useAppStore.getState().theme)`을 호출한다(스토어 초기값이
// `loadStoredThemeId()`라 저장된 테마가 곧바로 적용된다).
import { CSS_TOKEN_KEYS, DEFAULT_THEME_ID, THEMES, isThemeId } from "./themes";
import type { ThemeId } from "./themes";
import { storedXtermBackground, xtermBackground } from "../terminal/theme";
import type { XtermThemeOverride } from "../terminal/theme";

export const THEME_STORAGE_KEY = "agent-office.theme";

/**
 * 터미널 호스트 배경(terminal.css `.terminal-host`)이 참조하는 커스텀
 * 프로퍼티. xterm 캔버스가 그려지기 전(그리고 패딩 영역)에도 터미널 색과
 * 같은 바탕이 깔리도록 유효 xterm 팔레트의 background를 흘려보낸다.
 */
const TERM_BG_VAR = "--term-bg";

/**
 * 테마를 DOM에 적용하고 localStorage에 영속한다.
 * - documentElement에 `data-theme="<id>"` 속성을 세팅하고,
 * - 테마의 CSS 토큰 맵을 인라인 커스텀 프로퍼티로 주입한다(:root 폴백보다
 *   우선하므로 tokens.css 수정 없이 전 토큰이 즉시 전환된다).
 * document/localStorage가 없는 환경(node 단위테스트)에서는 해당 단계만
 * 조용히 건너뛴다 — 스토어 액션이 어디서 불려도 안전해야 하므로.
 */
export function applyTheme(id: ThemeId): void {
  const theme = THEMES[id];
  if (typeof document !== "undefined") {
    const root = document.documentElement;
    root.dataset.theme = id;
    for (const key of CSS_TOKEN_KEYS) root.style.setProperty(key, theme.css[key]);
    // 터미널 색상 선택("auto"면 이 테마, 아니면 고정 테마)까지 여기서 반영 —
    // main.tsx의 부트 경로가 applyTheme 하나만 부르므로 첫 페인트 전에
    // --term-bg가 세팅된다. 이후 유저가 선택을 바꾸면 App의 효과가 갱신한다.
    root.style.setProperty(TERM_BG_VAR, storedXtermBackground(id));
  }
  try {
    localStorage.setItem(THEME_STORAGE_KEY, id);
  } catch {
    // 프라이빗 모드/노드 환경 등 저장 불가 — 적용 자체는 유효하므로 무시.
  }
}

/**
 * 터미널 배경 커스텀 프로퍼티만 갱신한다 — 앱 테마는 그대로 두고 "터미널
 * 색상"만 바꿨을 때(설정 다이얼로그) 쓰는 경로. 영속은 스토어 액션의 몫.
 */
export function applyTerminalBg(id: ThemeId, override: XtermThemeOverride): void {
  if (typeof document === "undefined") return;
  document.documentElement.style.setProperty(TERM_BG_VAR, xtermBackground(id, override));
}

/** 저장된 테마 id를 읽는다. 없거나 알 수 없는 값이면 기본(daylight). */
export function loadStoredThemeId(): ThemeId {
  try {
    const raw = localStorage.getItem(THEME_STORAGE_KEY);
    return isThemeId(raw) ? raw : DEFAULT_THEME_ID;
  } catch {
    return DEFAULT_THEME_ID; // localStorage 부재(node) 포함
  }
}
