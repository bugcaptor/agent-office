// src/renderer/usage/__tests__/usageView.test.ts
//
// 사용량 표시 순수 함수 테스트(docs/usage-limits-design.md §4): 절박 윈도
// 선택, 임계 색상, 카운트다운·신선도 포맷, stale 판정.
//
// i18n 이후 이 모듈은 **문장이 아니라 번역 키 설명자**를 돌려주므로, 순수
// 함수 단언은 키·보간 기준이다. 문구 자체(정본 ko가 예전과 글자 하나까지
// 같은지, en이 말이 되는지)는 아래 "카탈로그 렌더" 묶음이 지킨다.

import { afterAll, describe, expect, it } from "vitest";
import { SOURCE_LANGUAGE, initI18nForTest, t as translate } from "@renderer/i18n";
import { renderText } from "@renderer/shared/textKey";
import type {
  AntigravityLiveStatus,
  ClaudeLiveStatus,
  CodexLiveStatus,
  ProviderUsage,
  UsageSnapshot,
  UsageWindow,
} from "@shared/types";
import {
  DEAD_THRESHOLD_MS,
  STALE_THRESHOLD_MS,
  badgeWindows,
  describeAntigravityLiveStatus,
  describeCodexLiveStatus,
  describeLiveStatus,
  isProviderGone,
  visibleUsageProviders,
  formatAgo,
  formatCountdown,
  formatFreshness,
  formatLiveAttempts,
  isStale,
  mergeUsageSnapshot,
  mostUrgentWindow,
  usageLevel,
  windowLabel,
} from "../usageView";
import type { TextKey } from "@renderer/shared/textKey";

/** 모듈 `t`는 ns 접두가 필요하다(`useTranslation("activity")`와 같은 결과). */
const tr = (key: string, params?: Record<string, string | number>) =>
  translate(`activity:${key}`, params);

/** 설명자 → 실제 문구(현재 언어). */
const render = (text: TextKey): string => renderText(text, tr);

/** 진단 조각들을 화면과 같은 방식(가운뎃점)으로 잇는다. */
const renderAttempts = (parts: TextKey[]): string => parts.map(render).join(" · ");

function win(partial: Partial<UsageWindow>): UsageWindow {
  return {
    kind: "session",
    label: null,
    usedPercent: 0,
    resetsAtMs: null,
    windowMinutes: null,
    isActive: null,
    ...partial,
  };
}

function provider(windows: UsageWindow[]): ProviderUsage {
  return { provider: "claude", fetchedAtMs: 0, planLabel: null, windows };
}

function live(partial: Partial<ClaudeLiveStatus> = {}): ClaudeLiveStatus {
  return {
    outcome: "ok",
    tokenSource: "keychain_legacy",
    detail: null,
    lastAttemptMs: null,
    lastSuccessMs: null,
    via: "direct",
    ...partial,
  };
}

function codexLiveStatus(partial: Partial<CodexLiveStatus> = {}): CodexLiveStatus {
  return { outcome: "ok", detail: null, lastAttemptMs: null, lastSuccessMs: null, ...partial };
}

function antigravityLiveStatus(
  partial: Partial<AntigravityLiveStatus> = {},
): AntigravityLiveStatus {
  return { outcome: "ok", detail: null, lastAttemptMs: null, lastSuccessMs: null, ...partial };
}

function snap(
  claude: ProviderUsage | null,
  codex: ProviderUsage | null,
  claudeLive: ClaudeLiveStatus = live(),
  codexLive: CodexLiveStatus = codexLiveStatus(),
  antigravity: ProviderUsage | null = null,
  antigravityLive: AntigravityLiveStatus = antigravityLiveStatus(),
): UsageSnapshot {
  return { claude, codex, antigravity, claudeLive, codexLive, antigravityLive };
}

describe("usageLevel 임계 70/90", () => {
  it("<70 = normal", () => {
    expect(usageLevel(0)).toBe("normal");
    expect(usageLevel(69.9)).toBe("normal");
  });
  it(">=70 = warn", () => {
    expect(usageLevel(70)).toBe("warn");
    expect(usageLevel(89.9)).toBe("warn");
  });
  it(">=90 = danger", () => {
    expect(usageLevel(90)).toBe("danger");
    expect(usageLevel(100)).toBe("danger");
  });
});

