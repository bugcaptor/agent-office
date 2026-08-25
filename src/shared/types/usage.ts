// src/shared/types/usage.ts
//
// Domain slice: subscription usage / rate-limit windows.
// See src/shared/types.ts for the frozen-contract overview.

/**
 * 구독 사용량(rate limit) 한도 윈도 종류. Rust `UsageWindowKind`
 * (serde snake_case) 미러. 설계: docs/usage-limits-design.md §3.
 * `unknown`은 미래 확장 대비 폴백(예: 매핑 안 된 codex window_minutes).
 */
export type UsageWindowKind =
  | "session"
  | "weekly"
  /**
   * 모델별 5시간 창(Codex `rateLimitsByLimitId`의 이름 붙은 버킷). `session`과
   * 구분해 두어야 뱃지의 "5시간" 자리를 모델별 창이 가로채지 않는다 — 모델
   * 표시명은 `label`에 온다.
   */
  | "session_model"
  | "weekly_model"
  | "unknown";

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
  provider: "claude" | "codex" | "antigravity";
  /**
   * 신선도 기준 시각(epoch ms). 출처마다 갱신 조건이 다르다: 앱의 실시간
   * 조회가 성공하면 그 조회 시각이고, 실패하면 로컬 캐시(Claude는
   * `~/.claude.json`, Codex는 rollout jsonl)의 시각이다. 캐시 쪽은 CLI가
   * 실제로 돌아야만 갱신되므로 며칠씩 멈춰 있을 수 있다 — 그 이유는
   * `claudeLive`/`codexLive`가 설명한다.
   */
  fetchedAtMs: number;
  /** codex plan_type, claude organizationRateLimitTier 등. 없으면 null. */
  planLabel: string | null;
  windows: UsageWindow[];
}

/**
 * Claude 실시간 조회(`GET /api/oauth/usage`)의 마지막 시도 결과. Rust
 * `LiveFetchOutcome`(serde snake_case) 미러. 설계: docs/usage-design.md §7.
 *
 * 이 값이 `ok`가 아니면 화면의 Claude 숫자는 `~/.claude.json`의 로컬 캐시
 * 미러이고, 그 캐시는 **Claude Code에서 `/usage`를 열 때만** 갱신된다
 * (일반 대화로는 갱신되지 않는다) — 표시값이 며칠씩 멈춰 보이는 주된 원인.
 */
export type LiveFetchOutcome =
  | "never_attempted"
  | "ok"
  | "no_credentials"
  | "unauthorized"
  | "http_error"
  | "network_error"
  | "unexpected_response";

/**
 * 토큰을 읽어낸 출처. Rust `TokenSource` 미러. `file` + `unauthorized`는
 * "Keychain 접근이 막혀 `.credentials.json`으로 폴백했는데 그 토큰이 낡음"이라는
 * 흔한 실패 조합을 가리킨다.
 */
export type TokenSource = "keychain_scoped" | "keychain_legacy" | "file";

/**
 * 사용량 값을 실제로 얻어낸 전송 수단. Rust `FetchTransport` 미러.
 *
 * `direct`가 아니라는 것은 앱이 직접 거는 HTTPS가 이 환경에서 막혀 있고
 * (사내 MITM 프록시·self-signed 루트 등) 외부 프로세스로 우회 중이라는
 * 뜻이다 — 값 자체는 정상이지만 환경 진단으로서 표시할 가치가 있다.
 *
 * `claude_cli`는 `claude -p /usage` 경유다. 현재 CLI는 `-p` 모드에서 사용량을
 * 돌려주지 않으므로 사실상 나오지 않는 값이며, CLI가 나중에 지원하면 그때
 * 살아난다.
 */
export type FetchTransport = "direct" | "curl" | "claude_cli";

/**
 * 실시간 조회 진단. Rust `ClaudeLiveStatus` 미러(camelCase). 스냅샷마다 항상
 * 존재한다 — "아직 모름"은 null이 아니라 `never_attempted`다.
 */
export interface ClaudeLiveStatus {
  outcome: LiveFetchOutcome;
  /** 토큰을 못 읽었으면 null. 토큰 값 자체는 절대 오지 않는다. */
  tokenSource: TokenSource | null;
  /** 진단 보조(예: "HTTP 401", "시간 초과"). 없으면 null. */
  detail: string | null;
  /** 마지막 시도 시각(epoch ms). 스로틀에 막혀 건너뛴 폴링은 시도가 아니다. */
  lastAttemptMs: number | null;
  /** 마지막 성공 시각(epoch ms). 한 번도 성공한 적 없으면 null. */
  lastSuccessMs: number | null;
  /**
   * 마지막으로 값을 얻어낸 전송 수단. 한 번도 성공한 적 없으면 null.
   * 실패는 이 값을 지우지 않는다 — "지금은 실패 중이지만 아까 그 값은 curl
   * 우회로 받아온 것"이라는 설명이 성립해야 하기 때문이다.
   */
  via: FetchTransport | null;
}

