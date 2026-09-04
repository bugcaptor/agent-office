// src/renderer/usage/usageView.ts
//
// 구독 사용량(rate limit) 표시용 순수 함수 모음. 백엔드는 정규화된 원시
// 스냅샷만 주고(docs/usage-design.md §3), "가장 절박한 윈도 선택",
// 임계 색상, 카운트다운·신선도 포맷 같은 해석·표시는 여기서 한다. React·스토어
// 의존 없음 — 단위 테스트 대상(설계 §4).
//
// i18n: 표시 함수들은 **완성된 문장이 아니라 번역 키 설명자(TextKey)**를
// 돌려준다. 문장을 여기서 만들어 버리면 언어를 바꿔도 안 바뀌고, 순수 함수
// 테스트가 문구에 묶인다. 실제 번역은 `t`를 쥔 UsageDialog/UsageWidget이
// `renderText`로 한다(workdir/status.ts와 같은 결의 결정).

import type { TextKey } from "@renderer/shared/textKey";
import type {
  AntigravityLiveStatus,
  ClaudeLiveStatus,
  CodexLiveStatus,
  FetchTransport,
  GeminiLiveStatus,
  ProviderUsage,
  UsageSnapshot,
  UsageWindow,
} from "@shared/types";

/** 사용량을 표시하는 provider와 그 고정 순서. */
export const USAGE_PROVIDERS = ["claude", "codex", "antigravity", "gemini"] as const;

export type UsageProvider = (typeof USAGE_PROVIDERS)[number];

/** provider별 실시간 조회 진단의 합집합. 공통 필드만 읽는 자리에서 쓴다. */
export type AnyLiveStatus =
  | ClaudeLiveStatus
  | CodexLiveStatus
  | AntigravityLiveStatus
  | GeminiLiveStatus;

/** 신선도가 이보다 오래되면(ms) stale로 보고 흐리게 표시한다. */
export const STALE_THRESHOLD_MS = 30 * 60 * 1000;

/**
 * 신선도가 이보다 오래되면(ms) 그 provider를 **화면에서 통째로 뺀다**
 * (흐리게가 아니라 아예 표시하지 않음, kbm #2j4).
 *
 * 왜 흐리게로는 부족한가: 하루가 지나도록 갱신되지 않은 숫자는 "낡은 참값"이
 * 아니라 사실상 무의미하다 — 5시간 창은 네 번 넘게, 주간 창도 리셋 경계를
 * 지났을 수 있다. 흐린 숫자는 계속 읽히고, 읽히면 오해를 부른다. 자리만
 * 차지하는 `—`도 같은 이유로 뺀다.
 */
export const DEAD_THRESHOLD_MS = 24 * 60 * 60 * 1000;

/** 사용률 임계 단계. <70 기본 / ≥70 경고 / ≥90 위험. */
export type UsageLevel = "normal" | "warn" | "danger";

export function usageLevel(usedPercent: number): UsageLevel {
  if (usedPercent >= 90) return "danger";
  if (usedPercent >= 70) return "warn";
  return "normal";
}

/** BottomBar 뱃지 접두. */
export const PROVIDER_SHORT: Record<UsageProvider, string> = {
  claude: "CL",
  codex: "CX",
  antigravity: "AG",
  gemini: "GM",
};

/** 스냅샷에서 provider의 값을 꺼낸다(스냅샷이 없으면 null). */
export function providerUsage(
  snapshot: UsageSnapshot | null | undefined,
  provider: UsageProvider,
): ProviderUsage | null {
  return snapshot ? (snapshot[provider] ?? null) : null;
}

/** 스냅샷에서 provider의 실시간 조회 진단을 꺼낸다(없으면 undefined). */
export function providerLive(
  snapshot: UsageSnapshot | null | undefined,
  provider: UsageProvider,
): AnyLiveStatus | undefined {
  if (!snapshot) return undefined;
  switch (provider) {
    case "claude":
      return snapshot.claudeLive;
    case "codex":
      return snapshot.codexLive;
    case "antigravity":
      return snapshot.antigravityLive;
    case "gemini":
      return snapshot.geminiLive;
  }
}

