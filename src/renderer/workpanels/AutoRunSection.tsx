import { useTranslation } from "react-i18next";
import type { AutoRun, AutoRunLogEntry, WorkItem } from "@shared/types";
import { useAppStore } from "../store/appStore";
import { formatUsd } from "../analytics/pricing";
import { useWorkItemStore } from "./workItemStore";
import { workItemTitle } from "./workItemDraft";
import { sendErrorText } from "./sendErrorText";
import { endAutoRun, MAX_FOLLOW_UPS, pauseAutoRun, planItems, resumeAutoRun, startAutoRun, startBlockedReason } from "./autoRun";

/** 시작 차단·일시정지 상세 코드를 화면 문장으로. 자동 진행 코드가 아니면 보내기 오류 문구를 쓴다. */
export function codeText(t: (key: string, options?: Record<string, unknown>) => string, code: string): string {
  if (code.startsWith("auto-run-") && !code.startsWith("auto-run-judge")) return t(`autoRun.blocked.${code}`);
  if (code.startsWith("work-send-") || code.startsWith("automation-")) return sendErrorText(code);
  return code;
}

function pauseText(t: (key: string, options?: Record<string, unknown>) => string, run: AutoRun): { reason: string; detail?: string } {
  const reason = t(`autoRun.reasons.${run.pauseReason ?? "user"}`, { max: MAX_FOLLOW_UPS });
  if (!run.pauseDetail) return { reason };
  // 판단자 메시지는 그대로, 오류 코드는 문장으로 보여 준다.
  const message = run.pauseReason === "judge-stop" || run.pauseReason === "followup-limit" || run.pauseReason === "task-busy" || run.pauseReason === "task-changed" || run.pauseReason === "session-ended";
  return { reason, detail: message ? run.pauseDetail : codeText(t, run.pauseDetail) };
}

function LogLine({ entry, titles }: { entry: AutoRunLogEntry; titles: Map<string, string> }) {
  const { t } = useTranslation("workpanels");
  const time = new Date(entry.at).toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" });
  const title = titles.get(entry.taskId ?? "") ?? t("missingWork");
  const head = t(`autoRun.logKinds.${entry.kind}`, { title });
  // 멈춤 기록의 text는 사유 코드다.
  const text = entry.kind === "pause" ? t(`autoRun.reasons.${entry.text ?? "user"}`, { max: MAX_FOLLOW_UPS }) : entry.text?.trim();
  return <li><time>{time}</time> <strong>{head}</strong>{text && <span className="work-auto-run-text">{text}</span>}</li>;
}

/** 루트 업무 상세 팝업의 자동 진행 영역(docs/auto-run-design.md). 선행 작업이 있거나 실행 기록이 있을 때만 보인다. */
export function AutoRunSection({ root }: { root: WorkItem }) {
  const { t } = useTranslation("workpanels");
  const items = useWorkItemStore((s) => s.items);
  const agents = useAppStore((s) => s.agents);
  const observerEnabled = useAppStore((s) => s.appSettings.observerEnabled);
  const costEnabled = useAppStore((s) => s.appSettings.sessionCostEnabled);
  const run = root.autoRun;
  const plan = planItems(root, items);
  if (!run && !plan.length) return null;
  const titles = new Map(items.map((item) => [item.id, workItemTitle(item)]));
  const done = plan.filter((item) => item.status === "done").length;
  const runner = root.assigneeAgentId ? agents[root.assigneeAgentId]?.name : undefined;
  const blocked = run && run.state !== "finished" ? undefined : startBlockedReason(root, items, agents, observerEnabled);
  const current = run?.currentId ? items.find((item) => item.id === run.currentId) : undefined;
  const paused = run?.state === "paused" ? pauseText(t, run) : undefined;
  return <section className="work-auto-run" aria-labelledby={`auto-run-${root.id}`}>
    <h3 id={`auto-run-${root.id}`}>{t("autoRun.title")}</h3>
    {!run && <p className="work-muted">{t("autoRun.idle", { count: plan.length - done, name: runner ?? t("unassigned") })}</p>}
    {run?.state === "running" && <p>{current
      ? <>{t("autoRun.running", { title: workItemTitle(current) })} <small>{t("autoRun.followUps", { count: run.followUps, max: MAX_FOLLOW_UPS })}</small></>
      : t("autoRun.preparing")}</p>}
    {paused && <div role="status" className="work-auto-run-paused"><p>{paused.reason}</p>{paused.detail && <p className="work-auto-run-text">{paused.detail}</p>}</div>}
    {run?.state === "finished" && <p>{t("autoRun.finished")}</p>}
    <p className="work-muted">{t("autoRun.progress", { done, total: plan.length })}
      {run && costEnabled && run.costUsd > 0 && <> · {t("autoRun.cost", { cost: formatUsd(run.costUsd) })}</>}</p>
    <div className="work-dialog-actions">
      {run && run.state !== "running" && <button className="pixel-btn" onClick={() => endAutoRun(root.id)}>{t(run.state === "finished" ? "autoRun.clear" : "autoRun.end")}</button>}
      <span className="work-dialog-spacer" />
      {run?.state === "running" && <button className="pixel-btn" onClick={() => pauseAutoRun(root.id)}>{t("autoRun.pause")}</button>}
      {run?.state === "paused" && <button className="pixel-btn primary" onClick={() => resumeAutoRun(root.id)}>{t("autoRun.resume")}</button>}
      {(!run || run.state === "finished") && <button className="pixel-btn primary" disabled={!!blocked} aria-describedby={blocked ? `auto-run-hint-${root.id}` : undefined}
        onClick={() => { if (run) endAutoRun(root.id); startAutoRun(root.id); }}>{t("autoRun.start")}</button>}
    </div>
    {blocked && <p id={`auto-run-hint-${root.id}`} className="work-send-hint">{codeText(t, blocked)}</p>}
    {run && run.state !== "running" && <p className="work-muted">{t("autoRun.endHint")}</p>}
    {run && run.log.length > 0 && <details className="work-auto-run-log">
      <summary>{t("autoRun.log", { count: run.log.length })}</summary>
      <ol>{run.log.map((entry, n) => <LogLine key={`${entry.at}-${n}`} entry={entry} titles={titles} />)}</ol>
    </details>}
  </section>;
}
