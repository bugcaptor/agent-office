// src/renderer/sessionlog/format.ts
//
// 세션 로그 목록의 표시 포맷 순수 함수들(테스트 대상). 컴포넌트는 이걸 쓰기만 한다.

/** `2026-07-25 14:03` — 목록의 시작 시각. */
export function formatWhen(at: number): string {
  if (!Number.isFinite(at) || at <= 0) return "시각 미상";
  const d = new Date(at);
  const p = (n: number) => String(n).padStart(2, "0");
  return `${d.getFullYear()}-${p(d.getMonth() + 1)}-${p(d.getDate())} ${p(d.getHours())}:${p(d.getMinutes())}`;
}

/** 시작~마지막 기록 사이의 길이. 1분 미만은 "1분 미만". */
export function formatDuration(startedAt: number, modifiedAt: number): string {
  const ms = modifiedAt - startedAt;
  if (!Number.isFinite(ms) || ms < 0) return "";
  const minutes = Math.floor(ms / 60_000);
  if (minutes < 1) return "1분 미만";
  if (minutes < 60) return `${minutes}분`;
  const hours = Math.floor(minutes / 60);
  const rest = minutes % 60;
  return rest === 0 ? `${hours}시간` : `${hours}시간 ${rest}분`;
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