describe("mostUrgentWindow", () => {
  it("usedPercent 최대 윈도를 고른다", () => {
    const u = provider([
      win({ kind: "session", usedPercent: 61 }),
      win({ kind: "weekly", usedPercent: 18 }),
      win({ kind: "weekly_model", usedPercent: 24 }),
    ]);
    expect(mostUrgentWindow(u)?.kind).toBe("session");
  });

  it("동률이면 먼저 나온 윈도를 유지한다", () => {
    const u = provider([
      win({ kind: "weekly", usedPercent: 50 }),
      win({ kind: "session", usedPercent: 50 }),
    ]);
    expect(mostUrgentWindow(u)?.kind).toBe("weekly");
  });

  it("null/빈 윈도는 null", () => {
    expect(mostUrgentWindow(null)).toBeNull();
    expect(mostUrgentWindow(provider([]))).toBeNull();
  });
});

describe("badgeWindows", () => {
  it("session+weekly 둘 다 있으면 [session, weekly] 순서", () => {
    const u = provider([
      win({ kind: "session", usedPercent: 12 }),
      win({ kind: "weekly", usedPercent: 61 }),
    ]);
    expect(badgeWindows(u).map((w) => w.kind)).toEqual(["session", "weekly"]);
  });

  it("session이 없으면 가장 절박한 창 하나만", () => {
    const u = provider([
      win({ kind: "weekly", usedPercent: 18 }),
      win({ kind: "weekly_model", usedPercent: 24 }),
    ]);
    const result = badgeWindows(u);
    expect(result).toHaveLength(1);
    expect(result[0].kind).toBe("weekly_model");
    expect(result[0].usedPercent).toBe(24);
  });

  it("윈도가 없으면 빈 배열", () => {
    expect(badgeWindows(null)).toEqual([]);
    expect(badgeWindows(provider([]))).toEqual([]);
  });

  it("session만 있으면 [session] 하나", () => {
    const u = provider([win({ kind: "session", usedPercent: 33 })]);
    const result = badgeWindows(u);
    expect(result).toHaveLength(1);
    expect(result[0].kind).toBe("session");
  });

  it("weekly_model 여러 개 중 최대를 두 번째로 고른다", () => {
    const u = provider([
      win({ kind: "session", usedPercent: 12 }),
      win({ kind: "weekly_model", label: "A", usedPercent: 40 }),
      win({ kind: "weekly_model", label: "B", usedPercent: 70 }),
      win({ kind: "weekly_model", label: "C", usedPercent: 55 }),
    ]);
    const result = badgeWindows(u);
    expect(result.map((w) => w.kind)).toEqual(["session", "weekly_model"]);
    expect(result[1].label).toBe("B");
    expect(result[1].usedPercent).toBe(70);
  });
});

describe("windowLabel", () => {
  it("종류별 라벨 키", () => {
    expect(windowLabel(win({ kind: "session" }))).toEqual({ key: "usage.window.session" });
    expect(windowLabel(win({ kind: "weekly" }))).toEqual({ key: "usage.window.weekly" });
    expect(windowLabel(win({ kind: "weekly_model", label: "Fable" }))).toEqual({
      key: "usage.window.weeklyModel",
      params: { label: "Fable" },
    });
    expect(windowLabel(win({ kind: "weekly_model", label: null }))).toEqual({
      key: "usage.window.weeklyModelGeneric",
    });
    expect(windowLabel(win({ kind: "unknown", windowMinutes: 1440 }))).toEqual({
      key: "usage.window.minutes",
      params: { minutes: 1440 },
    });
    expect(windowLabel(win({ kind: "unknown", windowMinutes: null }))).toEqual({
      key: "usage.window.other",
    });
  });
});

describe("formatCountdown", () => {
  const NOW = 1_784_000_000_000;
  it("resetsAtMs null이면 표시 없음(null)", () => {
    expect(formatCountdown(null, NOW)).toBeNull();
  });
  it("이미 지났으면 리셋 대기 중", () => {
    expect(formatCountdown(NOW - 1000, NOW)).toEqual({ key: "usage.countdown.pending" });
    expect(formatCountdown(NOW, NOW)).toEqual({ key: "usage.countdown.pending" });
  });
  it("분 단위", () => {
    expect(formatCountdown(NOW + 45 * 60000, NOW)).toEqual({
      key: "usage.countdown.mins",
      params: { mins: 45 },
    });
  });
  it("시간+분", () => {
    expect(formatCountdown(NOW + (3 * 60 + 12) * 60000, NOW)).toEqual({
      key: "usage.countdown.hours",
      params: { hours: 3, mins: 12 },
    });
  });
  it("하루 이상은 일+시간", () => {
    expect(formatCountdown(NOW + (2 * 24 * 60 + 5 * 60) * 60000, NOW)).toEqual({
      key: "usage.countdown.days",
      params: { days: 2, hours: 5 },
    });
  });
});

