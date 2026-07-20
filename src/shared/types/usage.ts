// src/shared/types/usage.ts
//
// Domain slice: subscription usage / rate-limit windows.
// See src/shared/types.ts for the frozen-contract overview.

/**
 * 구독 사용량(rate limit) 한도 윈도 종류. Rust `UsageWindowKind`
 * (serde snake_case) 미러. 설계: docs/usage-limits-design.md §3.
 * `unknown`은 미래 확장 대비 폴백(예: 매핑 안 된 codex window_minutes).
 */
export type UsageWindowKind = "session" | "weekly" | "weekly_model" | "unknown";

/**
 * 한도 윈도 1개. Rust `UsageWindow`(camelCase) 미러. 단위는 전부 백엔드에서
 * 정규화됨: `resetsAtMs`는 epoch ms(Claude ISO·Codex 초 모두 변환), 백분율은
 * `usedPercent`. nullable 필드는 `T | null`(optional 아님).
 */
export interface UsageWindow {
  kind: UsageWindowKind;
  /** weekly_model일 때 모델 표시명 등. 없으면 null. */
  label: string | null;
  usedPercent: number;
  /** epoch ms로 정규화. 파싱 불가/부재 시 null. */
  resetsAtMs: number | null;
  windowMinutes: number | null;
  /**
   * "지금 구속 중인 윈도"인지(Claude `limits[]`에만 있음). **유효성이
   * 아니다** — 실측(`~/.claude.json`)상 weekly_all/weekly_scoped도 살아
   * 있는 한도인데 false로 온다. 걸러내는 용도로 쓰지 말 것, 표시용 보조
   * 정보로만 쓴다. Codex와 Claude five_hour/seven_day 폴백 경로는 항상 null.
   */
  isActive: boolean | null;
}

/**
 * provider별 사용량. Rust `ProviderUsage`(camelCase) 미러.
 * `windows`는 가변 배열 — UI가 "5시간+주간 둘 다 있음"을 하드코딩하지 않는다.
 */
export interface ProviderUsage {
  provider: "claude" | "codex";
  /** 신선도 기준 시각(epoch ms). 로컬 CLI가 실제로 돌 때만 갱신되는 캐시. */
  fetchedAtMs: number;
  /** codex plan_type, claude organizationRateLimitTier 등. 없으면 null. */
  planLabel: string | null;
  windows: UsageWindow[];
}

/**
 * `load_usage_snapshot` 응답. Rust `UsageSnapshot` 미러. 파싱에 실패한 소스는
 * 해당 provider가 null이며, 커맨드 자체는 항상 성공한다.
 */
export interface UsageSnapshot {
  claude: ProviderUsage | null;
  codex: ProviderUsage | null;
}
