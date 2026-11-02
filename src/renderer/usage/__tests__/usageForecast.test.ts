import { describe, expect, it } from "vitest";
import type { ProviderUsage, UsageSnapshot, UsageWindow } from "@shared/types";
import {
  addCost, assessUsage, costSince, emptyLedger, emptyModel, learnFromUsage, nowcastPercent, parseModel,
  predictTaskNeeds, providerOfTokens, tokensCost, USAGE_LIMIT_PERCENT, WAKE_GRACE_MS, windowKey, windowRatio,
  type Anchor, type ForecastModel, type TaskSample,
} from "../usageForecast";

const MIN = 60_000;
const HOUR = 60 * MIN;
const T0 = 1_800_000_000_000;

function win(kind: UsageWindow["kind"], usedPercent: number, resetsAtMs: number | null, label: string | null = null): UsageWindow {
  return { kind, label, usedPercent, resetsAtMs, windowMinutes: null, isActive: null };
}
function provider(fetchedAtMs: number, windows: UsageWindow[], name: ProviderUsage["provider"] = "claude"): ProviderUsage {
  return { provider: name, fetchedAtMs, planLabel: null, windows };
}
function snapshot(claude: ProviderUsage | null, codex: ProviderUsage | null = null): UsageSnapshot {
  const never = { outcome: "never_attempted", detail: null, lastAttemptMs: null, lastSuccessMs: null } as const;
  return { claude, codex, opencode: null, openrouter: null, claudeLive: { ...never, tokenSource: null, via: null }, codexLive: never, opencodeLive: never, openrouterLive: never };
}
function task(cost: number, patch: Partial<TaskSample> = {}): TaskSample {
  return { provider: "claude", cost, judgeCost: 0, durationMs: HOUR, at: T0, ...patch };
}
/** 비율(%p/달러)을 직접 심은 학습값. */
function modelWith(ratio: Record<string, number>, tasks: TaskSample[] = []): ForecastModel {
  const model = emptyModel();
  for (const [key, r] of Object.entries(ratio)) model.windows[key] = { sumPercent: r * 100, sumCost: 100, at: T0 };
  model.tasks = tasks;
  return model;
}
const SESSION = windowKey("claude", { kind: "session", label: null });
const WEEKLY = windowKey("claude", { kind: "weekly", label: null });

describe("비용 장부", () => {
  it("턴 토큰을 구독별로 나누고 요율을 모르는 모델도 비용으로 센다", () => {
    expect(providerOfTokens({ model: "claude-opus-5-5", input: 1 })).toBe("claude");
    expect(providerOfTokens({ byModel: [{ model: "gpt-5.5", input: 1 }] })).toBe("codex");
    expect(tokensCost({ model: "claude-opus-5-5", input: 1_000_000 })).toBeCloseTo(4);
    // 단가표에 없는 모델이어도 0으로 빠지지 않는다. 빠지면 비율 학습이 그 턴을 놓친다.
    expect(tokensCost({ model: "mystery-1", output: 1_000_000 })).toBeGreaterThan(0);
  });
  it("늦게 온 이벤트도 제자리에 넣고 시각 이후 비용만 더한다", () => {
    const ledger = emptyLedger(T0);
    addCost(ledger, "claude", "a", 1, T0 + 5 * MIN);
    addCost(ledger, "claude", "a", 2, T0 + 10 * MIN);
    addCost(ledger, "claude", "b", 4, T0 + 1 * MIN);
    addCost(ledger, "codex", "a", 8, T0 + 10 * MIN);
    expect(ledger.total.claude).toBe(7);
    expect(ledger.byAgent).toEqual({ a: 11, b: 4 });
    expect(costSince(ledger, "claude", T0 + 5 * MIN)).toBe(3);
    expect(costSince(ledger, "claude", T0)).toBe(7);
    expect(costSince(ledger, "claude", T0 + 11 * MIN)).toBe(0);
  });
});