describe("formatFreshness / isStale", () => {
  const NOW = 1_784_000_000_000;
  it("1분 미만은 방금 갈래", () => {
    expect(formatFreshness(NOW - 30_000, NOW)).toEqual({ key: "usage.freshness.justNow" });
  });
  it("분 단위", () => {
    expect(formatFreshness(NOW - 14 * 60000, NOW)).toEqual({
      key: "usage.freshness.mins",
      params: { mins: 14 },
    });
  });
  it("시간+분", () => {
    expect(formatFreshness(NOW - (2 * 60 + 3) * 60000, NOW)).toEqual({
      key: "usage.freshness.hours",
      params: { hours: 2, mins: 3 },
    });
  });
  it("하루 이상", () => {
    expect(formatFreshness(NOW - 3 * 24 * 60 * 60000, NOW)).toEqual({
      key: "usage.freshness.days",
      params: { days: 3 },
    });
  });
  it("미래 신선도(시계 밀림)는 방금으로 클램프", () => {
    expect(formatFreshness(NOW + 60_000, NOW)).toEqual({ key: "usage.freshness.justNow" });
  });
  it("stale 임계는 30분 초과", () => {
    expect(isStale(NOW - STALE_THRESHOLD_MS, NOW)).toBe(false);
    expect(isStale(NOW - STALE_THRESHOLD_MS - 1, NOW)).toBe(true);
  });
});

describe("mergeUsageSnapshot", () => {
  const claudeUsage: ProviderUsage = provider([win({ usedPercent: 42 })]);
  const codexUsage: ProviderUsage = {
    provider: "codex",
    fetchedAtMs: 100,
    planLabel: null,
    windows: [win({ usedPercent: 7 })],
  };

  it("새 값이 null이면 이전 값을 유지한다", () => {
    const merged = mergeUsageSnapshot(snap(claudeUsage, codexUsage), snap(null, null));
    expect(merged.claude).toEqual(claudeUsage);
    expect(merged.codex).toEqual(codexUsage);
  });

  it("새 값이 있으면 교체한다", () => {
    const newerClaude: ProviderUsage = provider([win({ usedPercent: 55 })]);
    const merged = mergeUsageSnapshot(snap(claudeUsage, codexUsage), snap(newerClaude, null));
    expect(merged.claude).toEqual(newerClaude);
    expect(merged.codex).toEqual(codexUsage);
  });

  it("실시간 진단은 병합하지 않고 항상 새 응답을 쓴다", () => {
    // 값(누적)과 달리 진단은 "지금 상태" — 이전 실패가 남으면 이미 복구된
    // 상태를 계속 실패로 보여주게 된다.
    const prev = snap(claudeUsage, codexUsage, live({ outcome: "unauthorized" }));
    const next = snap(null, null, live({ outcome: "ok" }));
    expect(mergeUsageSnapshot(prev, next).claudeLive.outcome).toBe("ok");
  });

  it("새 값이 이전 값보다 오래된 스냅샷이면(fetchedAtMs) 이전 값을 유지한다", () => {
    // codex::load의 best-available 폴백: 최신 rollout이 일시적으로 못 읽히면
    // 더 오래된 파일의 스냅샷이 온다 — 메모리상 신선한 값이 역행하면 안 된다.
    const fresh: ProviderUsage = { ...codexUsage, fetchedAtMs: 200 };
    const staleFallback: ProviderUsage = {
      ...codexUsage,
      fetchedAtMs: 100,
      windows: [win({ usedPercent: 3 })],
    };
    expect(mergeUsageSnapshot(snap(null, fresh), snap(null, staleFallback)).codex).toEqual(fresh);
  });

  it("fetchedAtMs 동률이면 새 값을 쓴다", () => {
    const sameTs: ProviderUsage = { ...codexUsage, windows: [win({ usedPercent: 9 })] };
    expect(mergeUsageSnapshot(snap(null, codexUsage), snap(null, sameTs)).codex).toEqual(sameTs);
  });

  it("prev가 null이면 next를 그대로 쓴다", () => {
    const next = snap(claudeUsage, null);
    expect(mergeUsageSnapshot(null, next)).toEqual(next);
  });

  it("prev도 next도 없는 provider는 null 그대로", () => {
    const merged = mergeUsageSnapshot(null, snap(null, null));
    expect(merged.claude).toBeNull();
    expect(merged.codex).toBeNull();
  });
});

