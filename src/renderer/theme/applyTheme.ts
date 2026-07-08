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

export const THEME_STORAGE_KEY = "agent-office.theme";

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
  }
  try {
    localStorage.setItem(THEME_STORAGE_KEY, id);
  } catch {
    // 프라이빗 모드/노드 환경 등 저장 불가 — 적용 자체는 유효하므로 무시.
  }
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