describe("비율 학습", () => {
  it("정수로 반올림된 사용률도 앵커를 옮겨 가며 모으면 실제 비율에 수렴한다", () => {
    // 실제: 1달러 = 0.7%p. 보고값은 내림한 정수.
    const ledger = emptyLedger(T0);
    const model = emptyModel();
    const anchors: Record<string, Anchor> = {};
    const reset = T0 + 5 * HOUR;
    let spent = 0;
    for (let n = 0; n <= 40; n++) {
      const at = T0 + n * 5 * MIN;
      if (n > 0) { addCost(ledger, "claude", "a", 1.3, at - MIN); spent += 1.3; }
      learnFromUsage(model, anchors, ledger, "claude", provider(at, [win("session", Math.floor(spent * 0.7), reset)]), at);
    }
    expect(windowRatio(model, SESSION, T0 + 40 * 5 * MIN)).toBeCloseTo(0.7, 1);
  });
  it("창이 리셋되면 앵커만 새로 놓고, 장부를 열기 전 조회값은 쓰지 않는다", () => {
    const ledger = emptyLedger(T0);
    const model = emptyModel();
    const anchors: Record<string, Anchor> = {};
    expect(learnFromUsage(model, anchors, ledger, "claude", provider(T0 - MIN, [win("session", 50, T0 + HOUR)]), T0)).toBe(false);
    expect(anchors).toEqual({});
    learnFromUsage(model, anchors, ledger, "claude", provider(T0 + MIN, [win("session", 80, T0 + HOUR)]), T0 + MIN);
    addCost(ledger, "claude", "a", 5, T0 + 2 * HOUR);
    // 리셋 뒤 새 창: 80% → 3%. 줄어든 것을 음수 표본으로 넣지 않는다.
    expect(learnFromUsage(model, anchors, ledger, "claude", provider(T0 + 2 * HOUR + 2 * MIN, [win("session", 3, T0 + 6 * HOUR)]), T0 + 2 * HOUR)).toBe(false);
    expect(model.windows[SESSION]).toBeUndefined();
    expect(anchors[SESSION]).toMatchObject({ percent: 3, resetsAtMs: T0 + 6 * HOUR });
  });
  it("요금제 이름이 바뀌면 그 provider의 비율을 버리고 다시 배운다", () => {
    const ledger = emptyLedger(T0);
    const model = modelWith({ [SESSION]: 2, [WEEKLY]: 0.2, [windowKey("codex", { kind: "session", label: null })]: 1 });
    for (const stat of Object.values(model.windows)) stat.plan = "max_5x";
    const anchors: Record<string, Anchor> = {};
    // 이름을 모르는 조회값으로는 버리지 않는다.
    learnFromUsage(model, anchors, ledger, "claude", { ...provider(T0 + MIN, [win("session", 10, T0 + HOUR)]), planLabel: null }, T0 + MIN);
    expect(windowRatio(model, SESSION, T0 + MIN)).toBe(2);
    expect(learnFromUsage(model, anchors, ledger, "claude", { ...provider(T0 + 2 * MIN, [win("session", 10, T0 + HOUR)]), planLabel: "max_20x" }, T0 + 2 * MIN)).toBe(true);
    expect(windowRatio(model, SESSION, T0 + 2 * MIN)).toBeNull();
    expect(windowRatio(model, WEEKLY, T0 + 2 * MIN)).toBeNull();
    // 다른 provider는 그대로다.
    expect(windowRatio(model, windowKey("codex", { kind: "session", label: null }), T0 + 2 * MIN)).toBe(1);
  });
  it("이름이 그대로인데 한도가 바뀌면 큰 표본이 연달아 어긋날 때 새 비율로 빠르게 옮겨 간다", () => {
    // 오래 배운 비율 2%p/달러. 한도가 4배로 늘어 실제는 0.5%p/달러.
    const model = modelWith({ [SESSION]: 2 });
    model.windows[SESSION] = { sumPercent: 2000, sumCost: 1000, at: T0 };
    const ledger = emptyLedger(T0);
    const anchors: Record<string, Anchor> = {};
    const reset = T0 + 5 * HOUR;
    let spent = 0;
    const observe = (n: number) => {
      const at = T0 + n * 10 * MIN;
      if (n > 0) { addCost(ledger, "claude", "a", 12, at - MIN); spent += 12; }
      learnFromUsage(model, anchors, ledger, "claude", provider(at, [win("session", Math.floor(spent * 0.5), reset)]), at);
      return windowRatio(model, SESSION, at)!;
    };
    observe(0);
    // 한 번 어긋난 것은 앱 밖 사용일 수 있어 아직 크게 움직이지 않는다.
    expect(observe(1)).toBeGreaterThan(1.9);
    // 두 번째로 같은 방향이면 옛 값을 버리고 새 표본으로 시작한다.
    expect(observe(2)).toBeCloseTo(0.5, 0);
    for (let n = 3; n <= 6; n++) observe(n);
    expect(observe(7)).toBeCloseTo(0.5, 0);
  });
  it("앱이 거의 안 쓰는 사이 오른 사용률(앱 밖 사용)은 비율에 넣지 않는다", () => {
    const ledger = emptyLedger(T0);
    const model = emptyModel();
    const anchors: Record<string, Anchor> = {};
    learnFromUsage(model, anchors, ledger, "claude", provider(T0 + MIN, [win("session", 10, T0 + HOUR)]), T0 + MIN);
    expect(learnFromUsage(model, anchors, ledger, "claude", provider(T0 + 20 * MIN, [win("session", 30, T0 + HOUR)]), T0 + 20 * MIN)).toBe(false);
    expect(model.windows[SESSION]).toBeUndefined();
    // 기준점은 옮겨서, 다음 표본에 앱 밖 사용분이 섞이지 않는다.
    addCost(ledger, "claude", "a", 5, T0 + 30 * MIN);
    learnFromUsage(model, anchors, ledger, "claude", provider(T0 + 40 * MIN, [win("session", 40, T0 + HOUR)]), T0 + 40 * MIN);
    expect(model.windows[SESSION]).toMatchObject({ sumPercent: 10, sumCost: 5 });
  });
  it("누적 비용이 모자라면 비율을 믿지 않는다", () => {
    const model = emptyModel();
    model.windows[SESSION] = { sumPercent: 3, sumCost: 0.5, at: T0 };
    expect(windowRatio(model, SESSION, T0)).toBeNull();
    model.windows[SESSION] = { sumPercent: 30, sumCost: 10, at: T0 };
    expect(windowRatio(model, SESSION, T0)).toBe(3);
    // 오래된 학습값은 감쇠해 다시 모자라진다.
    expect(windowRatio(model, SESSION, T0 + 30 * 24 * HOUR)).toBeNull();
  });
});