/**
 * 이 provider를 화면에서 아예 빼야 하는지(설계: kbm #2j4).
 *
 * - 값이 있으면 신선도만 본다 — 하루보다 낡았으면 뺀다.
 * - 값이 없으면 **한 번이라도 시도해 봤는지**로 가른다. 아직 시도 전
 *   (`never_attempted`)이나 진단 자체가 없으면(구버전 응답) 부팅 직후일 수
 *   있으니 남겨 두고, 시도했는데도 값이 하나도 없으면(미설치·미로그인) 뺀다.
 *   이 갈래에는 "며칠째"를 잴 시각이 없다 — 실패 시작 시각을 앱 재시작 너머로
 *   보존하지 않기 때문이다. 애초에 보여줄 숫자가 없으므로 기다릴 이유도 없다.
 */
export function isProviderGone(
  usage: ProviderUsage | null,
  live: AnyLiveStatus | null | undefined,
  now: number,
): boolean {
  if (usage) return now - usage.fetchedAtMs > DEAD_THRESHOLD_MS;
  if (!live || live.outcome === "never_attempted") return false;
  return true;
}

/** 지금 그려야 할 provider 목록(고정 순서 유지). 전부 빠지면 빈 배열. */
export function visibleUsageProviders(
  snapshot: UsageSnapshot | null | undefined,
  now: number,
): UsageProvider[] {
  return USAGE_PROVIDERS.filter(
    (p) => !isProviderGone(providerUsage(snapshot, p), providerLive(snapshot, p), now),
  );
}

/**
 * provider의 가장 절박한 윈도(usedPercent 최대) 하나. 윈도가 없으면 null.
 * 동률이면 먼저 나온 윈도를 유지한다(안정적).
 */
export function mostUrgentWindow(usage: ProviderUsage | null): UsageWindow | null {
  if (!usage || usage.windows.length === 0) return null;
  return usage.windows.reduce((best, w) => (w.usedPercent > best.usedPercent ? w : best));
}

/**
 * 뱃지에 표시할 윈도 목록(최대 2). [5시간(session) 창, 계정 전체 주간(weekly)
 * 창] 순서로 반환한다. 주간 창이 없으면 두 번째 자리는 나머지 중 가장 절박한
 * 창으로 채운다. session 창이 없으면 두 번째 자리 후보 하나만, 윈도 자체가
 * 없으면 빈 배열(이슈 #36 — 주간 창이 더 절박할 때 5시간 창 변동이 뱃지에서
 * 안 보이던 문제).
 *
 * 두 번째 자리는 모델별 주간 창(예: Claude Fable)의 사용률이 총 주간 사용률보다
 * 높아도 총 주간 창을 우선한다 — 뱃지는 계정 전체 한도의 대표값이고, 모델별
 * 값은 상세 모달에서 본다.
 *
 * Codex의 모델별 버킷(예: Spark)은 특정 모델에만 쓸 수 있는 특수 한도다.
 * 계정 전체 사용량의 대표값이 아니므로 컴팩트 뱃지에서는 제외하고, 라벨 없는
 * 기본 버킷만 고른다. 모델별 값은 상세 모달의 전체 윈도 목록에 그대로 남는다.
 */
export function badgeWindows(usage: ProviderUsage | null): UsageWindow[] {
  if (!usage || usage.windows.length === 0) return [];
  const windows =
    usage.provider === "codex"
      ? usage.windows.filter((w) => w.label === null)
      : usage.windows;
  if (windows.length === 0) return [];
  const session = windows.find((w) => w.kind === "session") ?? null;
  const rest = windows.filter((w) => w.kind !== "session");
  const second =
    rest.length === 0
      ? null
      : (rest.find((w) => w.kind === "weekly") ??
        rest.reduce((best, w) => (w.usedPercent > best.usedPercent ? w : best)));
  if (!session) return second ? [second] : [];
  return second ? [session, second] : [session];
}

