import { useEffect, useRef, useState, type ReactNode } from "react";
import { createPortal } from "react-dom";
import { useTranslation } from "react-i18next";
import type { WorkItem } from "@shared/types";
import { useAppStore } from "../store/appStore";
import { useWorkItemStore, WORK_STATUS_ORDER } from "./workItemStore";
import { WorkPanelFrame } from "./WorkPanelFrame";
import { WorkEditor } from "./WorkEditor";
import { EMPTY_WORK_DRAFT, hasWorkContent, hasWorkDraftChanges, workItemTitle, type WorkItemDraft } from "./workItemDraft";
import { WorkCardMonitor } from "./WorkCardMonitor";
import { useWorkMonitorStore } from "./workMonitor";
import { useSessionSummaryStore } from "./sessionSummaryStore";
import { requestWorkSend, decideWorkSend, useWorkSendStore, workSendDisabledReason } from "./workSendStore";
import { sendErrorText } from "./sendErrorText";
import { AgentFace } from "./AgentFace";
import { dependentCount, pendingPrerequisites } from "./workDependencies";

/** 상세·편집 팝업. 헤더 ×, Esc, 배경 클릭은 초안을 유지한 채 닫는다. */
function WorkDialog({ title, onClose, children }: { title: string; onClose(): void; children: ReactNode }) {
  const { t } = useTranslation("workpanels");
  const ref = useRef<HTMLElement>(null);
  // 헤더의 ×가 아니라 첫 입력란에 포커스를 둔다.
  useEffect(() => { ref.current?.querySelector<HTMLElement>("input, textarea, select")?.focus(); }, []);
  return createPortal(<div className="modal-backdrop" onMouseDown={(e) => { if (e.target === e.currentTarget) onClose(); }}
    onKeyDown={(e) => { if (e.key === "Escape") { e.stopPropagation(); onClose(); } }}>
    <section ref={ref} className="pixel-panel work-detail-dialog" role="dialog" aria-modal="true" aria-labelledby="work-detail-title">
      <header className="work-dialog-header">
        <h2 id="work-detail-title">{title}</h2>
        <button type="button" className="pixel-btn work-dialog-close" aria-label={t("collapseWork")} title={t("collapseWork")} onClick={onClose}>×</button>
      </header>
      {children}
    </section>
  </div>, document.body);
}

