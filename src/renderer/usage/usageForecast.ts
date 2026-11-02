// src/renderer/usage/usageForecast.ts
//
// 자동 진행이 다음 일을 보내기 전에 "이걸 보내면 구독 한도 창이 버티는가"를
// 예측하는 순수 모듈. 상태(장부·학습값)는 usageForecastStore.ts가 쥐고, 여기는
// 계산만 한다.
//
// 한도 사용률(%)만 보면 두 가지가 틀린다.
// 1. 표시값이 늦다. Claude 실시간 조회는 15분마다라 그 사이 쓴 양이 빠진다.
// 2. 다음 태스크가 얼마나 쓸지 모른다. 80%에서 5%짜리 태스크는 보내도 되고,
//    60%에서 50%짜리 태스크는 보내면 안 된다.
//
// 그래서 실제 사용량을 토큰으로 센다. 훅이 주는 턴별 토큰을 API 요율 기준 비용
// (USD 상당, 이하 "비용")으로 바꿔 provider·캐릭터별로 누적하고(장부), 한도 창마다
// "비용 1달러가 몇 %p인가"(비율)를 사용률 변화와 비용 변화의 짝에서 배운다.
// 앵커를 보고된 값으로 옮겨 가며 차이를 더하므로 정수로 반올림된 사용률도 오래
// 모으면 오차가 쌓이지 않는다. 앱이 거의 안 쓰는 동안 오른 사용률(앱 밖 사용)은 비율에
// 넣지 않는다. 앱과 앱 밖이 섞인 구간은 비율을 조금 키워 예측이 일찍 기다리는 쪽으로 틀린다.
// 요금제가 바뀌면 같은 비용의 %p가 달라진다. 조회값의 요금제 이름(planLabel)이 바뀌면
// 그 provider의 비율을 버리고 다시 배운다. 이름이 그대로인데 한도가 바뀐 경우는 큰 표본이
// 같은 방향으로 두 번 연달아 비율과 2배 넘게 어긋나면 옛 학습값을 버린다.
// 태스크 표본은 비용(달러) 단위라 요금제와 무관하게 그대로 쓴다.
//
// 예측: 지금 사용률 ≈ 보고값 + 비율 × 보고 뒤 쓴 비용. 다음 태스크 ≈ 최근 태스크
// 비용의 상위 분위(75%) × 비율. 창이 태스크 도중 리셋되면 리셋 전까지 걸리는 몫만
// 이 창에 더한다(최근 태스크 소요 시간의 중앙값으로 나눈다). 학습값이 없으면 이미
// 한도에 닿았는지만 본다.
import type { ProviderUsage, SessionEventTokens, SessionModelTokens, UsageSnapshot, UsageWindow, UsageWindowKind } from "@shared/types";
import { estimateCostBreakdown } from "../analytics/pricing";

export type ForecastProvider = "claude" | "codex";
export const FORECAST_PROVIDERS: readonly ForecastProvider[] = ["claude", "codex"];

/** 이 사용률(%)을 넘길 것 같으면 보내지 않고 리셋을 기다린다. */
export const USAGE_LIMIT_PERCENT = 95;
/** 창이 이만큼(%) 이하로 비어 있으면 예측이 커도 기다리지 않는다. 리셋해도 나아지지 않는다. */
const FRESH_PERCENT = 5;
/** 비율 학습값의 반감기. 요금제·모델이 바뀌면 며칠 안에 새 값을 따라간다. */
const RATIO_HALF_LIFE_MS = 3 * 24 * 60 * 60 * 1000;
/** 비율을 믿기 위한 (감쇠 뒤) 누적 비용. */
const MIN_RATIO_COST = 1;
/** 표본 하나로 묶는 최소 변화. 이보다 작으면 앵커를 그대로 두고 계속 모은다. */
const MIN_SAMPLE_COST = 0.2;
const MIN_SAMPLE_PERCENT = 2;
/** 어긋남을 판단할 만큼 큰 표본(%p). 정수 반올림 오차가 20% 아래다. */
const DRIFT_MIN_PERCENT = 5;
/** 표본 비율이 학습값과 이 배수 넘게 다르면 어긋난 것이다. */
const DRIFT_FACTOR = 2;
/** 같은 방향으로 이만큼 연달아 어긋나면 옛 학습값을 버린다. 한 번은 앱 밖 사용일 수 있다. */
const DRIFT_STREAK = 2;
/** 같은 창으로 보는 리셋 시각 차이. Codex는 남은 초로 리셋 시각을 만들어 조금씩 흔들린다. */
const RESET_TOLERANCE_MS = 10 * 60 * 1000;
const BUCKET_MS = 60 * 1000;
const BUCKET_KEEP_MS = 8 * 24 * 60 * 60 * 1000;
/** 예측에 쓰는 최근 표본 수와 분위. */
export const MAX_SAMPLES = 40;
const RECENT_SAMPLES = 20;
const MIN_SAMPLES = 2;
const QUANTILE = 0.75;
/** 리셋 시각 뒤 이만큼 더 기다린 다음 다시 본다. 조회값이 리셋을 반영할 시간을 준다. */
export const WAKE_GRACE_MS = 2 * 60 * 1000;
/** 리셋 시각을 모르는 창이 막고 있으면 이 간격으로 다시 본다. */
const UNKNOWN_RESET_RETRY_MS = 15 * 60 * 1000;
/** 요율을 모르는 모델의 대표 요율($/Mtok). 비율 학습이 그 턴을 빼먹지 않게 한다. */
const FALLBACK_RATE = { input: 3, output: 15, cacheRead: 0.3, cacheWrite: 3.75 };