/** 윈도 종류 라벨 키. 모델별 창은 모델명(label)을 곁들인다. */
export function windowLabel(w: UsageWindow): TextKey {
  switch (w.kind) {
    case "session":
      return { key: "usage.window.session" };
    case "weekly":
      return { key: "usage.window.weekly" };
    case "session_model":
      return w.label
        ? { key: "usage.window.sessionModel", params: { label: w.label } }
        : { key: "usage.window.sessionModelGeneric" };
    case "weekly_model":
      return w.label
        ? { key: "usage.window.weeklyModel", params: { label: w.label } }
        : { key: "usage.window.weeklyModelGeneric" };
    case "unknown":
      // 라벨이 있으면 그것이 유일한 뜻이다(Gemini의 모델별 버킷은 창 길이를
      // 주지 않는다) — "기타"로 뭉개면 어느 모델 얘기인지 알 수 없다.
      if (w.label) return { key: "usage.window.labelled", params: { label: w.label } };
      return w.windowMinutes
        ? { key: "usage.window.minutes", params: { minutes: w.windowMinutes } }
        : { key: "usage.window.other" };
  }
}

/**
 * 리셋까지 남은 시간("N시간 N분 후 리셋") 키. 이미 지났으면 "리셋 대기 중",
 * resetsAtMs가 null이면 null(표시 없음 — 예전의 빈 문자열 자리). 하루 이상은
 * "N일 N시간 후 리셋".
 */
export function formatCountdown(resetsAtMs: number | null, now: number): TextKey | null {
  if (resetsAtMs === null) return null;
  const diff = resetsAtMs - now;
  if (diff <= 0) return { key: "usage.countdown.pending" };
  const totalMin = Math.floor(diff / 60000);
  const days = Math.floor(totalMin / (60 * 24));
  const hours = Math.floor((totalMin % (60 * 24)) / 60);
  const mins = totalMin % 60;
  if (days > 0) return { key: "usage.countdown.days", params: { days, hours } };
  if (hours > 0) return { key: "usage.countdown.hours", params: { hours, mins } };
  return { key: "usage.countdown.mins", params: { mins } };
}

/** 경과 시간 문구의 갈래. ago("N분 전")와 freshness("N분 전 기준")가 공유한다. */
type AgoVariant = "justNow" | "days" | "hours" | "mins";

/**
 * 경과 시간 → 갈래 + 보간 파라미터. 임계값(1분 / 60분 / 24시간)과 내림 규칙은
 * 손조립 시절 그대로다 — 표기가 바뀌는 지점을 건드리면 체감이 달라진다.
 */
function agoParts(atMs: number, now: number): { variant: AgoVariant; params?: Record<string, number> } {
  const diff = Math.max(0, now - atMs);
  const totalMin = Math.floor(diff / 60000);
  if (totalMin < 1) return { variant: "justNow" };
  const days = Math.floor(totalMin / (60 * 24));
  const hours = Math.floor((totalMin % (60 * 24)) / 60);
  const mins = totalMin % 60;
  if (days > 0) return { variant: "days", params: { days } };
  if (hours > 0) return { variant: "hours", params: { hours, mins } };
  return { variant: "mins", params: { mins } };
}

/**
 * 과거 시각을 "N분 전"으로. 1분 미만은 "방금", 1시간 이상은 "N시간 N분 전",
 * 하루 이상은 "N일 전".
 */
export function formatAgo(atMs: number, now: number): TextKey {
  const { variant, params } = agoParts(atMs, now);
  return { key: `usage.ago.${variant}`, params };
}

/**
 * 신선도를 "N분 전 기준"으로. ago와 같은 갈래를 쓰되 키만 다르다 — "{{ago}}
 * 기준"처럼 조립하면 언어마다 어순이 다른 자리를 번역자가 못 고친다.
 */
export function formatFreshness(fetchedAtMs: number, now: number): TextKey {
  const { variant, params } = agoParts(fetchedAtMs, now);
  return { key: `usage.freshness.${variant}`, params };
}

/** 신선도가 STALE_THRESHOLD_MS를 넘었는지. */
export function isStale(fetchedAtMs: number, now: number): boolean {
  return now - fetchedAtMs > STALE_THRESHOLD_MS;
}

