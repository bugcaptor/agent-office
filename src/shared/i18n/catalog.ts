// src/shared/i18n/catalog.ts
//
// 번역 카탈로그 로더. `locales/<언어>/<네임스페이스>.json`을 전부 eager glob으로
// 끌어와 i18next가 그대로 먹을 수 있는 `resources` 모양으로 만든다.
//
// **언어를 추가하는 방법 = `locales/`에 폴더 하나를 더 만드는 것뿐이다.**
// 이 파일도, 설정 드롭다운도, 타입도 고칠 필요가 없다 — 지원 언어 목록과
// 드롭다운 항목이 전부 이 glob에서 도출된다.
//
// renderer가 아니라 shared에 두는 이유: 웹 원격(src/web)을 나중에 다국어화할 때
// 자기 글루만 만들면 같은 JSON을 그대로 재사용할 수 있게 하려는 것이다(이번
// 범위는 데스크톱 renderer뿐).

/** 네임스페이스 하나의 내용(중첩 객체 — i18next의 키 `ns:a.b.c`). */
export type CatalogNamespace = Record<string, unknown>;

/** 한 언어의 전체 카탈로그: 네임스페이스 이름 → 내용. */
export type CatalogLanguage = Record<string, CatalogNamespace>;

/** i18next `resources` 모양: 언어 → 네임스페이스 → 내용. */
export type CatalogResources = Record<string, CatalogLanguage>;

/**
 * 정본 언어. 이 앱은 한국어로 먼저 쓰이므로 ko가 소스이자 폴백이다 —
 * 다른 언어에 키가 빠져도 사용자에겐 한국어가 보이지, 키 문자열이 새지 않는다.
 * (누락 자체는 catalogParity 테스트가 커밋 전에 잡는다.)
 */
export const SOURCE_LANGUAGE = "ko";

/** 카탈로그가 없는 언어로 떨어졌을 때 마지막으로 시도할 언어. */
export const FALLBACK_LANGUAGE = "en";

/**
 * 각 언어 `common.json`의 `_meta` 블록. 번역 문자열이 아니라 카탈로그 자체의
 * 메타데이터라 `_` 접두사로 구분한다.
 */
interface CatalogMeta {
  /** 설정 드롭다운에 쓸 **그 언어로 표기한** 언어 이름("한국어", "English"). */
  label: string;
}

const modules = import.meta.glob("./locales/*/*.json", { eager: true }) as Record<
  string,
  { default: CatalogNamespace }
>;

function buildResources(): CatalogResources {
  const out: CatalogResources = {};
  for (const [path, mod] of Object.entries(modules)) {
    // "./locales/ko/common.json" → ["ko", "common"]
    const m = /\/locales\/([^/]+)\/([^/]+)\.json$/.exec(path);
    if (!m) continue;
    const [, lang, ns] = m;
    (out[lang] ??= {})[ns] = mod.default;
  }
  return out;
}

/** i18next에 그대로 넘기는 리소스 트리. */
export const resources: CatalogResources = buildResources();

/**
 * 카탈로그가 실제로 존재하는 언어 코드들(정본 언어가 항상 맨 앞).
 * 설정 드롭다운·폴백 해석·파리티 테스트가 전부 이 목록을 본다.
 */
export const SUPPORTED_LANGUAGES: string[] = Object.keys(resources).sort((a, b) => {
  if (a === SOURCE_LANGUAGE) return -1;
  if (b === SOURCE_LANGUAGE) return 1;
  return a.localeCompare(b);
});

/**
 * 언어의 자기 이름("한국어", "English"). `_meta.label`이 없으면 코드 자체를
 * 돌려준다 — 새 언어를 넣다가 `_meta`를 빠뜨려도 드롭다운이 비지는 않는다.
 */
export function languageLabel(lang: string): string {
  const meta = resources[lang]?.common?._meta as CatalogMeta | undefined;
  return meta?.label ?? lang;
}

/**
 * 지역 코드를 문자(script)까지 넓힌 코드로 바꾼다 — `zh-TW` → `zh-Hant`,
 * `zh-CN` → `zh-Hans`. 문자가 없으면 null.
 *
 * 이게 없으면 `zh-*`가 전부 프리픽스 일치로 잡혀 정렬상 앞에 오는 `zh-Hans`에
 * 붙는다. 대만·홍콩 사용자에게 간체가 나가는 건 오답이다. 지역→문자 표는
 * `Intl.Locale.maximize()`(CLDR)가 들고 있으니 우리가 관리하지 않는다.
 */
function scriptCode(locale: string): string | null {
  try {
    const l = new Intl.Locale(locale).maximize();
    return l.script ? `${l.language}-${l.script}` : null;
  } catch {
    return null; // 파싱할 수 없는 로케일 문자열
  }
}

/**
 * 임의의 로케일 문자열(`ko-KR`, `en-US`, `system` 아님)을 보유 카탈로그 중
 * 하나로 좁힌다. 정확히 일치 → 문자 일치(`zh-TW` → `zh-Hant`) → 프리픽스
 * 일치(`en-GB` → `en`) → 폴백 순.
 */
export function matchLanguage(locale: string | null | undefined): string {
  if (!locale) return FALLBACK_LANGUAGE;
  const lower = locale.toLowerCase();
  const exact = SUPPORTED_LANGUAGES.find((l) => l.toLowerCase() === lower);
  if (exact) return exact;
  const script = scriptCode(locale)?.toLowerCase();
  if (script) {
    const byScript = SUPPORTED_LANGUAGES.find((l) => l.toLowerCase() === script);
    if (byScript) return byScript;
  }
  const prefix = lower.split("-")[0];
  const byPrefix = SUPPORTED_LANGUAGES.find((l) => l.toLowerCase().split("-")[0] === prefix);
  if (byPrefix) return byPrefix;
  return SUPPORTED_LANGUAGES.includes(FALLBACK_LANGUAGE) ? FALLBACK_LANGUAGE : SOURCE_LANGUAGE;
}
