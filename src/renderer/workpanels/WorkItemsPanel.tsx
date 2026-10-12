import { useEffect, useState } from "react";
import { createPortal } from "react-dom";
import { useTranslation } from "react-i18next";
import type { WorkItem, WorkStatus } from "@shared/types";
import { useAppStore } from "../store/appStore";
import { useWorkItemStore, WORK_STATUS_ORDER } from "./workItemStore";
import { WorkPanelFrame } from "./WorkPanelFrame";
import { WorkAgentFields } from "./WorkAgentFields";
import { WorkCardMonitor } from "./WorkCardMonitor";
import { useWorkMonitorStore } from "./workMonitor";
import { useSessionSummaryStore } from "./sessionSummaryStore";
import { requestWorkSend, decideWorkSend, useWorkSendStore, workSendDisabledReason } from "./workSendStore";
import { sendErrorText } from "./sendErrorText";
import { AgentFace } from "./AgentFace";

function WorkEditor({ item, busy }: { item: WorkItem; busy: boolean }) {
  const { t } = useTranslation("workpanels");
  const edit = useWorkItemStore((s) => s.edit);
  const agents = useAppStore((s) => s.agents);
  const [title, setTitle] = useState(item.title);
  return <fieldset disabled={busy} className="work-editor" onBlur={() => void useWorkItemStore.getState().flush()}>
    <label>{t("title")}<input value={title} maxLength={500} onChange={(e) => setTitle(e.target.value)} onBlur={() => { if (title.trim()) edit(item.id, { title: title.trim() }); else setTitle(item.title); }} /></label>
    <label>{t("body")}<textarea value={item.body} rows={4} onChange={(e) => edit(item.id, { body: e.target.value })} /></label>
    <label>{t("assignee")}<select value={item.assigneeAgentId ?? ""} onChange={(e) => edit(item.id, { assigneeAgentId: e.target.value || undefined })}>
      <option value="">{t("unassigned")}</option>
      {item.assigneeAgentId && !agents[item.assigneeAgentId] && <option value={item.assigneeAgentId}>{t("missingAgent")}</option>}
      {Object.values(agents).map((agent) => <option key={agent.id} value={agent.id}>{agent.name}</option>)}
    </select></label>
    <WorkAgentFields value={item.agent} inherited={item.assigneeAgentId ? agents[item.assigneeAgentId]?.defaultAgent : undefined} onChange={(agent) => edit(item.id, { agent })} />
    <label>{t("statusLabel")}<select value={item.status} onChange={(e) => edit(item.id, { status: e.target.value as WorkStatus })}>
      {WORK_STATUS_ORDER.map((status) => <option key={status} value={status}>{t(`status.${status}`)}</option>)}
    </select></label>
  </fieldset>;
}
export function WorkItemsPanel() {
  const { t } = useTranslation("workpanels");
  const { items, loaded, error, selectedId, add, remove, select, reorder, load } = useWorkItemStore();
  const agents = useAppStore((s) => s.agents);
  const openTerminal = useAppStore((s) => s.openTerminal);
  const busy = useWorkSendStore((s) => s.busy);
  const confirmation = useWorkSendStore((s) => s.confirmation);
  const summaries = useSessionSummaryStore((s) => s.summaries);
  const monitored = useWorkMonitorStore((s) => s.sessions);
  const [draft, setDraft] = useState("");
  const [deleting, setDeleting] = useState<WorkItem | null>(null);
  useEffect(() => { if (!loaded && !error) void load(); }, [loaded, error, load]);
  useEffect(() => () => { decideWorkSend(false); void useWorkItemStore.getState().flush(); }, []);
  return <><WorkPanelFrame side="left" title={t("workTitle")}>
    <form onSubmit={(e) => { e.preventDefault(); if (add(draft)) setDraft(""); }}>
      <input aria-label={t("newWork")} placeholder={t("newWork")} value={draft} disabled={!loaded} maxLength={500} onChange={(e) => setDraft(e.target.value)} />
    </form>
    {error && <div role="alert">{t(error === "load" ? "loadError" : "saveError")} <button className="pixel-btn" onClick={() => void (error === "load" ? load() : useWorkItemStore.getState().flush())}>{t("retry")}</button></div>}
    {!items.length && loaded && <p className="work-muted">{t("emptyWork")}</p>}
    {WORK_STATUS_ORDER.map((status) => <details className="work-group" key={status} open={status !== "done"}>
      <summary>{t(`status.${status}`)} ({items.filter((i) => i.status === status).length})</summary>
      {items.filter((i) => i.status === status).sort((a, b) => a.order - b.order).map((item) => {
        const sessionId = item.sessionIds[item.sessionIds.length - 1];
        const terminalAgent = (sessionId ? summaries[sessionId]?.agentId ?? monitored[sessionId]?.agentId : undefined) ?? item.assigneeAgentId;
        const disabledReason = workSendDisabledReason(item.assigneeAgentId ? agents[item.assigneeAgentId] : undefined);
        const sending = !!busy[item.id];
        const agentBusy = item.assigneeAgentId && Object.values(busy).includes(item.assigneeAgentId);
        return <article className={`work-card${sessionId && monitored[sessionId]?.phase === "waiting" ? " waiting" : ""}`} key={item.id} id={`work-item-${item.id}`}
        onKeyDown={(e) => { if (e.key === "Escape") { select(null); e.stopPropagation(); } }}
        onDragOver={(e) => e.preventDefault()}
        onDrop={(e) => {
          e.preventDefault();
          const rect = e.currentTarget.getBoundingClientRect();
          reorder(e.dataTransfer.getData("application/x-agent-office-work"), item.id, e.clientY > rect.top + rect.height / 2);
        }}>
        <button className="work-card-heading" draggable onDragStart={(e) => e.dataTransfer.setData("application/x-agent-office-work", item.id)} aria-expanded={selectedId === item.id} onClick={() => { select(selectedId === item.id ? null : item.id); if (sessionId && terminalAgent && agents[terminalAgent]) openTerminal(terminalAgent); }}>
          {item.assigneeAgentId && <AgentFace agentId={item.assigneeAgentId} />}
          <span><strong>{item.title}</strong><small>{item.assigneeAgentId ? agents[item.assigneeAgentId]?.name ?? t("missingAgent") : t("unassigned")}</small></span>
        </button>
        {sessionId && <WorkCardMonitor sessionId={sessionId} assignee={item.assigneeAgentId} status={item.status} />}
        {selectedId === item.id && <>
          <WorkEditor item={item} busy={sending} />
          {item.lastError && <p role="alert">{sendErrorText(item.lastError)}</p>}
          <div className="work-card-actions">
            <button className="pixel-btn" disabled={!!disabledReason || !!agentBusy} title={disabledReason ? sendErrorText(disabledReason) : agentBusy ? t("sending") : undefined} onClick={() => void requestWorkSend(item.id)}>{t(sending ? "sending" : "send")}</button>
            {terminalAgent && agents[terminalAgent] && <button className="pixel-btn" onClick={() => openTerminal(terminalAgent)}>{t("terminal")}</button>}
            <button className="pixel-btn" onClick={() => setDeleting(item)}>{t("delete")}</button>
          </div>
        </>}
      </article>; })}
    </details>)}
  </WorkPanelFrame>
    {confirmation && createPortal(<div className="modal-backdrop" onKeyDown={(e) => { if (e.key === "Escape") { e.stopPropagation(); decideWorkSend(false); } }}>
      <section className="pixel-panel work-delete-dialog" role="alertdialog" aria-modal="true" aria-labelledby="work-send-title" aria-describedby="work-send-body">
        <h2 id="work-send-title">{t("restartSendTitle")}</h2>
        <p id="work-send-body">{t("restartSendBody", { name: confirmation.agentName, title: confirmation.title })}</p>
        <p>{t("restartSendWarning")}</p>
        {confirmation.target.hasUncommitted && <p>{t("restartSendUncommitted")}</p>}
        <div className="dialog-actions">
          <button className="pixel-btn" autoFocus onClick={() => decideWorkSend(false)}>{t("cancel")}</button>
          <button className="pixel-btn" onClick={() => decideWorkSend(true)}>{t("restartSendConfirm")}</button>
        </div>
      </section>
    </div>, document.body)}
    {deleting && createPortal(<div className="modal-backdrop" onKeyDown={(e) => { if (e.key === "Escape") { e.stopPropagation(); setDeleting(null); } }}>
      <section className="pixel-panel work-delete-dialog" role="alertdialog" aria-modal="true" aria-labelledby="work-delete-title" aria-describedby="work-delete-body">
        <h2 id="work-delete-title">{t("deleteTitle")}</h2>
        <p id="work-delete-body">{t("deleteBody", { title: deleting.title })}</p>
        <div className="dialog-actions">
          <button className="pixel-btn" autoFocus onClick={() => setDeleting(null)}>{t("cancel")}</button>
          <button className="pixel-btn" onClick={() => { remove(deleting.id); setDeleting(null); }}>{t("delete")}</button>
        </div>
      </section>
    </div>, document.body)}
  </>;
}