// ── Claude 실시간 조회 진단 표시(설계 §7) ─────────────────────────────
//
// 왜 필요한가: 실시간 조회가 실패하면 화면 숫자는 `~/.claude.json` 캐시
// 미러로 조용히 강등되는데, 그 캐시는 Claude Code에서 `/usage`를 열 때만
// 갱신된다(일반 대화로는 절대 안 갱신됨 — CLI 2.1.220 실측). 그래서 "쓰고
// 있는데 숫자가 며칠째 그대로"가 원인 불명으로 보인다. 사유를 문구 키로
// 돌려주는 건 여기(순수 함수), 그리기는 UsageDialog/UsageWidget이 한다.
//
// "표시값은 로컬 캐시…" 꼬리말은 카탈로그 쪽에서 i18next `$t()` 중첩으로
// 붙인다(`usage.live.cacheNote`) — 실패 문구마다 같은 문장을 복사하지 않게.

/** 진단 문구 한 줄 + 심각도(색). */
export interface LiveStatusNote {
  level: "ok" | "warn" | "error";
  /** 상세 모달용 전체 문장. */
  text: TextKey;
  /** 위젯 툴팁용 짧은 꼬리표. */
  short: TextKey;
}

/** 전송 수단 표시명. `direct`는 우회가 아니므로 라벨을 쓰지 않는다. */
export function transportLabel(via: FetchTransport): TextKey {
  switch (via) {
    case "direct":
      return { key: "usage.transport.direct" };
    case "curl":
      return { key: "usage.transport.curl" };
    case "claude_cli":
      return { key: "usage.transport.claudeCli" };
  }
}

/** 우회 수단으로 값을 받아오고 있는지(=앱의 직접 HTTPS가 막힌 환경인지). */
function detour(via: FetchTransport | null | undefined): FetchTransport | null {
  return via && via !== "direct" ? via : null;
}

/**
 * 실시간 조회 상태 → 표시 문구 키. `null`(구버전 백엔드 응답 등 필드 부재)이면
 * 아무것도 표시하지 않는다.
 */
export function describeLiveStatus(
  status: ClaudeLiveStatus | null | undefined,
): LiveStatusNote | null {
  if (!status) return null;
  switch (status.outcome) {
    case "ok": {
      // 우회로 성공 중이면 값은 최신이지만 환경 진단으로서 알릴 가치가 있다 —
      // 앱이 직접 거는 HTTPS만 막혀 있다는 뜻이라, 사설 인증서를 신뢰 목록에
      // 넣거나 프록시 설정을 손보면 우회 없이도 돌아간다.
      const via = detour(status.via);
      if (via) {
        return {
          level: "ok",
          text: { key: "usage.live.okDetour", params: { via: transportLabel(via) } },
          short: { key: "usage.live.okDetourShort", params: { via: transportLabel(via) } },
        };
      }
      return {
        level: "ok",
        text: { key: "usage.live.okDirect" },
        short: { key: "usage.live.okDirectShort" },
      };
    }
    case "never_attempted":
      return {
        level: "warn",
        text: { key: "usage.live.neverAttempted" },
        short: { key: "usage.live.waitingShort" },
      };
    case "no_credentials":
      return {
        level: "error",
        text: { key: "usage.live.noCredentials" },
        short: { key: "usage.live.noCredentialsShort" },
      };
    case "unauthorized":
      return {
        level: "error",
        text: {
          key:
            status.tokenSource === "file"
              ? "usage.live.unauthorizedFile"
              : "usage.live.unauthorizedKeychain",
          params: { detail: status.detail ?? "HTTP 401" },
        },
        short: { key: "usage.live.unauthorizedShort" },
      };
    case "http_error":
      return {
        level: "error",
        text: {
          key: "usage.live.httpError",
          params: { detail: status.detail ?? { key: "usage.live.detailHttpError" } },
        },
        short: {
          key: "usage.live.httpErrorShort",
          params: { detail: status.detail ?? { key: "usage.live.detailHttpError" } },
        },
      };
    case "network_error":
      return {
        level: "warn",
        text: {
          key: "usage.live.networkError",
          params: { detail: status.detail ?? { key: "usage.live.detailError" } },
        },
        short: { key: "usage.live.networkErrorShort" },
      };
    case "unexpected_response":
      return {
        level: "warn",
        text: {
          key: "usage.live.unexpected",
          params: { detail: status.detail ?? { key: "usage.live.detailUnknown" } },
        },
        short: { key: "usage.live.unexpectedShort" },
      };
  }
}

