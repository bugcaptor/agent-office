// src/renderer/i18n/index.ts
//
// i18n 런타임(renderer 글루). 카탈로그(@shared/i18n/catalog)를 i18next 인스턴스에
// 물리고, React 안팎에서 같이 쓸 수 있는 `t()`를 내보낸다.
//
// **React 밖에서도 부를 수 있어야 한다**는 게 이 앱의 결정적 제약이다: PixiJS
// 씬, zustand 스토어, workdirStore·speechGenerator 같은 순수 TS 모듈이 전부
// 표시 문자열을 만든다. 그래서 훅(`useTranslation`)은 React 트리에서만 쓰고,
// 그 밖에서는 이 모듈의 `t`/`i18n`을 직접 import한다.
//
// 초기화는 **모듈 로드 시점에 동기로** 끝난다. 리소스가 정적 import(glob eager)라
// 네트워크 백엔드가 없고, 첫 페인트 전에 언어가 정해져 있어야 문구 플래시가
// 없기 때문이다. 실제 설정(AppSettings.language)은 비동기라 한 박자 늦게
// 도착하므로, 그때까지는 localStorage에 캐시해 둔 지난번 해석 결과를 쓴다.
import i18next, { type i18n as I18nInstance } from "i18next";
import { initReactI18next } from "react-i18next";

import {
  FALLBACK_LANGUAGE,
  SOURCE_LANGUAGE,
  SUPPORTED_LANGUAGES,
  languageLabel,
  matchLanguage,
  resources,
} from "@shared/i18n/catalog";

/** `AppSettings.language`가 이 값이면 OS 로케일을 따른다. */
export const LANGUAGE_SYSTEM = "system";

/**
 * 마지막으로 해석된 실제 언어를 담아 두는 localStorage 키. 설정 파일은 비동기라
 * 첫 페인트 때 아직 없다 — 이 캐시가 그 공백을 메워 "한국어로 그렸다가 영어로
 * 바뀌는" 플래시를 없앤다(theme/terminalViewMode와 같은 관례).
 */
export const LANGUAGE_STORAGE_KEY = "agent-office.lang";

function readCachedLanguage(): string | null {
  try {
    const raw = localStorage.getItem(LANGUAGE_STORAGE_KEY);
    return raw && SUPPORTED_LANGUAGES.includes(raw) ? raw : null;
  } catch {
    return null; // localStorage 부재(node 테스트) 포함
  }
}

function writeCachedLanguage(lang: string): void {
  try {
    localStorage.setItem(LANGUAGE_STORAGE_KEY, lang);
  } catch {
    /* 저장 불가 환경에서는 조용히 무시 */
  }
}

/** OS/브라우저 로케일. node 테스트에는 `navigator`가 없다. */
function osLocale(): string | null {
  if (typeof navigator === "undefined") return null;
  return navigator.language ?? null;
}

/**
 * 설정값(`"system"` 또는 언어 코드)을 **실제로 쓸 언어**로 해석한다.
 * - `"system"`(또는 빈 값): OS 로케일을 카탈로그 보유 언어에 매칭, 없으면 en.
 * - 언어 코드: 그대로 쓰되 카탈로그가 없으면 같은 규칙으로 좁힌다.
 */
export function resolveLanguage(setting: string | null | undefined): string {
  if (!setting || setting === LANGUAGE_SYSTEM) return matchLanguage(osLocale());
  return matchLanguage(setting);
}

/** 설정 드롭다운을 채울 항목들 — 카탈로그에 있는 언어가 곧 목록이다. */
export function availableLanguages(): { code: string; label: string }[] {
  return SUPPORTED_LANGUAGES.map((code) => ({ code, label: languageLabel(code) }));
}

/**
 * 개발 중 미번역 키 잡기 — 폴백 언어에도 없어서 키 문자열이 그대로 화면에 샐
 * 상황을 콘솔로 알린다. 프로덕션에서는 조용하다(어차피 ko 폴백이 있어 사용자
 * 눈에는 한국어가 보이고, 언어 간 누락은 catalogParity 테스트가 커밋 전에 잡는다).
 */
function missingKeyHandler(lngs: readonly string[], ns: string, key: string): void {
  if (!import.meta.env?.DEV) return;
  console.error(`i18n: 번역 키 누락 — ${ns}:${key} (${lngs.join(",")})`);
}

/**
 * 이 앱의 i18next 인스턴스. 전역 싱글턴(`i18next` 기본 인스턴스)이 아니라
 * `createInstance()`인 이유: 테스트가 서로의 언어 상태를 밟지 않도록 재초기화
 * 지점을 명시적으로 두기 위해서다(`initI18nForTest` 참고).
 */
export const i18n: I18nInstance = i18next.createInstance();

void i18n.use(initReactI18next).init({
  resources,
  lng: readCachedLanguage() ?? resolveLanguage(LANGUAGE_SYSTEM),
  fallbackLng: SOURCE_LANGUAGE,
  supportedLngs: SUPPORTED_LANGUAGES,
  defaultNS: "common",
  // 리소스가 정적 import라 로딩이 없다 → React Suspense 불필요.
  react: { useSuspense: false },
  // 값 안의 `{{...}}` 보간만 쓰고 HTML은 넣지 않는다. React가 이미 이스케이프하므로
  // i18next의 이스케이프는 끈다(안 끄면 따옴표·꺾쇠가 `&#39;` 따위로 이중 변환된다).
  interpolation: { escapeValue: false },
  saveMissing: true,
  missingKeyHandler,
});

/**
 * React 밖(PixiJS·zustand·순수 모듈)에서 쓰는 번역 함수. React 안에서는
 * `useTranslation()`을 써야 언어 변경 시 리렌더가 걸린다.
 */
export const t = i18n.t.bind(i18n);

/** `Intl.*`(날짜·상대시간·숫자)에 넘길 로케일. `navigator.language` 직접 참조 금지 —
 * 사용자가 설정에서 오버라이드한 언어를 무시하게 된다. */
export function currentLocale(): string {
  return i18n.language || SOURCE_LANGUAGE;
}

/**
 * 설정값을 적용한다(해석 → 언어 전환 → 캐시 갱신). 이미 그 언어면 아무것도
 * 하지 않는다 — 부팅 때 캐시와 설정이 일치하는 흔한 경우에 불필요한
 * `languageChanged` 방출을 막는다.
 */
export function applyLanguageSetting(setting: string | null | undefined): void {
  const lang = resolveLanguage(setting);
  writeCachedLanguage(lang);
  if (i18n.language === lang) return;
  void i18n.changeLanguage(lang);
}

/** 테스트용 — 언어를 동기로 못박는다. 파일 간 언어 상태 누수를 막으려면
 * `afterEach`에서 정본 언어로 되돌린다. */
export async function initI18nForTest(lang: string = SOURCE_LANGUAGE): Promise<void> {
  await i18n.changeLanguage(matchLanguage(lang));
}

export { SOURCE_LANGUAGE, FALLBACK_LANGUAGE, SUPPORTED_LANGUAGES };