describe("formatAgo", () => {
  const NOW = 1_784_000_000_000;
  it("1분 미만은 방금 갈래", () => {
    expect(formatAgo(NOW - 30_000, NOW)).toEqual({ key: "usage.ago.justNow" });
  });
  it("분/시간/일 단위", () => {
    expect(formatAgo(NOW - 14 * 60000, NOW)).toEqual({
      key: "usage.ago.mins",
      params: { mins: 14 },
    });
    expect(formatAgo(NOW - (2 * 60 + 3) * 60000, NOW)).toEqual({
      key: "usage.ago.hours",
      params: { hours: 2, mins: 3 },
    });
    expect(formatAgo(NOW - 3 * 24 * 60 * 60000, NOW)).toEqual({
      key: "usage.ago.days",
      params: { days: 3 },
    });
  });
});

describe("describeLiveStatus", () => {
  it("성공 중이면 ok 레벨(정상임을 확인할 수 있어야 진단으로 쓸모가 있다)", () => {
    const note = describeLiveStatus(live({ outcome: "ok" }))!;
    expect(note.level).toBe("ok");
    expect(note.text).toEqual({ key: "usage.live.okDirect" });
  });

  it("자격증명 없음은 error 단계", () => {
    const note = describeLiveStatus(live({ outcome: "no_credentials", tokenSource: null }))!;
    expect(note.level).toBe("error");
    expect(note.text).toEqual({ key: "usage.live.noCredentials" });
  });

  /** 실제로 가장 흔한 실패 조합 — 사유가 이 구분을 하지 못하면 진단 가치가 없다. */
  it("401 + 파일 토큰이면 파일 폴백 만료 쪽 키를 고른다", () => {
    const note = describeLiveStatus(
      live({ outcome: "unauthorized", tokenSource: "file", detail: "HTTP 401" }),
    )!;
    expect(note.level).toBe("error");
    expect(note.text).toEqual({
      key: "usage.live.unauthorizedFile",
      params: { detail: "HTTP 401" },
    });
  });

  it("401 + Keychain 토큰이면 재로그인 쪽 키를 고른다", () => {
    const note = describeLiveStatus(
      live({ outcome: "unauthorized", tokenSource: "keychain_legacy" }),
    )!;
    expect(note.text.key).toBe("usage.live.unauthorizedKeychain");
    // detail이 없으면 예전 그대로 "HTTP 401"로 채운다.
    expect(note.text.params).toEqual({ detail: "HTTP 401" });
  });

  it("detail이 없는 실패는 기본 문구 키를 보간 자리에 끼운다", () => {
    expect(describeLiveStatus(live({ outcome: "http_error" }))!.text.params).toEqual({
      detail: { key: "usage.live.detailHttpError" },
    });
    expect(describeLiveStatus(live({ outcome: "network_error" }))!.text.params).toEqual({
      detail: { key: "usage.live.detailError" },
    });
    expect(describeLiveStatus(live({ outcome: "unexpected_response" }))!.text.params).toEqual({
      detail: { key: "usage.live.detailUnknown" },
    });
  });

  it("네트워크/응답 형태 문제는 warn(일시적일 수 있음)", () => {
    expect(describeLiveStatus(live({ outcome: "network_error" }))!.level).toBe("warn");
    expect(describeLiveStatus(live({ outcome: "unexpected_response" }))!.level).toBe("warn");
  });

  it("상태가 없으면(구버전 응답) 아무 문구도 만들지 않는다", () => {
    expect(describeLiveStatus(null)).toBeNull();
    expect(describeLiveStatus(undefined)).toBeNull();
  });

  // ── 우회 전송(사내 프록시·사설 인증서 환경) ──

  it("curl 우회로 성공 중이면 값은 정상이라고 하되 우회 사실을 알린다", () => {
    const note = describeLiveStatus(live({ outcome: "ok", via: "curl" }))!;
    expect(note.level).toBe("ok");
    expect(note.short).toEqual({
      key: "usage.live.okDetourShort",
      params: { via: { key: "usage.transport.curl" } },
    });
    expect(note.text.key).toBe("usage.live.okDetour");
  });

  it("claude CLI 우회도 같은 방식으로 알린다", () => {
    const note = describeLiveStatus(live({ outcome: "ok", via: "claude_cli" }))!;
    expect(note.level).toBe("ok");
    expect(note.short.params).toEqual({ via: { key: "usage.transport.claudeCli" } });
  });

  it("직접 조회 성공에는 우회 문구가 붙지 않는다", () => {
    const note = describeLiveStatus(live({ outcome: "ok", via: "direct" }))!;
    expect(note.short).toEqual({ key: "usage.live.okDirectShort" });
  });

  it("via가 없는 구버전 응답도 직접 조회로 취급한다", () => {
    const note = describeLiveStatus(live({ outcome: "ok", via: null }))!;
    expect(note.short).toEqual({ key: "usage.live.okDirectShort" });
  });
});

