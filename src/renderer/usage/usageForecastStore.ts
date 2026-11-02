// src/renderer/usage/usageForecastStore.ts
//
// 사용량 예측(usageForecast.ts)의 상태: 이번 실행의 비용 장부, 비율 학습 앵커,
// 재시작 너머로 남기는 학습값(localStorage). 자동 진행(workpanels/autoRun.ts)이
// 보내기 전에 assess*로 묻고, 끝난 태스크·턴의 비용을 표본으로 넘긴다.
//
// 학습값은 이 기기의 편의값이라 localStorage에 둔다. 읽기·쓰기에 실패하면 빈
// 학습값으로 시작할 뿐이다(그동안은 이미 한도에 닿았는지만 본다).
import type { TurnUsageEvent, UsageSnapshot, WorkAgent } from "@shared/types";
import { tauriApi } from "../ipc/tauriApi";
import { useAppStore } from "../store/appStore";
import {
  addCost, assessUsage, emptyLedger, emptyModel, FORECAST_PROVIDERS, judgeNeeds, learnFromUsage, parseModel,
  predictFollowupNeeds, predictTaskNeeds, providerOfTokens, pushSample, tokensCost, usageOutlook,
  type Anchor, type CostLedger, type ForecastModel, type ForecastProvider, type TaskSample, type TurnSample,
  type UsageBlock, type UsageOutlook,
} from "./usageForecast";

const STORAGE_KEY = "agent-office.usageForecast";

let ledger: CostLedger = emptyLedger(Date.now());
let anchors: Record<string, Anchor> = {};
let model: ForecastModel = load();

function load(): ForecastModel {
  try {
    const raw = localStorage.getItem(STORAGE_KEY);
    return raw ? parseModel(JSON.parse(raw)) : emptyModel();
  } catch {
    return emptyModel();
  }
}
function save(): void {
  try {
    localStorage.setItem(STORAGE_KEY, JSON.stringify(model));
  } catch {
    // 저장 실패는 무시한다 — 다음 실행이 빈 학습값으로 시작할 뿐이다.
  }
}

/** 작업자 CLI의 구독. 사용량 창이 있는 것만 센다. */
export function workerProvider(agent: Pick<WorkAgent, "cli"> | undefined): ForecastProvider | null {
  return agent?.cli === "claude" || agent?.cli === "codex" ? agent.cli : null;
}

/** 훅이 준 턴 토큰을 장부에 더한다. 태스크 대화처럼 캐릭터가 없는 세션도 같은 구독을 쓰므로 센다. */
export function recordTurnUsage(e: TurnUsageEvent): void {
  addCost(ledger, providerOfTokens(e.tokens), e.agentId, tokensCost(e.tokens), e.at);
}

/** 판단자(claude -p) 호출 비용. 훅을 타지 않으므로 따로 넣는다. */
export function recordJudgeCost(costUsd: number | undefined, at = Date.now()): void {
  if (costUsd) addCost(ledger, "claude", null, costUsd, at);
}

/** 캐릭터 하나의 이번 실행 누적 비용. 표본은 두 표시의 차이다. 장부가 바뀌었으면(재시작) 비교하지 않는다. */
export interface CostMark { epoch: number; cost: number; at: number }
export function costMark(agentId: string, at = Date.now()): CostMark {
  return { epoch: ledger.startedAt, cost: ledger.byAgent[agentId] ?? 0, at };
}
export function costSinceMark(agentId: string, mark: CostMark | undefined): number | null {
  if (!mark || mark.epoch !== ledger.startedAt) return null;
  return Math.max(0, (ledger.byAgent[agentId] ?? 0) - mark.cost);
}

export function recordTaskSample(sample: TaskSample): void {
  model = { ...model, tasks: pushSample(model.tasks, sample) };
  save();
}
export function recordFollowupSample(sample: TurnSample): void {
  model = { ...model, followups: pushSample(model.followups, sample) };
  save();
}

function snapshot(): UsageSnapshot | null { return useAppStore.getState().usage; }

/** 새 태스크를 보내도 되는가. 막히면 막는 창. */
export function assessTask(worker: ForecastProvider | null, now = Date.now()): UsageBlock | null {
  return assessUsage(predictTaskNeeds(model, worker), snapshot(), model, ledger, now);
}
/** 같은 세션에 후속 지시를 넣어도 되는가. */
export function assessFollowup(worker: ForecastProvider | null, now = Date.now()): UsageBlock | null {
  return assessUsage(predictFollowupNeeds(model, worker), snapshot(), model, ledger, now);
}
/** 판단자를 불러도 되는가. */
export function assessJudge(now = Date.now()): UsageBlock | null {
  return assessUsage(judgeNeeds(), snapshot(), model, ledger, now);
}
/** 다음 태스크의 창별 예측(화면 표시용). */
export function taskOutlook(worker: ForecastProvider | null, now = Date.now()): UsageOutlook[] {
  return usageOutlook(predictTaskNeeds(model, worker), snapshot(), model, ledger, now);
}
/** 최근 태스크 표본 수(화면 표시용). */
export function taskSampleCount(worker: ForecastProvider | null): number {
  return model.tasks.filter((s) => s.provider === worker).length;
}

/** 기다림을 풀기 전에 사용량을 새로 받아 둔다. 실패하면 있는 값으로 판단한다. */
export async function refreshUsage(): Promise<void> {
  try {
    const next = await tauriApi.loadUsageSnapshot();
    if (next) useAppStore.getState().applyUsageSnapshot(next);
  } catch (error) {
    console.warn("usage-forecast: refresh failed", error);
  }
}

function learn(usage: UsageSnapshot | null): void {
  if (!usage) return;
  const now = Date.now();
  let learned = false;
  for (const provider of FORECAST_PROVIDERS) {
    const data = usage[provider];
    if (data && learnFromUsage(model, anchors, ledger, provider, data, now)) learned = true;
  }
  if (learned) { model = { ...model }; save(); }
}

/** 사용량 조회값이 바뀔 때마다 비율을 배운다. */
export function installUsageForecast(): () => void {
  learn(useAppStore.getState().usage);
  return useAppStore.subscribe((state, previous) => { if (state.usage !== previous.usage) learn(state.usage); });
}

/** 지금 학습값(읽기 전용으로 쓴다). */
export function usageForecastModel(): ForecastModel { return model; }

/** 테스트용: 장부·앵커·학습값을 비운다. */
export function resetUsageForecast(now = Date.now(), next: ForecastModel = emptyModel()): void {
  ledger = emptyLedger(now);
  anchors = {};
  model = next;
}