// ── 장부 ──────────────────────────────────────────────────────────────

/** provider·캐릭터별 누적 비용과 분 단위 시계열. 앱 실행 동안만 산다. */
export interface CostLedger {
  /** 장부를 연 시각. 이보다 앞의 사용량은 모른다. */
  startedAt: number;
  total: Record<ForecastProvider, number>;
  byAgent: Record<string, number>;
  /** 분 시작 시각 → 그 분의 비용. 오름차순. */
  buckets: Record<ForecastProvider, Array<[number, number]>>;
}

export function emptyLedger(now: number): CostLedger {
  return { startedAt: now, total: { claude: 0, codex: 0 }, byAgent: {}, buckets: { claude: [], codex: [] } };
}

const CLAUDE_MODEL = /claude|opus|sonnet|haiku|fable|mythos/i;
/** 턴 토큰을 낸 구독. 토큰은 claude·codex 훅만 주므로 Claude 계열이 아니면 Codex다. */
export function providerOfTokens(tokens: SessionEventTokens): ForecastProvider {
  const model = tokens.model ?? tokens.byModel?.find((m) => m.model)?.model;
  return model && CLAUDE_MODEL.test(model) ? "claude" : "codex";
}

function fallbackCost(tokens: SessionModelTokens): number {
  return ((tokens.input ?? 0) * FALLBACK_RATE.input + (tokens.output ?? 0) * FALLBACK_RATE.output
    + (tokens.cacheRead ?? 0) * FALLBACK_RATE.cacheRead + (tokens.cacheWrite ?? 0) * FALLBACK_RATE.cacheWrite) / 1_000_000;
}

/** 턴 하나의 비용. 요율을 모르는 모델이 섞이면 대표 요율로 센다. */
export function tokensCost(tokens: SessionEventTokens): number {
  const known = estimateCostBreakdown(tokens);
  if (known && !known.hasUnknown) return known.costUsd;
  return Math.max(known?.costUsd ?? 0, fallbackCost(tokens));
}

/** 비용 하나를 더한다(장부를 고친다). */
export function addCost(ledger: CostLedger, provider: ForecastProvider, agentId: string | null, cost: number, at: number): void {
  if (!(cost > 0) || !Number.isFinite(cost)) return;
  ledger.total[provider] += cost;
  if (agentId) ledger.byAgent[agentId] = (ledger.byAgent[agentId] ?? 0) + cost;
  const buckets = ledger.buckets[provider];
  const minute = Math.floor(at / BUCKET_MS) * BUCKET_MS;
  const last = buckets[buckets.length - 1];
  if (last && last[0] === minute) last[1] += cost;
  else if (!last || last[0] < minute) buckets.push([minute, cost]);
  else {
    // 늦게 온 이벤트. 제자리에 넣는다.
    const index = buckets.findIndex(([m]) => m >= minute);
    if (buckets[index][0] === minute) buckets[index][1] += cost;
    else buckets.splice(index, 0, [minute, cost]);
  }
  const keep = minute - BUCKET_KEEP_MS;
  while (buckets.length && buckets[0][0] < keep) buckets.shift();
}