describe("예측", () => {
  it("다음 태스크는 최근 태스크 비용의 상위 분위에 판단자 비용을 더한다", () => {
    const model = modelWith({}, [task(2, { judgeCost: 0.1 }), task(4, { judgeCost: 0.1 }), task(6, { judgeCost: 0.1 }), task(100, { provider: "codex" })]);
    const [claude] = predictTaskNeeds(model, "claude");
    expect(claude.provider).toBe("claude");
    expect(claude.cost).toBeCloseTo(5 + 0.1);
    // Codex 작업자면 Claude에는 판단자 몫만 남는다.
    const codex = predictTaskNeeds(model, "codex");
    expect(codex.find((n) => n.provider === "codex")?.cost).toBeNull();
    expect(codex.find((n) => n.provider === "claude")?.cost).toBeCloseTo(0.1);
    expect(predictTaskNeeds(emptyModel(), "claude")[0].cost).toBeNull();
  });
  it("보고 뒤 쓴 비용을 더해 지금 사용률을 추정하고, 리셋이 지났으면 리셋 뒤 쓴 양만 센다", () => {
    const ledger = emptyLedger(T0);
    addCost(ledger, "claude", "a", 10, T0 + 30 * MIN);
    const w = win("session", 40, T0 + HOUR);
    expect(nowcastPercent(w, provider(T0 + 10 * MIN, [w]), 2, ledger, "claude", T0 + 40 * MIN)).toBe(60);
    expect(nowcastPercent(w, provider(T0 + 10 * MIN, [w]), null, ledger, "claude", T0 + 40 * MIN)).toBe(40);
    const past = win("session", 99, T0 + 20 * MIN);
    expect(nowcastPercent(past, provider(T0 + 10 * MIN, [past]), 2, ledger, "claude", T0 + 40 * MIN)).toBe(20);
  });
});