// ── Codex 실시간 조회 진단 표시 ──────────────────────────────────────
//
// Claude와 같은 목적, 다른 어휘. Codex는 앱이 자격증명을 만지지 않고 codex
// CLI(`codex app-server`의 account/rateLimits/read RPC)에 물어보므로, 실패는
// 인증이 아니라 "CLI가 없다/죽었다/모르는 응답을 줬다"로 나타난다. 실패하면
// 표시값은 rollout jsonl 스냅샷으로 강등되는데, 그건 Codex CLI가 실제로 돌
// 때만 갱신된다 — 그게 "쓰지도 않았는데 숫자가 그대로"의 정체다.
// 공통 꼬리말은 `usage.codexLive.cacheNote`(카탈로그 `$t()` 중첩).

/**
 * Codex 실시간 조회 상태 → 표시 문구 키. `null`(구버전 백엔드 응답 등 필드
 * 부재)이면 아무것도 표시하지 않는다.
 */
export function describeCodexLiveStatus(
  status: CodexLiveStatus | null | undefined,
): LiveStatusNote | null {
  if (!status) return null;
  switch (status.outcome) {
    case "ok":
      return {
        level: "ok",
        text: { key: "usage.codexLive.ok" },
        short: { key: "usage.live.okDirectShort" },
      };
    case "never_attempted":
      return {
        level: "warn",
        text: { key: "usage.codexLive.neverAttempted" },
        short: { key: "usage.live.waitingShort" },
      };
    case "cli_missing":
      return {
        level: "error",
        text: { key: "usage.codexLive.cliMissing" },
        short: { key: "usage.codexLive.cliMissingShort" },
      };
    case "cli_failed":
      return {
        level: "error",
        text: {
          key: "usage.codexLive.cliFailed",
          params: { detail: status.detail ?? { key: "usage.codexLive.detailUnknownCause" } },
        },
        short: { key: "usage.codexLive.cliFailedShort" },
      };
    case "timeout":
      return {
        level: "warn",
        text: { key: "usage.codexLive.timeout" },
        short: { key: "usage.codexLive.timeoutShort" },
      };
    case "rpc_error":
      return {
        level: "error",
        text: {
          key: "usage.codexLive.rpcError",
          params: { detail: status.detail ?? { key: "usage.live.detailUnknown" } },
        },
        short: { key: "usage.codexLive.rpcErrorShort" },
      };
    case "unexpected_response":
      return {
        level: "warn",
        text: {
          key: "usage.codexLive.unexpected",
          params: { detail: status.detail ?? { key: "usage.live.detailUnknown" } },
        },
        short: { key: "usage.codexLive.unexpectedShort" },
      };
  }
}

// ── Antigravity 실시간 조회 진단 표시 ────────────────────────────────
//
// Codex와 같은 결(자격증명을 앱이 만지지 않고 CLI에 물어본다)이되, 결정적
// 차이가 하나 있다: **강등할 로컬 파일 캐시가 없다.** 그래서 실패 문구의
// 꼬리말이 "표시값은 로컬 캐시…"가 아니라 "표시값이 없거나 직전 조회 값"이다
// (`usage.antigravityLive.noCacheNote`).

/**
 * Antigravity 실시간 조회 상태 → 표시 문구 키. `null`(구버전 백엔드 응답 등
 * 필드 부재)이면 아무것도 표시하지 않는다.
 */
