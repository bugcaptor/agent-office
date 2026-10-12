import type { WorkStatus } from "@shared/types";
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { useAppStore } from "../store/appStore";
import { formatDuration } from "../timeline/format";
import { formatUsd } from "../analytics/pricing";
import { useSessionUsageSeed } from "../usage/useSessionUsageSeed";
import { mergeTotals } from "../usage/sessionCost";
import { useSessionSummaryStore } from "./sessionSummaryStore";
import { useWorkMonitorStore } from "./workMonitor";

export function WorkCardMonitor({ sessionId, assignee, status }: { sessionId: string; assignee?: string; status: WorkStatus }) {
  useSessionUsageSeed();
  const { t } = useTranslation("workpanels");
  const observed = useWorkMonitorStore((s) => s.sessions[sessionId]);
  const summary = useSessionSummaryStore((s) => s.summaries[sessionId]);
  const agentId = summary?.agentId ?? observed?.agentId ?? assignee ?? "";
  const usage = useAppStore((s) => s.sessionUsage[agentId]);
  const seed = useAppStore((s) => s.sessionUsageSeed?.bySession[sessionId]);
  const timing = useAppStore((s) => s.timeTracking[agentId]);
  const costEnabled = useAppStore((s) => s.appSettings.sessionCostEnabled);
  const liveIds = useSessionSummaryStore((s) => s.liveIds);
  const live = liveIds.includes(sessionId);
  // 사람이 완료로 옮긴 업무는 세션 신호보다 완료를 우선 보이고 경과 시간도 멈춘다.
  const done = status === "done";
  const [now, setNow] = useState(Date.now);
  useEffect(() => { if (!live || done) return; const tick = setInterval(() => setNow(Date.now()), 1000); return () => clearInterval(tick); }, [live, done]);
  const current = usage?.sessionId === sessionId;
  const totals = current ? seed ? mergeTotals(seed, usage.totals) : usage.totals : seed;
  const phase = !live ? "ended" : observed?.phase ?? (current && timing?.phase !== "idle" ? timing?.phase : status === "review" ? "review" : "working");
  const label = done ? "status.done" : phase === "waiting" ? "monitorWaiting" : phase === "review" ? "monitorReview" : phase === "ended" ? "monitorEnded" : "monitorWorking";
  return <div className={`work-card-monitor${!done && phase === "waiting" ? " waiting" : ""}`}>
    <span>{t(label)}</span>
    {summary && ((live && !done) || summary.endedAt !== undefined) && <small>{t("elapsed", { time: formatDuration(Math.max(0, (summary.endedAt ?? now) - summary.startedAt)) })}</small>}
    {totals && <small title={t("costHint")}>{t("stats", { turns: totals.turns, tokens: totals.input + totals.output + totals.cacheRead + totals.cacheWrite, cost: costEnabled ? totals.costUnknownTurns > 0 && totals.costUsd === 0 ? "—" : `${totals.costUnknownTurns > 0 ? "*" : ""}${formatUsd(totals.costUsd)}` : "—" })}</small>}
  </div>;
}