/** `since` 이후(그 분 포함) 쓴 비용. 경계의 분을 통째로 넣어 조금 크게 센다. */
export function costSince(ledger: CostLedger, provider: ForecastProvider, since: number): number {
  const from = Math.floor(Math.max(since, ledger.startedAt) / BUCKET_MS) * BUCKET_MS;
  let sum = 0;
  const buckets = ledger.buckets[provider];
  for (let n = buckets.length - 1; n >= 0 && buckets[n][0] >= from; n--) sum += buckets[n][1];
  return sum;
}

// ── 학습값 ────────────────────────────────────────────────────────────

/** 한도 창 하나의 비율 학습값(감쇠 누적). */
export interface WindowStat {
  sumPercent: number;
  sumCost: number;
  at: number;
  /** 배울 때의 요금제 이름. 바뀌면 이 학습값을 버린다. */
  plan?: string | null;
  /** 연달아 어긋난 표본 수. 양수는 비율이 커지는 쪽, 음수는 작아지는 쪽. */
  drift?: number;
}
/** 끝난 태스크 하나. `provider`는 작업자의 구독(구독 사용량이 없는 CLI면 null). */
export interface TaskSample { provider: ForecastProvider | null; cost: number; judgeCost: number; durationMs: number; at: number }
/** 후속 지시 한 턴. */
export interface TurnSample { provider: ForecastProvider | null; cost: number; durationMs: number; at: number }
/** 앱 재시작 너머로 남기는 학습값. */
export interface ForecastModel {
  windows: Record<string, WindowStat>;
  tasks: TaskSample[];
  followups: TurnSample[];
}
/** 비율 학습의 기준점. 장부 기준이라 앱 실행 동안만 쓴다. */
export interface Anchor { resetsAtMs: number; percent: number; fetchedAtMs: number; cost: number }

export function emptyModel(): ForecastModel { return { windows: {}, tasks: [], followups: [] }; }

export function windowKey(provider: ForecastProvider, w: Pick<UsageWindow, "kind" | "label">): string {
  return `${provider}:${w.kind}:${w.label ?? ""}`;
}

function decay(stat: WindowStat, now: number): number {
  return Math.pow(0.5, Math.max(0, now - stat.at) / RATIO_HALF_LIFE_MS);
}

/** 비용 1달러가 이 창에서 몇 %p인가. 아직 모르면 null. */
export function windowRatio(model: ForecastModel, key: string, now: number): number | null {
  const stat = model.windows[key];
  if (!stat || stat.sumCost * decay(stat, now) < MIN_RATIO_COST) return null;
  return stat.sumPercent / stat.sumCost;
}

function addRatioSample(model: ForecastModel, key: string, percent: number, cost: number, plan: string | null, now: number): void {
  const stat = model.windows[key];
  let keep = stat ? decay(stat, now) : 0;
  // 큰 표본이 학습값과 같은 방향으로 연달아 크게 어긋나면 한도가 바뀐 것으로 보고 옛 값을 버린다.
  const current = windowRatio(model, key, now);
  let drift = 0;
  if (current !== null && percent >= DRIFT_MIN_PERCENT) {
    const sample = percent / cost;
    const direction = sample > current * DRIFT_FACTOR ? 1 : sample < current / DRIFT_FACTOR ? -1 : 0;
    const previous = stat?.drift ?? 0;
    drift = direction === 0 ? 0 : Math.sign(previous) === direction ? previous + direction : direction;
    if (Math.abs(drift) >= DRIFT_STREAK) { keep = 0; drift = 0; }
  } else {
    drift = stat?.drift ?? 0;
  }
  model.windows[key] = {
    sumPercent: (stat?.sumPercent ?? 0) * keep + percent,
    sumCost: (stat?.sumCost ?? 0) * keep + cost,
    at: now,
    plan,
    ...(drift ? { drift } : {}),
  };
}

/** 요금제 이름이 바뀐 provider의 비율을 버린다. 이름을 모르는 조회값은 판단하지 않는다. */
function forgetOtherPlan(model: ForecastModel, provider: ForecastProvider, plan: string | null): boolean {
  if (!plan) return false;
  let dropped = false;
  for (const [key, stat] of Object.entries(model.windows)) {
    if (key.startsWith(`${provider}:`) && stat.plan && stat.plan !== plan) { delete model.windows[key]; dropped = true; }
  }
  return dropped;
}