describe("formatLiveAttempts", () => {
  const NOW = 1_784_000_000_000;
  it("시도·성공 시각을 상대 시간 조각으로 낸다", () => {
    const s = live({
      outcome: "ok",
      lastAttemptMs: NOW - 3 * 60000,
      lastSuccessMs: NOW - 3 * 60000,
    });
    expect(formatLiveAttempts(s, NOW)).toEqual([
      { key: "usage.attempts.lastAttempt", params: { ago: { key: "usage.ago.mins", params: { mins: 3 } } } },
      { key: "usage.attempts.lastSuccess", params: { ago: { key: "usage.ago.mins", params: { mins: 3 } } } },
    ]);
  });

  it("성공 이력이 없으면 그렇게 말한다", () => {
    const s = live({ outcome: "unauthorized", lastAttemptMs: NOW - 60000, lastSuccessMs: null });
    expect(formatLiveAttempts(s, NOW).map((p) => p.key)).toEqual([
      "usage.attempts.lastAttempt",
      "usage.attempts.noSuccess",
    ]);
  });

  it("한 번도 시도 안 했으면 조각이 없다", () => {
    expect(formatLiveAttempts(live({ outcome: "never_attempted" }), NOW)).toEqual([]);
  });

  it("실패 중이어도 그 값을 무엇으로 받아왔는지는 남는다", () => {
    // via는 마지막 '성공'의 수단이라 실패가 지우지 않는다.
    const s = live({
      outcome: "network_error",
      lastAttemptMs: NOW - 60000,
      lastSuccessMs: NOW - 20 * 60000,
      via: "curl",
    });
    const parts = formatLiveAttempts(s, NOW);
    expect(parts[1].key).toBe("usage.attempts.lastSuccessVia");
    expect(parts[1].params?.via).toEqual({ key: "usage.transport.curl" });
  });
});

describe("describeCodexLiveStatus", () => {
  it("성공은 ok 단계 한 줄 — 정상임을 확인할 수 있어야 진단으로 쓸모가 있다", () => {
    const note = describeCodexLiveStatus(codexLiveStatus())!;
    expect(note.level).toBe("ok");
    expect(note.short).toEqual({ key: "usage.live.okDirectShort" });
  });

  it("실패는 사유별 키를 고른다", () => {
    const missing = describeCodexLiveStatus(codexLiveStatus({ outcome: "cli_missing" }))!;
    expect(missing.level).toBe("error");
    expect(missing.text).toEqual({ key: "usage.codexLive.cliMissing" });
    const rpc = describeCodexLiveStatus(
      codexLiveStatus({ outcome: "rpc_error", detail: "not logged in" }),
    )!;
    expect(rpc.text).toEqual({
      key: "usage.codexLive.rpcError",
      params: { detail: "not logged in" },
    });
    expect(describeCodexLiveStatus(codexLiveStatus({ outcome: "timeout" }))!.level).toBe("warn");
  });

  it("아직 시도 전이면 경고 단계", () => {
    const note = describeCodexLiveStatus(codexLiveStatus({ outcome: "never_attempted" }))!;
    expect(note.level).toBe("warn");
    expect(note.short).toEqual({ key: "usage.live.waitingShort" });
  });

  it("필드 자체가 없으면(구버전 응답) 아무것도 표시하지 않는다", () => {
    expect(describeCodexLiveStatus(null)).toBeNull();
    expect(describeCodexLiveStatus(undefined)).toBeNull();
  });

  it("formatLiveAttempts는 via가 없는 Codex 진단도 그대로 받는다", () => {
    const parts = formatLiveAttempts(
      codexLiveStatus({ lastAttemptMs: 1_000, lastSuccessMs: 1_000 }),
      61_000,
    );
    expect(parts.map((p) => p.key)).toEqual([
      "usage.attempts.lastAttempt",
      "usage.attempts.lastSuccess",
    ]);
  });
});

