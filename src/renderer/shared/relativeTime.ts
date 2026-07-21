// src/renderer/shared/relativeTime.ts
//
// 이슈 #67: 팔레트 헤더의 "N분 전 기준" 표시용 순수 포맷 함수. fetchedAt(과거
// 시각)과 now(기본 Date.now())의 차이를 사람이 읽는 한국어 문구로 바꾼다.
// markdownStore/workdirStore 캐시 엔트리의 fetchedAt과 함께 쓰인다.
export function formatRelativeTime(fetchedAt: number, now: number = Date.now()): string {
  const diffMs = Math.max(0, now - fetchedAt);
  const diffSec = Math.floor(diffMs / 1000);
  if (diffSec < 5) return "방금";
  if (diffSec < 60) return `${diffSec}초 전`;
  const diffMin = Math.floor(diffSec / 60);
  if (diffMin < 60) return `${diffMin}분 전`;
  const diffHour = Math.floor(diffMin / 60);
  if (diffHour < 24) return `${diffHour}시간 전`;
  const diffDay = Math.floor(diffHour / 24);
  return `${diffDay}일 전`;
}