/**
 * 새 사용량 조회값으로 비율을 배운다(학습값·앵커를 고친다). 고친 것이 있으면 true.
 * 같은 창(리셋 시각이 같고 사용률이 줄지 않음) 안에서 앵커 뒤의 사용률 변화와 비용
 * 변화를 짝지어 더한다. 리셋됐거나 처음 보는 창은 앵커만 새로 놓는다.
 */
export function learnFromUsage(
  model: ForecastModel,
  anchors: Record<string, Anchor>,
  ledger: CostLedger,
  provider: ForecastProvider,
  usage: ProviderUsage,
  now: number,
): boolean {
  const plan = usage.planLabel?.trim() || null;
  const forgot = forgetOtherPlan(model, provider, plan);
  // 장부를 열기 전에 조회한 값은 그때까지의 비용을 모른다.
  if (usage.fetchedAtMs < ledger.startedAt) return forgot;
  const costAt = ledger.total[provider] - costSince(ledger, provider, usage.fetchedAtMs + BUCKET_MS);
  let learned = forgot;
  for (const w of usage.windows) {
    if (w.resetsAtMs === null) continue;
    const key = windowKey(provider, w);
    const anchor = anchors[key];
    const fresh: Anchor = { resetsAtMs: w.resetsAtMs, percent: w.usedPercent, fetchedAtMs: usage.fetchedAtMs, cost: costAt };
    if (!anchor) { anchors[key] = fresh; continue; }
    if (usage.fetchedAtMs <= anchor.fetchedAtMs) continue;
    const sameWindow = Math.abs(w.resetsAtMs - anchor.resetsAtMs) <= RESET_TOLERANCE_MS && w.usedPercent >= anchor.percent - 0.5;
    if (!sameWindow) { anchors[key] = fresh; continue; }
    const percent = Math.max(0, w.usedPercent - anchor.percent);
    const cost = Math.max(0, costAt - anchor.cost);
    if (cost < MIN_SAMPLE_COST && percent < MIN_SAMPLE_PERCENT) continue;
    // 앱이 거의 안 쓰는 사이 오른 사용률은 앱 밖 사용이다. 비율에 넣지 않고 기준점만 옮긴다.
    if (cost < MIN_SAMPLE_COST) { anchors[key] = fresh; continue; }
    addRatioSample(model, key, percent, cost, plan, now);
    anchors[key] = fresh;
    learned = true;
  }
  return learned;
}

/** 표본 목록 끝에 붙이고 상한을 지킨다. */
export function pushSample<T>(list: T[], sample: T): T[] {
  return [...list, sample].slice(-MAX_SAMPLES);
}

// ── 예측 ──────────────────────────────────────────────────────────────

/** 다음 일 하나가 provider 하나에 쓸 양. 표본이 모자라면 cost가 null이다. */
export interface UsageNeed { provider: ForecastProvider; cost: number | null; durationMs: number | null; samples: number }

function quantile(values: number[], q: number): number {
  const sorted = [...values].sort((a, b) => a - b);
  const pos = (sorted.length - 1) * q;
  const low = Math.floor(pos);
  const high = Math.ceil(pos);
  return sorted[low] + (sorted[high] - sorted[low]) * (pos - low);
}

/**
 * 태스크 하나를 새로 보낼 때 쓸 양. 작업자 구독에는 최근 태스크 비용을, Claude에는
 * 판단자 비용을 더한다(판단자는 늘 claude -p다).
 */