describe("모델별 창 라벨", () => {
  it("session_model은 5시간 창 키에 모델명을 곁들인다", () => {
    expect(windowLabel(win({ kind: "session_model", label: "Spark" }))).toEqual({
      key: "usage.window.sessionModel",
      params: { label: "Spark" },
    });
    expect(windowLabel(win({ kind: "session_model", label: null }))).toEqual({
      key: "usage.window.sessionModelGeneric",
    });
  });

  it("뱃지의 5시간 자리는 모델별 창이 가로채지 않는다", () => {
    // session_model만 있고 plain session이 없으면 5시간 자리는 비고, 가장
    // 절박한 창 하나만 나온다(모델 창이 계정 전체 한도인 척하지 않게).
    const windows = [
      win({ kind: "weekly", usedPercent: 30 }),
      win({ kind: "session_model", label: "Spark", usedPercent: 4 }),
    ];
    expect(badgeWindows(provider(windows)).map((w) => w.kind)).toEqual(["weekly"]);
  });
});

describe("isProviderGone (하루 넘게 실패하면 아예 숨김)", () => {
  const now = 10 * DEAD_THRESHOLD_MS;
  const at = (fetchedAtMs: number): ProviderUsage => ({
    provider: "claude",
    fetchedAtMs,
    planLabel: null,
    windows: [win({ usedPercent: 10 })],
  });

  it("값이 하루 이내면 남긴다 — 30분 stale 경계와 무관하다", () => {
    expect(isProviderGone(at(now - STALE_THRESHOLD_MS - 1), live(), now)).toBe(false);
    expect(isProviderGone(at(now - DEAD_THRESHOLD_MS), live(), now)).toBe(false);
  });

  it("값이 하루보다 낡으면 뺀다", () => {
    expect(isProviderGone(at(now - DEAD_THRESHOLD_MS - 1), live(), now)).toBe(true);
  });

  it("값이 없어도 첫 조회 전이면 남긴다(부팅 직후)", () => {
    expect(isProviderGone(null, live({ outcome: "never_attempted" }), now)).toBe(false);
    // 진단 필드 자체가 없는 구버전 응답도 같은 취급.
    expect(isProviderGone(null, null, now)).toBe(false);
  });

  it("시도했는데 값이 하나도 없으면 뺀다(미설치·미로그인)", () => {
    expect(isProviderGone(null, codexLiveStatus({ outcome: "cli_missing" }), now)).toBe(true);
    // 조회는 성공했다는데 값이 없는 경우도 보여줄 숫자가 없기는 마찬가지다.
    expect(isProviderGone(null, codexLiveStatus({ outcome: "ok" }), now)).toBe(true);
  });
});

describe("visibleUsageProviders", () => {
  const now = 10 * DEAD_THRESHOLD_MS;
  const fresh = (): ProviderUsage => ({
    provider: "claude",
    fetchedAtMs: now,
    planLabel: null,
    windows: [win({ usedPercent: 10 })],
  });

  it("고정 순서를 유지한다", () => {
    const s = snap(fresh(), fresh(), live(), codexLiveStatus(), fresh());
    expect(visibleUsageProviders(s, now)).toEqual(["claude", "codex", "antigravity"]);
  });

  it("낡거나 값 없는 provider만 빠진다", () => {
    const stale = { ...fresh(), fetchedAtMs: now - DEAD_THRESHOLD_MS - 1 };
    const s = snap(stale, fresh(), live(), codexLiveStatus(), null, antigravityLiveStatus());
    expect(visibleUsageProviders(s, now)).toEqual(["codex"]);
  });

  it("스냅샷이 아직 없으면 전부 남긴다(첫 폴링 전)", () => {
    expect(visibleUsageProviders(null, now)).toEqual(["claude", "codex", "antigravity"]);
  });
});

describe("describeAntigravityLiveStatus", () => {
  it("성공은 ok 단계", () => {
    expect(describeAntigravityLiveStatus(antigravityLiveStatus())!.level).toBe("ok");
  });

  it("agy 부재는 error 단계 + 전용 키", () => {
    const note = describeAntigravityLiveStatus(
      antigravityLiveStatus({ outcome: "cli_missing" }),
    )!;
    expect(note.level).toBe("error");
    expect(note.text).toEqual({ key: "usage.antigravityLive.cliMissing" });
  });

  it("명령 실패는 detail을 실어 나른다", () => {
    const note = describeAntigravityLiveStatus(
      antigravityLiveStatus({ outcome: "command_failed", detail: "not logged in" }),
    )!;
    expect(note.text).toEqual({
      key: "usage.antigravityLive.commandFailed",
      params: { detail: "not logged in" },
    });
  });

  it("진단이 없으면 아무것도 그리지 않는다", () => {
    expect(describeAntigravityLiveStatus(null)).toBeNull();
  });
});

