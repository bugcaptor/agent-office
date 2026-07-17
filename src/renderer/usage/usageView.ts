// src/renderer/usage/usageView.ts
//
// 구독 사용량(rate limit) 표시용 순수 함수 모음. 백엔드는 정규화된 원시
// 스냅샷만 주고(docs/usage-limits-design.md §3), "가장 절박한 윈도 선택",
// 임계 색상, 카운트다운·신선도 포맷 같은 해석·표시는 여기서 한다. React·스토어
// 의존 없음 — 단위 테스트 대상(설계 §4).

import type { ProviderUsage, UsageSnapshot, UsageWindow } from "@shared/types";

/** 신선도가 이보다 오래되면(ms) stale로 보고 흐리게 표시한다. */
export const STALE_THRESHOLD_MS = 30 * 60 * 1000;

/** 사용률 임계 단계. <70 기본 / ≥70 경고 / ≥90 위험. */
export type UsageLevel = "normal" | "warn" | "danger";

export function usageLevel(usedPercent: number): UsageLevel {
  if (usedPercent >= 90) return "danger";
  if (usedPercent >= 70) return "warn";
  return "normal";
}

/** BottomBar 뱃지 접두. */
export const PROVIDER_SHORT: Record<"claude" | "codex", string> = {
  claude: "CL",
  codex: "CX",
};

/**
 * provider의 가장 절박한 윈도(usedPercent 최대) 하나. 윈도가 없으면 null.
 * 동률이면 먼저 나온 윈도를 유지한다(안정적).
 */
export function mostUrgentWindow(usage: ProviderUsage | null): UsageWindow | null {
  if (!usage || usage.windows.length === 0) return null;
  return usage.windows.reduce((best, w) => (w.usedPercent > best.usedPercent ? w : best));
}

/** 윈도 종류 한국어 라벨. weekly_model은 모델명(label)을 곁들인다. */
export function windowLabel(w: UsageWindow): string {
  switch (w.kind) {
    case "session":
      return "5시간";
    case "weekly":
      return "주간";
    case "weekly_model":
      return w.label ? `주간 · ${w.label}` : "주간 (모델별)";
    case "unknown":
      return w.windowMinutes ? `${w.windowMinutes}분 창` : "기타";
  }
}

/**
 * 리셋까지 남은 시간을 "N시간 N분 후 리셋"으로. 이미 지났으면 "리셋 대기 중",
 * resetsAtMs가 null이면 빈 문자열. 하루 이상은 "N일 N시간 후 리셋".
 */
export function formatCountdown(resetsAtMs: number | null, now: number): string {
  if (resetsAtMs === null) return "";
  const diff = resetsAtMs - now;
  if (diff <= 0) return "리셋 대기 중";
  const totalMin = Math.floor(diff / 60000);
  const days = Math.floor(totalMin / (60 * 24));
  const hours = Math.floor((totalMin % (60 * 24)) / 60);
  const mins = totalMin % 60;
  if (days > 0) return `${days}일 ${hours}시간 후 리셋`;
  if (hours > 0) return `${hours}시간 ${mins}분 후 리셋`;
  return `${mins}분 후 리셋`;
}

/**
 * 신선도를 "N분 전 기준"으로. 1분 미만은 "방금 기준", 1시간 이상은
 * "N시간 N분 전 기준", 하루 이상은 "N일 전 기준".
 */
export function formatFreshness(fetchedAtMs: number, now: number): string {
  const diff = Math.max(0, now - fetchedAtMs);
  const totalMin = Math.floor(diff / 60000);
  if (totalMin < 1) return "방금 기준";
  const days = Math.floor(totalMin / (60 * 24));
  const hours = Math.floor((totalMin % (60 * 24)) / 60);
  const mins = totalMin % 60;
  if (days > 0) return `${days}일 전 기준`;
  if (hours > 0) return `${hours}시간 ${mins}분 전 기준`;
  return `${mins}분 전 기준`;
}

/** 신선도가 STALE_THRESHOLD_MS를 넘었는지. */
export function isStale(fetchedAtMs: number, now: number): boolean {
  return now - fetchedAtMs > STALE_THRESHOLD_MS;
}

/**
 * provider 하나에 대해 이전/새 값 중 신선한 쪽을 고른다. 새 값이 null이면
 * 이전 값 유지, 둘 다 있으면 fetchedAtMs가 큰 쪽(동률은 새 값). 백엔드
 * codex::load는 최신 rollout이 일시적으로 못 읽히면 더 오래된 파일의 유효
 * 스냅샷을 반환할 수 있어(best-available), 단순 교체는 메모리상 더 신선한
 * 값을 옛 값으로 되돌린다 — 그래서 timestamp 비교로만 교체한다. 폴링 응답이
 * 겹쳐 순서가 뒤바뀌어도 같은 비교가 역행을 막는다.
 */
function fresherProvider(
  prev: ProviderUsage | null,
  next: ProviderUsage | null,
): ProviderUsage | null {
  if (!next) return prev;
  if (!prev) return next;
  return next.fetchedAtMs >= prev.fetchedAtMs ? next : prev;
}

/**
 * 새 스냅샷을 provider별로 이전 값과 병합한다. 일시적 파싱 실패(예:
 * `~/.claude.json` rewrite 도중 partial read)로 백엔드가 해당 provider를
 * null로 반환해도 이전 유효 값을 화면에서 지우지 않고, 새 값이 이전 값보다
 * 오래된 스냅샷이면(fetchedAtMs 비교) 이전 값을 유지한다. 어느 쪽이든
 * 신선도 표시가 자연히 오래됨을 알려준다.
 */
export function mergeUsageSnapshot(
  prev: UsageSnapshot | null,
  next: UsageSnapshot,
): UsageSnapshot {
  return {
    claude: fresherProvider(prev?.claude ?? null, next.claude),
    codex: fresherProvider(prev?.codex ?? null, next.codex),
  };
}