export function predictTaskNeeds(model: ForecastModel, worker: ForecastProvider | null): UsageNeed[] {
  const recent = model.tasks.slice(-RECENT_SAMPLES);
  const mine = worker ? recent.filter((s) => s.provider === worker) : [];
  const judged = recent.filter((s) => s.judgeCost > 0);
  const needs = new Map<ForecastProvider, UsageNeed>();
  if (worker) {
    const enough = mine.length >= MIN_SAMPLES;
    needs.set(worker, {
      provider: worker,
      cost: enough ? quantile(mine.map((s) => s.cost), QUANTILE) : null,
      durationMs: enough ? quantile(mine.map((s) => s.durationMs), 0.5) : null,
      samples: mine.length,
    });
  }
  const judge = judged.length >= MIN_SAMPLES ? quantile(judged.map((s) => s.judgeCost), QUANTILE) : null;
  const claude = needs.get("claude");
  if (claude) {
    // 작업자 비용을 모르면 판단자 몫만으로 예측하지 않는다. 작업자가 훨씬 크다.
    if (claude.cost !== null && judge !== null) claude.cost += judge;
  } else {
    const durations = recent.map((s) => s.durationMs);
    needs.set("claude", { provider: "claude", cost: judge, durationMs: durations.length >= MIN_SAMPLES ? quantile(durations, 0.5) : null, samples: judged.length });
  }
  return [...needs.values()];
}

/** 같은 세션에 후속 지시 한 턴을 넣을 때 쓸 양. 판단자는 이미 한도를 넘었는지만 본다. */
export function predictFollowupNeeds(model: ForecastModel, worker: ForecastProvider | null): UsageNeed[] {
  const needs: UsageNeed[] = [];
  if (worker) {
    const mine = model.followups.slice(-RECENT_SAMPLES).filter((s) => s.provider === worker);
    const enough = mine.length >= MIN_SAMPLES;
    needs.push({
      provider: worker,
      cost: enough ? quantile(mine.map((s) => s.cost), QUANTILE) : null,
      durationMs: enough ? quantile(mine.map((s) => s.durationMs), 0.5) : null,
      samples: mine.length,
    });
  }
  if (worker !== "claude") needs.push({ provider: "claude", cost: null, durationMs: null, samples: 0 });
  return needs;
}

/** 판단자 한 번. 이미 한도를 넘었는지만 본다. */
export function judgeNeeds(): UsageNeed[] {
  return [{ provider: "claude", cost: null, durationMs: null, samples: 0 }];
}

/** 막는 창 하나. `addPercent`가 null이면 예측 없이 이미 한도에 닿은 것이다. */
export interface UsageBlock {
  provider: ForecastProvider;
  kind: UsageWindowKind;
  label: string | null;
  /** 지금 사용률 추정(보고값 + 보고 뒤 쓴 양). */
  nowPercent: number;
  /** 다음 일이 이 창에 더할 것으로 보는 양(%p). */
  addPercent: number | null;
  limit: number;
  /** 다시 볼 시각(리셋 + 여유). */
  until: number;
  /** 리셋 시각을 알면 그 시각. */
  resetsAtMs: number | null;
}

/** 창 하나의 지금 사용률 추정. 리셋 시각이 지났으면 리셋 뒤 쓴 양만 센다. */
export function nowcastPercent(w: UsageWindow, usage: ProviderUsage, ratio: number | null, ledger: CostLedger, provider: ForecastProvider, now: number): number {
  if (w.resetsAtMs !== null && w.resetsAtMs <= now) return ratio ? ratio * costSince(ledger, provider, w.resetsAtMs) : 0;
  if (!ratio || usage.fetchedAtMs < ledger.startedAt) return w.usedPercent;
  return w.usedPercent + ratio * costSince(ledger, provider, usage.fetchedAtMs + BUCKET_MS);
}

/**
 * 다음 일을 지금 보내도 되는가. 막는 창이 있으면 가장 늦게 풀리는 것을 돌려준다
 * (그때 다시 본다). 사용량 정보가 없는 provider는 막지 않는다.
 */