describe("mergeUsageSnapshot codexLive", () => {
  it("진단은 병합하지 않고 항상 최신 응답을 쓴다", () => {
    const prev = snap(null, null, live(), codexLiveStatus({ outcome: "cli_missing" }));
    const next = snap(null, null, live(), codexLiveStatus({ outcome: "ok" }));
    expect(mergeUsageSnapshot(prev, next).codexLive.outcome).toBe("ok");
  });
});

// ── 카탈로그 렌더 ────────────────────────────────────────────────────
//
// 키만 검증하면 "번역이 실제로 있는지 / 예전 문구와 같은지"가 비어 버린다.
// 정본(ko)은 i18n 이행 전 문구를 글자 그대로 재현해야 하고, en은 최소한
// 말이 되고 보간 자리가 채워져야 한다.

describe("카탈로그 렌더(ko 정본)", () => {
  const NOW = 1_784_000_000_000;

  it("윈도 라벨", () => {
    expect(render(windowLabel(win({ kind: "session" })))).toBe("5시간");
    expect(render(windowLabel(win({ kind: "weekly" })))).toBe("주간");
    expect(render(windowLabel(win({ kind: "session_model", label: "Spark" })))).toBe("5시간 · Spark");
    expect(render(windowLabel(win({ kind: "session_model", label: null })))).toBe("5시간 (모델별)");
    expect(render(windowLabel(win({ kind: "weekly_model", label: "Fable" })))).toBe("주간 · Fable");
    expect(render(windowLabel(win({ kind: "weekly_model", label: null })))).toBe("주간 (모델별)");
    expect(render(windowLabel(win({ kind: "unknown", windowMinutes: 1440 })))).toBe("1440분 창");
    expect(render(windowLabel(win({ kind: "unknown", windowMinutes: null })))).toBe("기타");
  });

  it("카운트다운", () => {
    expect(render(formatCountdown(NOW - 1000, NOW)!)).toBe("리셋 대기 중");
    expect(render(formatCountdown(NOW + 45 * 60000, NOW)!)).toBe("45분 후 리셋");
    expect(render(formatCountdown(NOW + (3 * 60 + 12) * 60000, NOW)!)).toBe("3시간 12분 후 리셋");
    expect(render(formatCountdown(NOW + (2 * 24 * 60 + 5 * 60) * 60000, NOW)!)).toBe(
      "2일 5시간 후 리셋",
    );
  });

  it("경과·신선도", () => {
    expect(render(formatAgo(NOW - 30_000, NOW))).toBe("방금");
    expect(render(formatAgo(NOW - 14 * 60000, NOW))).toBe("14분 전");
    expect(render(formatAgo(NOW - (2 * 60 + 3) * 60000, NOW))).toBe("2시간 3분 전");
    expect(render(formatAgo(NOW - 3 * 24 * 60 * 60000, NOW))).toBe("3일 전");
    expect(render(formatFreshness(NOW - 30_000, NOW))).toBe("방금 기준");
    expect(render(formatFreshness(NOW - 14 * 60000, NOW))).toBe("14분 전 기준");
    expect(render(formatFreshness(NOW - (2 * 60 + 3) * 60000, NOW))).toBe("2시간 3분 전 기준");
    expect(render(formatFreshness(NOW - 3 * 24 * 60 * 60000, NOW))).toBe("3일 전 기준");
  });

  it("진단 조각 잇기", () => {
    const s = live({
      outcome: "ok",
      lastAttemptMs: NOW - 3 * 60000,
      lastSuccessMs: NOW - 3 * 60000,
    });
    expect(renderAttempts(formatLiveAttempts(s, NOW))).toBe("마지막 시도 3분 전 · 마지막 성공 3분 전");

    const failing = live({ outcome: "unauthorized", lastAttemptMs: NOW - 60000, lastSuccessMs: null });
    expect(renderAttempts(formatLiveAttempts(failing, NOW))).toBe(
      "마지막 시도 1분 전 · 성공 이력 없음",
    );

    const detoured = live({
      outcome: "network_error",
      lastAttemptMs: NOW - 60000,
      lastSuccessMs: NOW - 20 * 60000,
      via: "curl",
    });
    expect(renderAttempts(formatLiveAttempts(detoured, NOW))).toBe(
      "마지막 시도 1분 전 · 마지막 성공 20분 전 (curl 우회)",
    );
  });

  it("Claude 진단 문구가 예전 정보를 그대로 담는다", () => {
    expect(render(describeLiveStatus(live({ outcome: "ok" }))!.text)).toContain("직접 조회");
    const noCred = render(
      describeLiveStatus(live({ outcome: "no_credentials", tokenSource: null }))!.text,
    );
    expect(noCred).toContain("Keychain");

    const fileToken = render(
      describeLiveStatus(live({ outcome: "unauthorized", tokenSource: "file", detail: "HTTP 401" }))!
        .text,
    );
    expect(fileToken).toContain(".credentials.json");
    expect(fileToken).toContain("HTTP 401");

    const keychain = render(
      describeLiveStatus(live({ outcome: "unauthorized", tokenSource: "keychain_legacy" }))!.text,
    );
    expect(keychain).toContain("재로그인");
    expect(keychain).not.toContain(".credentials.json");

    // 실패 문구는 캐시가 /usage로만 갱신된다는 사실을 함께 말한다.
    for (const outcome of [
      "no_credentials",
      "unauthorized",
      "http_error",
      "network_error",
      "unexpected_response",
      "never_attempted",
    ] as const) {
      expect(render(describeLiveStatus(live({ outcome }))!.text)).toContain("/usage");
    }

    const detour = describeLiveStatus(live({ outcome: "ok", via: "curl" }))!;
    expect(render(detour.short)).toContain("curl 우회");
    expect(render(detour.text)).toContain("직접 거는 HTTPS가");
    expect(render(describeLiveStatus(live({ outcome: "ok", via: "claude_cli" }))!.short)).toContain(
      "claude CLI 우회",
    );
    expect(render(describeLiveStatus(live({ outcome: "ok", via: "direct" }))!.short)).toBe(
      "실시간 조회 정상",
    );
    expect(render(describeLiveStatus(live({ outcome: "ok", via: "direct" }))!.text)).not.toContain(
      "우회",
    );
  });

  it("Codex 진단 문구가 rollout 강등 안내를 함께 준다", () => {
    const missing = describeCodexLiveStatus(codexLiveStatus({ outcome: "cli_missing" }))!;
    expect(render(missing.text)).toContain("~/.codex/sessions");
    const rpc = describeCodexLiveStatus(
      codexLiveStatus({ outcome: "rpc_error", detail: "not logged in" }),
    )!;
    expect(render(rpc.text)).toContain("not logged in");
    expect(render(rpc.text)).toContain("codex login");
    expect(render(describeCodexLiveStatus(codexLiveStatus())!.short)).toBe("실시간 조회 정상");
    expect(
      render(describeCodexLiveStatus(codexLiveStatus({ outcome: "never_attempted" }))!.short),
    ).toBe("실시간 조회 대기 중");
  });

  it("detail이 없어도 보간 자리가 빈 괄호로 새지 않는다", () => {
    expect(render(describeLiveStatus(live({ outcome: "http_error" }))!.text)).toContain(
      "서버 오류(HTTP 오류)",
    );
    expect(render(describeLiveStatus(live({ outcome: "network_error" }))!.text)).toContain(
      "네트워크 오류",
    );
    expect(render(describeCodexLiveStatus(codexLiveStatus({ outcome: "cli_failed" }))!.text)).toContain(
      "종료했습니다(원인 불명)",
    );
  });
});