describe("보내도 되는가", () => {
  const ledger = () => emptyLedger(T0 - HOUR);
  const tasks = [task(10), task(10), task(10)];
  it("예측 사용량이 한도를 넘기면 가장 늦게 풀리는 창의 리셋까지 기다린다", () => {
    const model = modelWith({ [SESSION]: 2, [WEEKLY]: 0.2 }, tasks);
    const now = T0;
    // 5시간 70% + 20%p = 90%: 통과. 80% + 20%p = 100%: 막힘.
    expect(assessUsage(predictTaskNeeds(model, "claude"), snapshot(provider(now, [win("session", 70, now + 4 * HOUR)])), model, ledger(), now)).toBeNull();
    const block = assessUsage(predictTaskNeeds(model, "claude"),
      snapshot(provider(now, [win("session", 80, now + 4 * HOUR), win("weekly", 94, now + 3 * 24 * HOUR)])), model, ledger(), now);
    // 주간도 94% + 2%p로 넘친다. 주간 리셋까지 기다려야 둘 다 풀린다.
    expect(block).toMatchObject({ kind: "weekly", until: now + 3 * 24 * HOUR + WAKE_GRACE_MS, limit: USAGE_LIMIT_PERCENT });
    expect(block!.addPercent).toBeCloseTo(2);
  });
  it("태스크 도중 리셋되는 창에는 리셋 전 몫만 더한다", () => {
    const model = modelWith({ [SESSION]: 2 }, tasks);
    const now = T0;
    // 85% + 20%p는 넘지만, 15분 뒤 리셋되고 태스크는 1시간이라 이 창에는 5%p만 든다. 45분 뒤면 15%p라 넘친다.
    expect(assessUsage(predictTaskNeeds(model, "claude"), snapshot(provider(now, [win("session", 85, now + 15 * MIN)])), model, ledger(), now)).toBeNull();
    expect(assessUsage(predictTaskNeeds(model, "claude"), snapshot(provider(now, [win("session", 85, now + 45 * MIN)])), model, ledger(), now)).not.toBeNull();
  });
  it("학습값이 없으면 이미 한도에 닿았는지만 보고, 거의 빈 창은 큰 태스크라도 막지 않는다", () => {
    const now = T0;
    const cold = emptyModel();
    expect(assessUsage(predictTaskNeeds(cold, "claude"), snapshot(provider(now, [win("session", 90, now + HOUR)])), cold, ledger(), now)).toBeNull();
    expect(assessUsage(predictTaskNeeds(cold, "claude"), snapshot(provider(now, [win("session", 96, now + HOUR)])), cold, ledger(), now))
      .toMatchObject({ kind: "session", addPercent: null });
    // 리셋해도 나아지지 않는다(이미 빈 창). 계속 기다리면 영영 못 보낸다.
    const huge = modelWith({ [SESSION]: 2 }, [task(80), task(80)]);
    expect(assessUsage(predictTaskNeeds(huge, "claude"), snapshot(provider(now, [win("session", 2, now + 5 * HOUR)])), huge, ledger(), now)).toBeNull();
    // 사용량 정보가 없는 provider는 막지 않는다.
    expect(assessUsage(predictTaskNeeds(huge, "claude"), snapshot(null), huge, ledger(), now)).toBeNull();
  });
  it("쓰는지 모르는 모델별 창은 막지 않는다", () => {
    const now = T0;
    const model = modelWith({}, tasks);
    const usage = snapshot(provider(now, [win("weekly_model", 100, now + HOUR, "Opus")]));
    expect(assessUsage(predictTaskNeeds(model, "claude"), usage, model, ledger(), now)).toBeNull();
    const learned = modelWith({ [windowKey("claude", { kind: "weekly_model", label: "Opus" })]: 1 }, tasks);
    expect(assessUsage(predictTaskNeeds(learned, "claude"), usage, learned, ledger(), now)).toMatchObject({ kind: "weekly_model", label: "Opus" });
  });
  it("리셋 시각을 모르는 창이 막으면 잠시 뒤 다시 본다", () => {
    const now = T0;
    const block = assessUsage(predictTaskNeeds(emptyModel(), "codex"), snapshot(null, provider(now, [win("session", 100, null)], "codex")), emptyModel(), ledger(), now);
    expect(block).toMatchObject({ provider: "codex", resetsAtMs: null });
    expect(block!.until).toBeGreaterThan(now);
  });
});

describe("저장 형식", () => {
  it("깨진 항목은 버리고 나머지를 읽는다", () => {
    const model = parseModel({
      windows: { ok: { sumPercent: 1, sumCost: 2, at: 3 }, bad: { sumPercent: "x" }, neg: { sumPercent: -1, sumCost: 1, at: 1 } },
      tasks: [{ provider: "claude", cost: 1, judgeCost: 0, durationMs: 5, at: 1 }, { cost: "1" }, null, { provider: "kilo", cost: 1, judgeCost: 0, durationMs: 1, at: 1 }],
      followups: "nope",
    });
    expect(Object.keys(model.windows)).toEqual(["ok"]);
    expect(model.tasks.map((t) => t.provider)).toEqual(["claude", null]);
    expect(model.followups).toEqual([]);
    expect(parseModel(null)).toEqual(emptyModel());
  });
});