export function assessUsage(
  needs: UsageNeed[],
  snapshot: UsageSnapshot | null,
  model: ForecastModel,
  ledger: CostLedger,
  now: number,
  limit = USAGE_LIMIT_PERCENT,
): UsageBlock | null {
  let worst: UsageBlock | null = null;
  for (const need of needs) {
    const usage = snapshot?.[need.provider];
    if (!usage) continue;
    for (const w of usage.windows) {
      const ratio = windowRatio(model, windowKey(need.provider, w), now);
      // 모델별 창은 그 모델을 쓰는지 배운 뒤에만 본다. 안 쓰는 모델의 창이 막지 않게 한다.
      if ((w.kind === "session_model" || w.kind === "weekly_model") && !ratio) continue;
      const nowPercent = nowcastPercent(w, usage, ratio, ledger, need.provider, now);
      const resetsAtMs = w.resetsAtMs !== null && w.resetsAtMs > now ? w.resetsAtMs : null;
      const addPercent = ratio !== null && need.cost !== null ? ratio * need.cost : null;
      // 태스크 도중 리셋되면 리셋 전까지의 몫만 이 창에 든다.
      const share = addPercent === null ? 0
        : resetsAtMs !== null && need.durationMs ? addPercent * Math.min(1, Math.max(0, (resetsAtMs - now) / need.durationMs))
          : addPercent;
      const exhausted = nowPercent >= limit;
      const short = addPercent !== null && nowPercent > FRESH_PERCENT && nowPercent + share > limit;
      if (!exhausted && !short) continue;
      const until = resetsAtMs !== null ? resetsAtMs + WAKE_GRACE_MS : now + UNKNOWN_RESET_RETRY_MS;
      if (!worst || until > worst.until) {
        worst = { provider: need.provider, kind: w.kind, label: w.label, nowPercent, addPercent: addPercent === null ? null : share, limit, until, resetsAtMs };
      }
    }
  }
  return worst;
}

/** 화면에 보일 예측 한 줄(창마다). 막지 않는 창도 보인다. */
export interface UsageOutlook { provider: ForecastProvider; kind: UsageWindowKind; label: string | null; nowPercent: number; addPercent: number | null; resetsAtMs: number | null }

export function usageOutlook(needs: UsageNeed[], snapshot: UsageSnapshot | null, model: ForecastModel, ledger: CostLedger, now: number): UsageOutlook[] {
  const out: UsageOutlook[] = [];
  for (const need of needs) {
    const usage = snapshot?.[need.provider];
    if (!usage) continue;
    for (const w of usage.windows) {
      const ratio = windowRatio(model, windowKey(need.provider, w), now);
      if ((w.kind === "session_model" || w.kind === "weekly_model") && !ratio) continue;
      out.push({
        provider: need.provider, kind: w.kind, label: w.label,
        nowPercent: nowcastPercent(w, usage, ratio, ledger, need.provider, now),
        addPercent: ratio !== null && need.cost !== null ? ratio * need.cost : null,
        resetsAtMs: w.resetsAtMs !== null && w.resetsAtMs > now ? w.resetsAtMs : null,
      });
    }
  }
  return out;
}

// ── 영속 형식 ─────────────────────────────────────────────────────────

const finite = (v: unknown): v is number => typeof v === "number" && Number.isFinite(v);
const providerOrNull = (v: unknown): ForecastProvider | null => (v === "claude" || v === "codex" ? v : null);

/** 저장된 JSON → 학습값. 깨진 항목은 버린다. */
export function parseModel(raw: unknown): ForecastModel {
  const model = emptyModel();
  if (!raw || typeof raw !== "object") return model;
  const value = raw as Record<string, unknown>;
  if (value.windows && typeof value.windows === "object") {
    for (const [key, stat] of Object.entries(value.windows as Record<string, unknown>)) {
      const s = stat as Partial<WindowStat> | null;
      if (s && finite(s.sumPercent) && finite(s.sumCost) && finite(s.at) && s.sumPercent >= 0 && s.sumCost >= 0) {
        model.windows[key] = {
          sumPercent: s.sumPercent, sumCost: s.sumCost, at: s.at,
          ...(typeof s.plan === "string" ? { plan: s.plan } : {}),
          ...(finite(s.drift) && s.drift !== 0 ? { drift: Math.trunc(s.drift) } : {}),
        };
      }
    }
  }
  if (Array.isArray(value.tasks)) {
    model.tasks = value.tasks.flatMap((s: Partial<TaskSample> | null) =>
      s && finite(s.cost) && finite(s.judgeCost) && finite(s.durationMs) && finite(s.at)
        ? [{ provider: providerOrNull(s.provider), cost: s.cost, judgeCost: s.judgeCost, durationMs: s.durationMs, at: s.at }] : []).slice(-MAX_SAMPLES);
  }
  if (Array.isArray(value.followups)) {
    model.followups = value.followups.flatMap((s: Partial<TurnSample> | null) =>
      s && finite(s.cost) && finite(s.durationMs) && finite(s.at)
        ? [{ provider: providerOrNull(s.provider), cost: s.cost, durationMs: s.durationMs, at: s.at }] : []).slice(-MAX_SAMPLES);
  }
  return model;
}