describe("카탈로그 렌더(en)", () => {
  const NOW = 1_784_000_000_000;

  it("영어로도 문구가 채워지고 키가 새지 않는다", async () => {
    await initI18nForTest("en");
    expect(render(windowLabel(win({ kind: "session" })))).toBe("5h");
    expect(render(formatCountdown(NOW + (3 * 60 + 12) * 60000, NOW)!)).toBe("Resets in 3h 12m");
    expect(render(formatFreshness(NOW - 14 * 60000, NOW))).toBe("as of 14m ago");
    const s = live({ outcome: "ok", lastAttemptMs: NOW - 3 * 60000, lastSuccessMs: NOW - 3 * 60000 });
    expect(renderAttempts(formatLiveAttempts(s, NOW))).toBe("last try 3m ago · last success 3m ago");
    // 캐시 안내는 `$t()` 중첩이라 언어를 따라 함께 바뀌어야 한다.
    const http = render(describeLiveStatus(live({ outcome: "http_error" }))!.text);
    expect(http).toContain("HTTP error");
    expect(http).toContain("/usage");
    expect(http).not.toContain("$t(");
  });

  afterAll(async () => {
    await initI18nForTest(SOURCE_LANGUAGE); // 정본 복구(파일 간 언어 상태 누수 방지)
  });
});