export function describeAntigravityLiveStatus(
  status: AntigravityLiveStatus | null | undefined,
): LiveStatusNote | null {
  if (!status) return null;
  switch (status.outcome) {
    case "ok":
      return {
        level: "ok",
        text: { key: "usage.antigravityLive.ok" },
        short: { key: "usage.live.okDirectShort" },
      };
    case "never_attempted":
      return {
        level: "warn",
        text: { key: "usage.antigravityLive.neverAttempted" },
        short: { key: "usage.live.waitingShort" },
      };
    case "cli_missing":
      return {
        level: "error",
        text: { key: "usage.antigravityLive.cliMissing" },
        short: { key: "usage.antigravityLive.cliMissingShort" },
      };
    case "cli_failed":
      return {
        level: "error",
        text: {
          key: "usage.antigravityLive.cliFailed",
          params: { detail: status.detail ?? { key: "usage.codexLive.detailUnknownCause" } },
        },
        short: { key: "usage.antigravityLive.cliFailedShort" },
      };
    case "timeout":
      return {
        level: "warn",
        text: { key: "usage.antigravityLive.timeout" },
        short: { key: "usage.codexLive.timeoutShort" },
      };
    case "command_failed":
      return {
        level: "error",
        text: {
          key: "usage.antigravityLive.commandFailed",
          params: { detail: status.detail ?? { key: "usage.live.detailUnknown" } },
        },
        short: { key: "usage.antigravityLive.commandFailedShort" },
      };
    case "unexpected_response":
      return {
        level: "warn",
        text: {
          key: "usage.antigravityLive.unexpected",
          params: { detail: status.detail ?? { key: "usage.live.detailUnknown" } },
        },
        short: { key: "usage.codexLive.unexpectedShort" },
      };
  }
}

// ── Gemini 실시간 조회 진단 표시 ─────────────────────────────────────
//
// Claude와 같은 HTTP 어휘(우리가 직접 조회하는 경로라서)에 이 API만의 두
// 갈래가 더 있다. `ineligible`은 **오류가 아니다** — 계정에 Code Assist
// 라이선스가 없다는 사실 진술이고, 개인 무료 티어가 Antigravity로 이관된 뒤의
// 기본 상태다. 그래서 error가 아니라 warn 단계로 두고 "Antigravity를 보라"고
// 안내한다. `project_required`는 반대로 env 한 줄이면 풀린다.

/**
 * Gemini 실시간 조회 상태 → 표시 문구 키. `null`(구버전 백엔드 응답 등 필드
 * 부재)이면 아무것도 표시하지 않는다.
 */
export function describeGeminiLiveStatus(
  status: GeminiLiveStatus | null | undefined,
): LiveStatusNote | null {
  if (!status) return null;
  switch (status.outcome) {
    case "ok":
      return {
        level: "ok",
        text: { key: "usage.geminiLive.ok" },
        short: { key: "usage.live.okDirectShort" },
      };
    case "never_attempted":
      return {
        level: "warn",
        text: { key: "usage.geminiLive.neverAttempted" },
        short: { key: "usage.live.waitingShort" },
      };
    case "no_credentials":
      return {
        level: "error",
        text: { key: "usage.geminiLive.noCredentials" },
        short: { key: "usage.live.noCredentialsShort" },
      };
    case "refresh_failed":
      return {
        level: "error",
        text: {
          key: "usage.geminiLive.refreshFailed",
          params: { detail: status.detail ?? { key: "usage.live.detailUnknown" } },
        },
        short: { key: "usage.geminiLive.refreshFailedShort" },
      };
    case "unauthorized":
      return {
        level: "error",
        text: {
          key: "usage.geminiLive.unauthorized",
          params: { detail: status.detail ?? "HTTP 401" },
        },
        short: { key: "usage.live.unauthorizedShort" },
      };
    case "ineligible":
      return {
        level: "warn",
        text: {
          key: "usage.geminiLive.ineligible",
          params: { detail: status.detail ?? { key: "usage.geminiLive.detailNoLicense" } },
        },
        short: { key: "usage.geminiLive.ineligibleShort" },
      };
    case "project_required":
      return {
        level: "error",
        text: { key: "usage.geminiLive.projectRequired" },
        short: { key: "usage.geminiLive.projectRequiredShort" },
      };
    case "http_error":
      return {
        level: "error",
        text: {
          key: "usage.geminiLive.httpError",
          params: { detail: status.detail ?? { key: "usage.live.detailHttpError" } },
        },
        short: {
          key: "usage.live.httpErrorShort",
          params: { detail: status.detail ?? { key: "usage.live.detailHttpError" } },
        },
      };
    case "network_error":
      return {
        level: "warn",
        text: {
          key: "usage.geminiLive.networkError",
          params: { detail: status.detail ?? { key: "usage.live.detailError" } },
        },
        short: { key: "usage.live.networkErrorShort" },
      };
    case "unexpected_response":
      return {
        level: "warn",
        text: {
          key: "usage.geminiLive.unexpected",
          params: { detail: status.detail ?? { key: "usage.live.detailUnknown" } },
        },
        short: { key: "usage.live.unexpectedShort" },
      };
  }
}

