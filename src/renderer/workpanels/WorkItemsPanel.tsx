import { useEffect, useState } from "react";
import { createPortal } from "react-dom";
import { useTranslation } from "react-i18next";
import type { WorkItem, WorkStatus } from "@shared/types";
import { useAppStore } from "../store/appStore";
import { useWorkItemStore, WORK_STATUS_ORDER } from "./workItemStore";
import { WorkPanelFrame } from "./WorkPanelFrame";
import { AgentFace } from "./AgentFace";

function WorkEditor({ item }: { item: WorkItem }) {
  const { t } = useTranslation("workpanels");
  const edit = useWorkItemStore((s) => s.edit);
  const agents = useAppStore((s) => s.agents);
  const [title, setTitle] = useState(item.title);
  return <div className="work-editor" onBlur={() => void useWorkItemStore.getState().flush()}>
    <label>{t("title")}<input value={title} maxLength={500} onChange={(e) => setTitle(e.target.value)} onBlur={() => { if (title.trim()) edit(item.id, { title: title.trim() }); else setTitle(item.title); }} /></label>
    <label>{t("body")}<textarea value={item.body} rows={4} onChange={(e) => edit(item.id, { body: e.target.value })} /></label>
    <label>{t("assignee")}<select value={item.assigneeAgentId ?? ""} onChange={(e) => edit(item.id, { assigneeAgentId: e.target.value || undefined })}>
      <option value="">{t("unassigned")}</option>
      {item.assigneeAgentId && !agents[item.assigneeAgentId] && <option value={item.assigneeAgentId}>{t("missingAgent")}</option>}
      {Object.values(agents).map((agent) => <option key={agent.id} value={agent.id}>{agent.name}</option>)}
    </select></label>
    <label>{t("statusLabel")}<select value={item.status} onChange={(e) => edit(item.id, { status: e.target.value as WorkStatus })}>
      {WORK_STATUS_ORDER.map((status) => <option key={status} value={status}>{t(`status.${status}`)}</option>)}
    </select></label>
  </div>;
}
export function WorkItemsPanel() {
  const { t } = useTranslation("workpanels");
  const { items, loaded, error, selectedId, add, remove, select, reorder, load } = useWorkItemStore();
  const agents = useAppStore((s) => s.agents);
  const openTerminal = useAppStore((s) => s.openTerminal);
  const [draft, setDraft] = useState("");
  const [deleting, setDeleting] = useState<WorkItem | null>(null);
  useEffect(() => { if (!loaded && !error) void load(); }, [loaded, error, load]);
  useEffect(() => () => { void useWorkItemStore.getState().flush(); }, []);
  return <WorkPanelFrame side="left" title={t("workTitle")}>
    <form onSubmit={(e) => { e.preventDefault(); if (add(draft)) setDraft(""); }}>
      <input aria-label={t("newWork")} placeholder={t("newWork")} value={draft} disabled={!loaded} maxLength={500} onChange={(e) => setDraft(e.target.value)} />
    </form>
    {error && <div role="alert">{t(error === "load" ? "loadError" : "saveError")} <button className="pixel-btn" onClick={() => void (error === "load" ? load() : useWorkItemStore.getState().flush())}>{t("retry")}</button></div>}
    {!items.length && loaded && <p className="work-muted">{t("emptyWork")}</p>}
    {WORK_STATUS_ORDER.map((status) => <details className="work-group" key={status} open={status !== "done"}>
      <summary>{t(`status.${status}`)} ({items.filter((i) => i.status === status).length})</summary>
      {items.filter((i) => i.status === status).sort((a, b) => a.order - b.order).map((item) => <article className="work-card" key={item.id} id={`work-item-${item.id}`}
        onKeyDown={(e) => { if (e.key === "Escape") { select(null); e.stopPropagation(); } }}
        onDragOver={(e) => e.preventDefault()}
        onDrop={(e) => {
          e.preventDefault();
          const rect = e.currentTarget.getBoundingClientRect();
          reorder(e.dataTransfer.getData("application/x-agent-office-work"), item.id, e.clientY > rect.top + rect.height / 2);
        }}>
        <button className="work-card-heading" draggable onDragStart={(e) => e.dataTransfer.setData("application/x-agent-office-work", item.id)} aria-expanded={selectedId === item.id} onClick={() => { select(selectedId === item.id ? null : item.id); if (item.sessionIds.length && item.assigneeAgentId && agents[item.assigneeAgentId]) openTerminal(item.assigneeAgentId); }}>
          {item.assigneeAgentId && <AgentFace agentId={item.assigneeAgentId} />}
          <span><strong>{item.title}</strong><small>{item.assigneeAgentId ? agents[item.assigneeAgentId]?.name ?? t("missingAgent") : t("unassigned")}</small></span>
        </button>
        {selectedId === item.id && <>
          <WorkEditor item={item} />
          {item.lastError && <p role="alert">{item.lastError}</p>}
          <div className="work-card-actions">
            <button className="pixel-btn" disabled title={t("sendPending")}>{t("send")}</button>
            {item.assigneeAgentId && agents[item.assigneeAgentId] && <button className="pixel-btn" onClick={() => openTerminal(item.assigneeAgentId!)}>{t("terminal")}</button>}
            <button className="pixel-btn" onClick={() => setDeleting(item)}>{t("delete")}</button>
          </div>
        </>}
      </article>)}
    </details>)}
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
  </WorkPanelFrame>;
}
