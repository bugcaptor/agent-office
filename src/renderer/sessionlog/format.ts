// src/renderer/sessionlog/format.ts
//
// 세션 로그 목록의 표시 포맷 순수 함수들(테스트 대상). 컴포넌트는 이걸 쓰기만 한다.
//
// i18n: 문구가 들어가는 함수는 완성된 문장이 아니라 **번역 키 설명자**를
// 돌려준다(표시 없음은 null). 그래야 언어를 바꿔도 목록이 따라 바뀌고, 이
// 파일의 단위 테스트가 문구가 아니라 규칙(임계값·자릿수)을 검증하게 된다.
import type { TextKey } from "../shared/textKey";

/**
 * `2026-07-25 14:03` — 목록의 시작 시각. 시각을 모르면 null(호출자가
 * `sessionLog.whenUnknown`을 그린다).
 *
 * 로케일 포맷(`Intl.DateTimeFormat`)을 쓰지 않는 건 의도다 — 이 열은 목록을
 * 시간순으로 훑는 자리라 자릿수가 고정된 ISO 유사 표기가 폭·정렬 모두에
 * 유리하고, 언어를 바꿔도 열 너비가 흔들리지 않는다.
 */
export function formatWhen(at: number): string | null {
  if (!Number.isFinite(at) || at <= 0) return null;
  const d = new Date(at);
  const p = (n: number) => String(n).padStart(2, "0");
  return `${d.getFullYear()}-${p(d.getMonth() + 1)}-${p(d.getDate())} ${p(d.getHours())}:${p(d.getMinutes())}`;
}

/** 시작~마지막 기록 사이의 길이 키. 1분 미만은 "1분 미만", 값이 이상하면 null. */
export function formatDuration(startedAt: number, modifiedAt: number): TextKey | null {
  const ms = modifiedAt - startedAt;
  if (!Number.isFinite(ms) || ms < 0) return null;
  const minutes = Math.floor(ms / 60_000);
  if (minutes < 1) return { key: "sessionLog.durUnderMinute" };
  if (minutes < 60) return { key: "sessionLog.durMinutes", params: { minutes } };
  const hours = Math.floor(minutes / 60);
  const rest = minutes % 60;
  return rest === 0
    ? { key: "sessionLog.durHours", params: { hours } }
    : { key: "sessionLog.durHoursMinutes", params: { hours, rest } };
}

/** 사람이 읽는 파일 크기. */
export function formatBytes(bytes: number): string {
  if (!Number.isFinite(bytes) || bytes < 0) return "";
  if (bytes < 1024) return `${bytes}B`;
  if (bytes < 1024 * 1024) return `${Math.round(bytes / 1024)}KB`;
  return `${(bytes / (1024 * 1024)).toFixed(1)}MB`;
}

/** 작업 폴더는 뒤쪽(실제 프로젝트 이름)이 중요하므로 앞을 줄인다. */
export function shortenPath(path: string, max = 36): string {
  const home = "/Users/";
  let s = path;
  // `/Users/<me>/dev/foo` -> `~/dev/foo`
  if (s.startsWith(home)) {
    const rest = s.slice(home.length);
    const slash = rest.indexOf("/");
    if (slash >= 0) s = `~${rest.slice(slash)}`;
  }
  if (s.length <= max) return s;
  return `…${s.slice(s.length - (max - 1))}`;
}