/**
 * provider 하나의 실시간 조회 진단을 해석한다. provider마다 조회 경로가 달라
 * (Claude=HTTPS 직접 조회, Codex=codex CLI RPC, Antigravity=agy print 모드)
 * 해석 함수도 각자 것이며, 이 갈래를 화면 컴포넌트마다 되풀이하지 않도록
 * 한 곳에 모아 둔다.
 */
export function describeProviderLive(
  snapshot: UsageSnapshot | null | undefined,
  provider: UsageProvider,
): LiveStatusNote | null {
  if (!snapshot) return null;
  switch (provider) {
    case "claude":
      return describeLiveStatus(snapshot.claudeLive);
    case "codex":
      return describeCodexLiveStatus(snapshot.codexLive);
    case "antigravity":
      return describeAntigravityLiveStatus(snapshot.antigravityLive);
    case "gemini":
      return describeGeminiLiveStatus(snapshot.geminiLive);
  }
}

/**
 * 진단 문구 뒤에 붙일 "마지막 시도 N분 전 · 마지막 성공 N분 전"의 **조각들**.
 * 잇는 건(가운뎃점) 렌더 쪽 몫이다 — 조각 수가 0~2개로 달라지는 자리라
 * 카탈로그에 넣어 봐야 번역자가 손댈 게 없다.
 * 두 provider의 진단이 공유하는 필드만 읽는다(`via`는 Claude에만 있다).
 */
export function formatLiveAttempts(status: AnyLiveStatus, now: number): TextKey[] {
  const parts: TextKey[] = [];
  if (status.lastAttemptMs !== null) {
    parts.push({ key: "usage.attempts.lastAttempt", params: { ago: formatAgo(status.lastAttemptMs, now) } });
  }
  if (status.lastSuccessMs !== null) {
    // 실패 중이어도 "그 값을 무엇으로 받아왔는지"는 남는다(via는 마지막
    // 성공의 수단이다) — 우회로 연명 중인 환경을 여기서도 읽을 수 있게 한다.
    const via = detour("via" in status ? status.via : null);
    const ago = formatAgo(status.lastSuccessMs, now);
    parts.push(
      via
        ? { key: "usage.attempts.lastSuccessVia", params: { ago, via: transportLabel(via) } }
        : { key: "usage.attempts.lastSuccess", params: { ago } },
    );
  } else if (status.outcome !== "never_attempted") parts.push({ key: "usage.attempts.noSuccess" });
  return parts;
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
    antigravity: fresherProvider(prev?.antigravity ?? null, next.antigravity),
    gemini: fresherProvider(prev?.gemini ?? null, next.gemini),
    // 진단은 병합하지 않고 항상 최신 응답을 쓴다 — 값(누적)과 달리 "지금
    // 상태"라서 이전 것을 살려두면 이미 복구된 실패가 남는다.
    claudeLive: next.claudeLive ?? prev?.claudeLive,
    codexLive: next.codexLive ?? prev?.codexLive,
    antigravityLive: next.antigravityLive ?? prev?.antigravityLive,
    geminiLive: next.geminiLive ?? prev?.geminiLive,
  };
}