/**
 * Codex 실시간 조회(`codex app-server`의 `account/rateLimits/read` RPC)의
 * 마지막 시도 결과. Rust `CodexLiveOutcome`(serde snake_case) 미러.
 *
 * Claude의 `LiveFetchOutcome`과 **일부러 분리된 어휘**다 — 이쪽은 우리가
 * 토큰을 만지지 않고 codex CLI에 물어보는 경로라, 실패가 HTTP 상태코드가
 * 아니라 "CLI가 없다/죽었다/모르는 응답을 줬다"로 나타난다.
 *
 * 이 값이 `ok`가 아니면 화면의 Codex 숫자는 rollout jsonl(`~/.codex/sessions`)
 * 에 남은 스냅샷이고, 그 스냅샷은 **Codex CLI가 실제로 돌 때만** 갱신된다.
 */
export type CodexLiveOutcome =
  | "never_attempted"
  | "ok"
  /** `codex` 실행 파일을 찾지 못함(미설치·PATH 밖). */
  | "cli_missing"
  /** 프로세스는 떴는데 응답 없이 죽었거나 파이프가 끊김. */
  | "cli_failed"
  | "timeout"
  /** 서버가 JSON-RPC error를 돌려줌(미로그인·계정 문제 등). */
  | "rpc_error"
  | "unexpected_response";

/**
 * Codex 실시간 조회 진단. Rust `CodexLiveStatus` 미러(camelCase). 스냅샷마다
 * 항상 존재한다 — "아직 모름"은 null이 아니라 `never_attempted`다.
 * 자격증명을 앱이 만지지 않으므로 Claude와 달리 `tokenSource`/`via`가 없다.
 */
export interface CodexLiveStatus {
  outcome: CodexLiveOutcome;
  /** 진단 보조(예: "not logged in", "시간 초과"). 없으면 null. */
  detail: string | null;
  /** 마지막 시도 시각(epoch ms). 스로틀에 막혀 건너뛴 폴링은 시도가 아니다. */
  lastAttemptMs: number | null;
  /** 마지막 성공 시각(epoch ms). 한 번도 성공한 적 없으면 null. */
  lastSuccessMs: number | null;
}

/**
 * Antigravity 실시간 조회(`agy -p /usage --output-format json`)의 마지막 시도
 * 결과. Rust `AntigravityLiveOutcome`(serde snake_case) 미러.
 *
 * Codex와 어휘가 겹치지만 **분리돼 있다** — 이쪽은 JSON-RPC가 아니라 print
 * 모드 1회 실행이라 "RPC 오류"가 없고, 대신 CLI가 붙여 주는 `status` 필드가
 * 실패 갈래를 가른다.
 *
 * 이 값이 `ok`가 아니면 화면의 Antigravity 숫자는 **직전 성공 값이거나 아예
 * 없다** — Claude·Codex와 달리 강등할 로컬 파일 캐시가 없기 때문이다.
 */
export type AntigravityLiveOutcome =
  | "never_attempted"
  | "ok"
  /** `agy` 실행 파일을 찾지 못함(미설치·PATH 밖). */
  | "cli_missing"
  /** 프로세스는 떴는데 실패로 끝났거나 출력이 없었다. */
  | "cli_failed"
  | "timeout"
  /** CLI가 실패 status를 돌려줌(미로그인·계정 문제 등). */
  | "command_failed"
  | "unexpected_response";

/**
 * Antigravity 실시간 조회 진단. Rust `AntigravityLiveStatus` 미러(camelCase).
 * 스냅샷마다 항상 존재한다 — "아직 모름"은 null이 아니라 `never_attempted`.
 * Codex와 마찬가지로 자격증명을 앱이 만지지 않아 `tokenSource`/`via`가 없다.
 */
export interface AntigravityLiveStatus {
  outcome: AntigravityLiveOutcome;
  /** 진단 보조(예: "not logged in", "시간 초과"). 없으면 null. */
  detail: string | null;
  /** 마지막 시도 시각(epoch ms). 스로틀에 막혀 건너뛴 폴링은 시도가 아니다. */
  lastAttemptMs: number | null;
  /** 마지막 성공 시각(epoch ms). 한 번도 성공한 적 없으면 null. */
  lastSuccessMs: number | null;
}

/**
 * `load_usage_snapshot` 응답. Rust `UsageSnapshot` 미러. 파싱에 실패한 소스는
 * 해당 provider가 null이며, 커맨드 자체는 항상 성공한다.
 */
export interface UsageSnapshot {
  claude: ProviderUsage | null;
  codex: ProviderUsage | null;
  /**
   * Antigravity 사용량. **로컬 파일 캐시가 없어** 실시간 조회가 한 번도
   * 성공하지 않았으면 항상 null이다(다른 두 provider와 다른 점).
   */
  antigravity: ProviderUsage | null;
  claudeLive: ClaudeLiveStatus;
  codexLive: CodexLiveStatus;
  antigravityLive: AntigravityLiveStatus;
}
