// src/renderer/shared/relativeTime.ts
//
// 이슈 #67: 팔레트 헤더의 "N분 전 기준" 표시용 순수 포맷 함수. fetchedAt(과거
// 시각)과 now(기본 Date.now())의 차이를 사람이 읽는 문구로 바꾼다.
// markdownStore/workdirStore 캐시 엔트리의 fetchedAt과 함께 쓰인다.
//
// 문구는 손으로 조립하지 않고 `Intl.RelativeTimeFormat`에 맡긴다 — 언어를
// 하나 추가할 때 상대시간 표기가 카탈로그 한 줄 없이 따라오게 하려는 것이다
// (ko "3분 전" / en "3 minutes ago"). 카탈로그에 남는 건 Intl이 만들지 못하는
// 임계 구간 문구(`time.justNow`)뿐이다.
//
// 임계값(5초 / 60초 / 60분 / 24시간)은 예전 손조립 버전 그대로다 — 표기가
// 바뀌는 지점을 건드리면 이 함수를 쓰는 팔레트 헤더의 체감이 달라진다.
import { currentLocale, t } from "@renderer/i18n";

/**
 * 언어별 `Intl.RelativeTimeFormat` 캐시. 모듈 최상위에서 하나만 만들면 설정에서
 * 언어를 바꿔도 낡은 로케일에 고정되므로, **호출 시점의 `currentLocale()`**을
 * 키로 삼아 언어마다 따로 만들어 둔다(생성 비용만 아끼고 언어는 따라간다).
 */
const formatters = new Map<string, Intl.RelativeTimeFormat>();

/**
 * 지금 언어의 상대시간 포매터.
 *
 * `numeric: "always"`인 게 의도다. `"auto"`로 두면 ko가 하루/이틀 전을
 * "어제"/"그저께"로 바꿔 버리는데, 이 함수의 쓰임은 캐시 신선도 표시
 * ("N일 전 기준")라 관용 표현보다 **경과 시간을 숫자로 읽히는 쪽**이 맞다
 * (en도 마찬가지로 "yesterday"보다 "1 day ago"가 낫다). 분·시 단위는 두 모드가
 * 같은 결과를 내므로, 이 선택으로 손조립 시절의 ko 출력이 그대로 보존된다.
 */
export function relativeTimeFormatter(): Intl.RelativeTimeFormat {
  const locale = currentLocale();
  let f = formatters.get(locale);
  if (!f) {
    f = new Intl.RelativeTimeFormat(locale, { numeric: "always" });
    formatters.set(locale, f);
  }
  return f;
}

export function formatRelativeTime(fetchedAt: number, now: number = Date.now()): string {
  const diffMs = Math.max(0, now - fetchedAt);
  const diffSec = Math.floor(diffMs / 1000);
  if (diffSec < 5) return t("time.justNow");
  const rtf = relativeTimeFormatter();
  if (diffSec < 60) return rtf.format(-diffSec, "second");
  const diffMin = Math.floor(diffSec / 60);
  if (diffMin < 60) return rtf.format(-diffMin, "minute");
  const diffHour = Math.floor(diffMin / 60);
  if (diffHour < 24) return rtf.format(-diffHour, "hour");
  const diffDay = Math.floor(diffHour / 24);
  return rtf.format(-diffDay, "day");
}