export function WorkItemsPanel() {
  const { t } = useTranslation("workpanels");
  const { items, loaded, error, selectedId, add, edit, remove, select, reorder, load } = useWorkItemStore();
  const agents = useAppStore((s) => s.agents);
  const openTerminal = useAppStore((s) => s.openTerminal);
  const busy = useWorkSendStore((s) => s.busy);
  const confirmation = useWorkSendStore((s) => s.confirmation);
  const summaries = useSessionSummaryStore((s) => s.summaries);
  const monitored = useWorkMonitorStore((s) => s.sessions);
  const [draft, setDraft] = useState<WorkItemDraft | null>(null);
  const [creating, setCreating] = useState(false);
  // 닫기/다른 카드 선택은 초안을 보존한다. 저장과 취소에서만 지운다.
  const [edits, setEdits] = useState<Record<string, Partial<WorkItemDraft>>>({});
  const clearEdit = (id: string) => setEdits((current) => { const next = { ...current }; delete next[id]; return next; });
  const [deleting, setDeleting] = useState<WorkItem | null>(null);
  useEffect(() => { if (!loaded && !error) void load(); }, [loaded, error, load]);
  useEffect(() => () => { decideWorkSend(false); void useWorkItemStore.getState().flush(); }, []);
  const cardState = (item: WorkItem) => {
    const sessionId = item.sessionIds[item.sessionIds.length - 1];
    return {
      sessionId,
      dirty: hasWorkDraftChanges(item, edits[item.id] ?? {}),
      terminalAgent: (sessionId ? summaries[sessionId]?.agentId ?? monitored[sessionId]?.agentId : undefined) ?? item.assigneeAgentId,
      waitingOn: item.status === "done" ? [] : pendingPrerequisites(item, items),
    };
  };
  const selected = selectedId ? items.find((item) => item.id === selectedId) : undefined;
  const detail = () => {
    if (!selected) return null;
    const item = selected;
    const patch = edits[item.id] ?? {};
    const value = { ...item, ...patch };
    const { sessionId, dirty, terminalAgent } = cardState(item);
    // 보내기는 저장 후 초안 기준으로 판정한다.
    const assignee = value.assigneeAgentId;
    const waitingOn = value.status === "done" ? [] : pendingPrerequisites(value, items);
    const sending = !!busy[item.id];
    const agentBusy = !!assignee && Object.values(busy).includes(assignee);
    const blockedReason = !hasWorkContent(value) ? undefined : waitingOn.length ? "work-send-blocked"
      : workSendDisabledReason(assignee ? agents[assignee] : undefined) ?? (agentBusy && !sending ? "work-send-busy" : undefined);
    const save = () => { edit(item.id, patch); clearEdit(item.id); void useWorkItemStore.getState().flush(); };
    // 보내기에 성공하면 팝업을 닫아 열린 터미널을 보여준다. 실패·확인 취소는 오류를 볼 수 있게 열어 둔다.
    const send = async () => {
      if (dirty) save();
      const before = item.sessionIds.length;
      await requestWorkSend(item.id);
      const { items: after, selectedId: open } = useWorkItemStore.getState();
      const sent = after.find((i) => i.id === item.id);
      if (open === item.id && sent && !sent.lastError && sent.sessionIds.length > before) select(null);
    };
    const markDone = () => {
      edit(item.id, { status: "done" });
      // 완료 표시는 상태만 바꾼다. 나머지 초안은 그대로 둔다.
      setEdits((current) => { const { status: _status, ...rest } = current[item.id] ?? {}; return { ...current, [item.id]: rest }; });
      void useWorkItemStore.getState().flush(); select(null);
    };
    return <WorkDialog key={item.id} title={workItemTitle(item)} onClose={() => select(null)}>
      {sessionId && <WorkCardMonitor sessionId={sessionId} assignee={item.assigneeAgentId} status={item.status} />}
      <WorkEditor itemId={item.id} value={value} busy={sending} dirty={dirty}
        onChange={(change) => setEdits((current) => ({ ...current, [item.id]: { ...current[item.id], ...change } }))}
        onSave={save} onDiscard={() => clearEdit(item.id)} />
      {item.lastError && <p role="alert">{sendErrorText(item.lastError)}</p>}
      {/* 실행 줄: 위험한 삭제는 왼쪽 끝에 떼어 두고 주 동작인 보내기를 오른쪽 끝에 둔다. */}
      <div className="work-dialog-actions work-dialog-run">
        <button className="pixel-btn danger" disabled={sending} onClick={() => setDeleting(item)}>{t("delete")}</button>
        <span className="work-dialog-spacer" />
        {item.status !== "done" && <button className="pixel-btn" disabled={sending} onClick={markDone}>{t("markDone")}</button>}
        {/* 터미널은 팝업 뒤에 열리므로 초안을 둔 채 팝업을 닫는다. */}
        {terminalAgent && agents[terminalAgent] && <button className="pixel-btn" onClick={() => { select(null); openTerminal(terminalAgent); }}>{t("terminal")}</button>}
        <button className="pixel-btn primary work-send-btn" disabled={sending || !!blockedReason || !hasWorkContent(value)} aria-describedby={blockedReason ? "work-send-hint" : undefined}
          onClick={() => void send()}>{t(sending ? "sending" : dirty ? "saveAndSend" : "send")}</button>
      </div>
      {blockedReason && <p id="work-send-hint" className="work-send-hint">{sendErrorText(blockedReason)}</p>}
    </WorkDialog>;
  };
  return <><WorkPanelFrame side="left" title={t("workTitle")}>
    <button className="pixel-btn" disabled={!loaded} aria-expanded={creating} onClick={() => { setDraft((current) => current ?? { ...EMPTY_WORK_DRAFT }); setCreating(!creating); select(null); }}>{t(draft && !creating ? "resumeNewWork" : "newWork")}</button>
    {error && <div role="alert">{t(error === "load" ? "loadError" : "saveError")} <button className="pixel-btn" onClick={() => void (error === "load" ? load() : useWorkItemStore.getState().flush())}>{t("retry")}</button></div>}
    {!items.length && loaded && <p className="work-muted">{t("emptyWork")}</p>}
    {WORK_STATUS_ORDER.map((status) => <details className="work-group" key={status} open={status !== "done" || items.some((item) => item.status === status && item.id === selectedId)}>
      <summary>{t(`status.${status}`)} ({items.filter((i) => i.status === status).length})</summary>
      {items.filter((i) => i.status === status).sort((a, b) => a.order - b.order).map((item) => {
        const { sessionId, dirty, waitingOn } = cardState(item);
        const dependents = dependentCount(item.id, items);
        return <article className={`work-card${sessionId && item.status !== "done" && monitored[sessionId]?.phase === "waiting" ? " waiting" : ""}${selectedId === item.id ? " selected" : ""}`} key={item.id} id={`work-item-${item.id}`}
        onDragOver={(e) => e.preventDefault()}
        onDrop={(e) => {
          e.preventDefault();
          const rect = e.currentTarget.getBoundingClientRect();
          reorder(e.dataTransfer.getData("application/x-agent-office-work"), item.id, e.clientY > rect.top + rect.height / 2);
        }}>
        <button className="work-card-heading" draggable onDragStart={(e) => e.dataTransfer.setData("application/x-agent-office-work", item.id)} aria-haspopup="dialog" aria-expanded={selectedId === item.id} onClick={() => { select(item.id); setCreating(false); }}>
          {item.assigneeAgentId && <AgentFace agentId={item.assigneeAgentId} />}
          <span><strong className={!item.title.trim() ? "work-title-derived" : undefined}>{workItemTitle(item)}</strong>{dirty && <small>{t("unsavedChanges")}</small>}<small>{item.assigneeAgentId ? agents[item.assigneeAgentId]?.name ?? t("missingAgent") : t("unassigned")}</small>
            {waitingOn.length > 0 && <small className="work-blocked">{t("blockedBy", { titles: waitingOn.map(workItemTitle).join(", ") })}</small>}
            {dependents > 0 && item.status !== "done" && <small>{t("dependents", { count: dependents })}</small>}</span>
        </button>
        {sessionId && <WorkCardMonitor sessionId={sessionId} assignee={item.assigneeAgentId} status={item.status} />}
      </article>; })}
    </details>)}
  </WorkPanelFrame>
    {creating && draft && <WorkDialog title={t("newWorkTitle")} onClose={() => setCreating(false)}>
      <WorkEditor creating value={draft} dirty={hasWorkDraftChanges(EMPTY_WORK_DRAFT, draft)} onChange={(patch) => setDraft({ ...draft, ...patch })}
        onSave={() => { if (add(draft)) { setDraft(null); setCreating(false); select(null); void useWorkItemStore.getState().flush(); } }}
        onDiscard={() => { setDraft(null); setCreating(false); }} />
    </WorkDialog>}
    {!deleting && !confirmation && detail()}
    {confirmation && createPortal(<div className="modal-backdrop" onKeyDown={(e) => { if (e.key === "Escape") { e.stopPropagation(); decideWorkSend(false); } }}>
      <section className="pixel-panel work-delete-dialog" role="alertdialog" aria-modal="true" aria-labelledby="work-send-title" aria-describedby="work-send-body">
        <h2 id="work-send-title">{t("restartSendTitle")}</h2>
        <p id="work-send-body">{t("restartSendBody", { name: confirmation.agentName, title: confirmation.title })}</p>
        <p>{t("restartSendWarning")}</p>
        {confirmation.target.hasUncommitted && <p>{t("restartSendUncommitted")}</p>}
        <div className="dialog-actions">
          <button className="pixel-btn" autoFocus onClick={() => decideWorkSend(false)}>{t("cancel")}</button>
          <span className="work-dialog-spacer" />
          <button className="pixel-btn danger" onClick={() => decideWorkSend(true)}>{t("restartSendConfirm")}</button>
        </div>
      </section>
    </div>, document.body)}
    {deleting && createPortal(<div className="modal-backdrop" onKeyDown={(e) => { if (e.key === "Escape") { e.stopPropagation(); setDeleting(null); } }}>
      <section className="pixel-panel work-delete-dialog" role="alertdialog" aria-modal="true" aria-labelledby="work-delete-title" aria-describedby="work-delete-body">
        <h2 id="work-delete-title">{t("deleteTitle")}</h2>
        <p id="work-delete-body">{t("deleteBody", { title: workItemTitle(deleting) })}</p>
        <div className="dialog-actions">
          <button className="pixel-btn" autoFocus onClick={() => setDeleting(null)}>{t("cancel")}</button>
          <span className="work-dialog-spacer" />
          <button className="pixel-btn danger" onClick={() => { remove(deleting.id); clearEdit(deleting.id); setDeleting(null); }}>{t("delete")}</button>
        </div>
      </section>
    </div>, document.body)}
  </>;
}
