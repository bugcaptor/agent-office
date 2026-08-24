// src/renderer/shared/textKey.ts
//
// 순수 뷰모델이 화면 문구를 만들 때 쓰는 **번역 키 설명자**.
//
// 왜 필요한가: usageView·DailyBarChart·sessionlog/format 같은 순수 모듈이
// 완성된 문장을 돌려주면 (1) 언어를 바꿔도 이미 계산된 문자열은 안 바뀌고
// (2) 단위 테스트가 문구에 묶인다. 그래서 이 모듈들은 `{ key, params }`만
// 돌려주고, 실제 번역은 `t`를 쥔 렌더 컴포넌트가 `renderText`로 한다.
//
// `params` 값에 다시 `TextKey`를 넣을 수 있다 — "마지막 시도 {{ago}}"처럼
// 문장 안에 또 다른 번역 조각이 들어가는 자리를 위해서다(그 조각도 순수
// 함수가 골라야 하므로 문자열로 미리 풀 수 없다).
//
// i18next의 `$t()` 중첩과 역할이 다르다: `$t()`는 카탈로그 값 안에 **고정된**
// 다른 키를 끼워 넣고, 여기 중첩은 **런타임에 고른** 키를 끼워 넣는다.

/** 보간 값. 중첩 설명자는 `renderText`가 재귀로 푼다. */
export type TextParam = string | number | TextKey;

/** 번역 키 + 보간 파라미터. 순수 모듈이 문장 대신 이것을 돌려준다. */
export interface TextKey {
  key: string;
  params?: Record<string, TextParam>;
}

/**
 * `useTranslation(...)`의 `t`(또는 `@renderer/i18n`의 모듈 `t`)를 받는 최소
 * 시그니처. 순수 모듈이 i18next 타입에 의존하지 않게 하려는 것이다.
 */
export type Translate = (key: string, params?: Record<string, string | number>) => string;

/** 설명자를 실제 문구로. 중첩 설명자 파라미터도 함께 푼다. */
export function renderText(text: TextKey, t: Translate): string {
  if (!text.params) return t(text.key);
  const params: Record<string, string | number> = {};
  for (const [name, value] of Object.entries(text.params)) {
    params[name] = typeof value === "object" ? renderText(value, t) : value;
  }
  return t(text.key, params);
}
