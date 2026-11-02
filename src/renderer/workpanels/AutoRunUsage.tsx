// 자동 진행 영역의 사용량 대기 조각: 켜고 끄기, 다음 태스크 예측 한 줄, 대기 이유 문장.
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import type { AutoRunUsageWait, UsageWindowKind, WorkItem } from "@shared/types";
import { renderText, type Translate } from "@renderer/shared/textKey";
import { useAppStore } from "../store/appStore";
import { formatCountdown, windowLabel } from "../usage/usageView";
import { USAGE_LIMIT_PERCENT, type ForecastProvider } from "../usage/usageForecast";
import { taskOutlook } from "../usage/usageForecastStore";
import { useWorkItemStore } from "./workItemStore";
import { nextTaskWorker, setAutoRunUsageGuard } from "./autoRun";

/** 예측을 시작하는 데 필요한 끝난 태스크 수(usageForecast.ts의 MIN_SAMPLES와 같다). */
const MIN_TASKS = 2;

function windowName(tActivity: Translate, kind: UsageWindowKind, label: string | null): string {
  return renderText(windowLabel({ kind, label, usedPercent: 0, resetsAtMs: null, windowMinutes: null, isActive: null }), tActivity);
}
const percent = (n: number) => Math.max(0, Math.round(n));

/** 사용량 대기 이유와 언제 이어 가는지. */
export function usageWaitText(t: Translate, tActivity: Translate, wait: AutoRunUsageWait, now: number): string {
  const base = {
    provider: t(`autoRun.usage.provider.${wait.provider}`),
    window: windowName(tActivity, wait.kind, wait.label),
    now: percent(wait.nowPercent),
    limit: wait.limit,
    stage: t(`autoRun.usage.stage.${wait.stage}`),
  };
  const why = wait.addPercent === null || wait.nowPercent >= wait.limit
    ? t("autoRun.usage.waitExhausted", base)
    : t("autoRun.usage.waitPredicted", { ...base, add: Math.max(1, Math.round(wait.addPercent)) });
  const countdown = formatCountdown(wait.resetsAtMs, now);
  const until = new Date(wait.until).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
  const when = countdown
    ? t("autoRun.usage.waitUntil", { reset: renderText(countdown, tActivity) })
    : t("autoRun.usage.waitRetry", { time: until });
  return `${why} ${when}`;
}

function useNow(intervalMs: number): number {
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    const id = window.setInterval(() => setNow(Date.now()), intervalMs);
    return () => window.clearInterval(id);
  }, [intervalMs]);
  return now;
}

/** 다음 태스크가 창마다 얼마나 쓸지. 끝난 태스크가 모자라면 무엇만 보는지 알린다. */
function Outlook({ worker }: { worker: ForecastProvider | null }) {
  const { t } = useTranslation("workpanels");
  const { t: tActivity } = useTranslation("activity");
  // 조회값이 바뀌면 다시 계산한다.
  useAppStore((s) => s.usage);
  const now = useNow(60_000);
  const outlook = taskOutlook(worker, now);
  if (!outlook.length) return null;
  const predicted = outlook.filter((o) => o.addPercent !== null);
  if (!predicted.length) return <p className="work-muted">{t("autoRun.usage.learning", { min: MIN_TASKS, limit: USAGE_LIMIT_PERCENT })}</p>;
  const list = predicted.map((o) => t("autoRun.usage.outlookItem", {
    provider: t(`autoRun.usage.provider.${o.provider}`),
    window: windowName(tActivity, o.kind, o.label),
    now: percent(o.nowPercent),
    after: percent(o.nowPercent + (o.addPercent ?? 0)),
  })).join(", ");
  return <p className="work-muted">{t("autoRun.usage.outlook", { list })}</p>;
}

/** 사용량 대기 켜기·끄기와 예측. 실행 전이면 `guard`·`onGuard`로 시작 옵션을 받는다. */
export function AutoRunUsage({ root, guard, onGuard }: { root: WorkItem; guard: boolean; onGuard: (on: boolean) => void }) {
  const { t } = useTranslation("workpanels");
  const items = useWorkItemStore((s) => s.items);
  const agents = useAppStore((s) => s.agents);
  const run = root.autoRun;
  const on = run && run.state !== "finished" ? run.usageGuard !== false : guard;
  const change = (next: boolean) => { if (run && run.state !== "finished") setAutoRunUsageGuard(root.id, next); else onGuard(next); };
  const waiting = run?.state === "paused" && run.pauseReason === "usage-wait";
  return <div className="work-auto-run-usage">
    <label title={t("autoRun.usage.guardHint", { limit: USAGE_LIMIT_PERCENT })}>
      <input type="checkbox" checked={on} onChange={(e) => change(e.target.checked)} />
      {t("autoRun.usage.guard")}
    </label>
    {on && !waiting && run?.state !== "finished" && <Outlook worker={nextTaskWorker(root, items, agents)} />}
  </div>;
}

/** 대기 이유 문단. 남은 시간이 흐르게 다시 그린다. */
export function UsageWaitDetail({ wait }: { wait: AutoRunUsageWait }) {
  const { t } = useTranslation("workpanels");
  const { t: tActivity } = useTranslation("activity");
  const now = useNow(30_000);
  return <p className="work-auto-run-text">{usageWaitText(t, tActivity, wait, now)}</p>;
}
