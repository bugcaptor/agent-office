import { useEffect, useState } from "react";
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
import { WorkDialog } from "./WorkDialog";
import { TaskChatButton, useTaskChatButton } from "./TaskChatButton";
import { WorkTransferDialog } from "./WorkTransferDialog";
import { AutoRunSection, blockedText } from "./AutoRunSection";
import { endAutoRun, runSteps, startAutoRun, startBlockedReason } from "./autoRun";
import { planLayout } from "./workPlans";
import { assigneeKey, MISSING_ASSIGNEE } from "./workItemFile";
import { useWorkGroupStore } from "./panelLayoutStore";
import { projectAnchorCwd, projectNameFromCwd } from "../labels/labelText";

export function WorkItemsPanel() {
  const { t } = useTranslation("workpanels");
  const { items, loaded, error, selectedId, add, edit, remove, select, reorder, load } = useWorkItemStore();
  const agents = useAppStore((s) => s.agents);
  const agentOrder = useAppStore((s) => s.agentOrder);
  const openTerminal = useAppStore((s) => s.openTerminal);
  const taskLabels = useAppStore((s) => s.taskLabels);
  const observerEnabled = useAppStore((s) => s.appSettings.observerEnabled);
  const busy = useWorkSendStore((s) => s.busy);
  const confirmation = useWorkSendStore((s) => s.confirmation);
  const summaries = useSessionSummaryStore((s) => s.summaries);
  const monitored = useWorkMonitorStore((s) => s.sessions);
  const collapsedGroups = useWorkGroupStore((s) => s.collapsed);
  const setGroupCollapsed = useWorkGroupStore((s) => s.setCollapsed);
  const [draft, setDraft] = useState<WorkItemDraft | null>(null);
  const [creating, setCreating] = useState(false);
  // 닫기/다른 카드 선택은 초안을 보존한다. 저장과 취소에서만 지운다.
  const [edits, setEdits] = useState<Record<string, Partial<WorkItemDraft>>>({});
  const clearEdit = (id: string) => setEdits((current) => { const next = { ...current }; delete next[id]; return next; });
  const [deleting, setDeleting] = useState<WorkItem | null>(null);
  const [transferring, setTransferring] = useState(false);
  // 대화가 열리면 그 태스크 팝업을 닫는다(초안은 edits에 남는다).
  const closeIfOpen = (id: string) => { if (useWorkItemStore.getState().selectedId === id) select(null); };
  const chat = useTaskChatButton(selectedId ?? undefined, closeIfOpen);
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
    // 플랜 루트는 플랜 메모라 보내지 않는다. 자동 진행이 선행 태스크만 진행한다.
    const blockedReason = !hasWorkContent(value) ? undefined : value.plan ? "work-send-plan-root" : waitingOn.length ? "work-send-blocked"
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
    return <WorkDialog key={item.id} title={workItemTitle(item)} id={item.id} onClose={() => select(null)}>
      {sessionId && <WorkCardMonitor sessionId={sessionId} assignee={item.assigneeAgentId} status={item.status} />}
      <WorkEditor itemId={item.id} value={value} busy={sending} dirty={dirty}
        onChange={(change) => setEdits((current) => ({ ...current, [item.id]: { ...current[item.id], ...change } }))}
        onSave={save} onDiscard={() => clearEdit(item.id)} />
      {item.lastError && <p role="alert">{sendErrorText(item.lastError)}</p>}
      <AutoRunSection root={item} />
      {/* 실행 줄: 위험한 삭제는 왼쪽 끝에 떼어 두고 주 동작인 보내기를 오른쪽 끝에 둔다. */}
      <div className="work-dialog-actions work-dialog-run">
        <button className="pixel-btn danger" disabled={sending} onClick={() => setDeleting(item)}>{t("delete")}</button>
        <span className="work-dialog-spacer" />
        {item.status !== "done" && <button className="pixel-btn" disabled={sending} onClick={markDone}>{t("markDone")}</button>}
        {/* 대화는 저장된 태스크로 맥락을 만든다. 터미널은 팝업 뒤에 열리므로 둘 다 초안을 둔 채 팝업을 닫는다. */}
        <TaskChatButton chat={chat} label={t("taskChat.open")} title={dirty ? t("taskChat.savedOnly") : undefined} hintId="work-chat-hint" />
        {terminalAgent && agents[terminalAgent] && <button className="pixel-btn" onClick={() => { select(null); openTerminal(terminalAgent); }}>{t("terminal")}</button>}
        <button className="pixel-btn primary work-send-btn" disabled={sending || !!blockedReason || !hasWorkContent(value)} aria-describedby={blockedReason ? "work-send-hint" : undefined}
          onClick={() => void send()}>{t(sending ? "sending" : dirty ? "saveAndSend" : "send")}</button>
      </div>
      {chat.hint && <p id="work-chat-hint" className="work-send-hint">{chat.hint}</p>}
      {blockedReason && <p id="work-send-hint" className="work-send-hint">{sendErrorText(blockedReason)}</p>}
    </WorkDialog>;
  };
  // plan: 플랜 루트 카드(대기 줄 대신 진행도·접기·시작). member: 플랜 아래에 들인 카드(순서는 들여쓰기가 보여 주므로 후속 수는 뺀다).
  const card = (item: WorkItem, role?: { plan?: { done: number; total: number; expanded: boolean; toggle?: () => void }; member?: { unassigned: boolean } }) => {
    const { sessionId, dirty, waitingOn } = cardState(item);
    const dependents = dependentCount(item.id, items);
    const plan = role?.plan;
    const run = item.autoRun;
    const startable = plan && (!run || run.state === "finished");
    const blocked = startable ? startBlockedReason(item, items, agents, observerEnabled) : undefined;
    const heading = <button className="work-card-heading" draggable onDragStart={(e) => e.dataTransfer.setData("application/x-agent-office-work", item.id)} aria-haspopup="dialog" aria-expanded={selectedId === item.id} onClick={() => { select(item.id); setCreating(false); setTransferring(false); }}>
      <span>{plan && <span className="work-status-chip work-plan-chip">{t("plan.chip")}</span>}
        {(!plan || item.status !== "todo") && <span className="work-status-chip">{t(`status.${item.status}`)}</span>}
        <strong className={!item.title.trim() ? "work-title-derived" : undefined}>{workItemTitle(item)}</strong>{dirty && <small>{t("unsavedChanges")}</small>}
        {run && <small className={`work-auto-chip ${run.state}`}>{t(`autoRun.chip.${run.state}`)}</small>}
        {plan && !item.plan && <small title={t("plan.rootRunsHint")}>{t("plan.rootRuns")}</small>}
        {plan && <small className="work-plan-progress">{t("autoRun.progress", { done: plan.done, total: plan.total })}</small>}
        {role?.member?.unassigned && <small>{t("unassigned")}</small>}
        {!plan && waitingOn.length > 0 && <small className="work-blocked">{t("blockedBy", { titles: waitingOn.map(workItemTitle).join(", ") })}</small>}
        {!plan && !role?.member && dependents > 0 && item.status !== "done" && <small>{t("dependents", { count: dependents })}</small>}</span>
    </button>;
    // 담당은 캐릭터 묶음 머리에 있으므로 카드에는 얼굴·이름을 다시 쓰지 않는다.
    return <article className={`work-card status-${item.status}${plan ? " work-plan-root" : ""}${sessionId && item.status !== "done" && monitored[sessionId]?.phase === "waiting" ? " waiting" : ""}${selectedId === item.id ? " selected" : ""}`} key={item.id} id={`work-item-${item.id}`}
      onDragOver={(e) => e.preventDefault()}
      onDrop={(e) => {
        e.preventDefault();
        const rect = e.currentTarget.getBoundingClientRect();
        reorder(e.dataTransfer.getData("application/x-agent-office-work"), item.id, e.clientY > rect.top + rect.height / 2);
      }}>
      {plan ? <div className="work-plan-head">
        <button type="button" className="work-plan-toggle" disabled={!plan.toggle} aria-expanded={plan.expanded} aria-label={t("plan.toggle")} title={t("plan.toggle")} onClick={plan.toggle}>{plan.expanded ? "▾" : "▸"}</button>
        {heading}
        {startable && <button type="button" className="pixel-btn work-plan-start" disabled={!!blocked} aria-label={t("autoRun.start")} title={blocked ? blockedText(t, blocked, item, items) : t("autoRun.start")}
          onClick={() => { if (run) endAutoRun(item.id); startAutoRun(item.id); }}>▶</button>}
      </div> : heading}
      {sessionId && <WorkCardMonitor sessionId={sessionId} assignee={item.assigneeAgentId} status={item.status} />}
    </article>;
  };
  const plans = planLayout(items, (item) => assigneeKey(item, agents));
  // 플랜 루트와 그 아래 들인 업무. 접힘은 캐릭터 묶음과 같은 저장소에 `plan:<id>`로 둔다.
  const planBlock = (root: WorkItem, groupKey: string) => {
    const members = plans.members.get(root.id) ?? [];
    const all = runSteps(root, items);
    const key = `plan:${root.id}`;
    const expanded = !collapsedGroups.includes(key) || members.some((item) => item.id === selectedId);
    return <div className="work-plan" key={root.id}>
      {card(root, { plan: { done: all.filter((item) => item.status === "done").length, total: all.length, expanded, toggle: members.length ? () => setGroupCollapsed(key, expanded) : undefined } })}
      {expanded && members.length > 0 && <div className="work-plan-members">
        {members.map((item) => card(item, { member: { unassigned: assigneeKey(item, agents) !== groupKey } }))}
      </div>}
    </div>;
  };
  // 캐릭터(오피스 순서) → 미배정 → 삭제된 캐릭터 순. 캐릭터 묶음은 업무가 없어도 '＋'로 바로 만들 수 있게 보이고,
  // 미배정·삭제된 캐릭터 묶음은 업무가 있을 때만 보인다. agentOrder에 없는 캐릭터도 업무가 사라지지 않게 뒤에 붙인다.
  const groups = [...new Set([...agentOrder.filter((id) => agents[id]), ...Object.keys(agents)]), "", MISSING_ASSIGNEE].map((key) => ({
    key, name: key === "" ? t("unassigned") : key === MISSING_ASSIGNEE ? t("missingAgent") : agents[key].name,
    // 플랜 루트 아래로 옮긴 업무는 제 묶음에서 뺀다.
    items: items.filter((item) => assigneeKey(item, agents) === key && !plans.claimed.has(item.id)),
  })).filter((group) => group.items.length > 0 || agents[group.key]);
  // 프로젝트는 머리 위 라벨과 같은 규칙: 세션 cwd가 프로필 cwd 밖으로 나갔을 때만 세션 쪽 폴더 이름.
  const projectOf = (agentId: string) => {
    const cwd = projectAnchorCwd(taskLabels[agentId]?.cwd, agents[agentId]?.cwd);
    return { cwd, name: projectNameFromCwd(cwd) };
  };
  // 캐릭터를 지정하면 쓰던 초안은 살린 채 담당만 그 캐릭터로 바꾼다.
  const startNew = (assigneeAgentId: string) => {
    setDraft((current) => ({ ...(current ?? EMPTY_WORK_DRAFT), assigneeAgentId }));
    setCreating(true); setTransferring(false); select(null);
  };
  return <><WorkPanelFrame side="left" title={t("workTitle")}>
    <div className="work-panel-actions">
      <button className="pixel-btn work-new-btn" disabled={!loaded} aria-expanded={creating} onClick={() => { setDraft((current) => current ?? { ...EMPTY_WORK_DRAFT }); setCreating(!creating); setTransferring(false); select(null); }}>{t(draft && !creating ? "resumeNewWork" : "newWork")}</button>
      <button className="pixel-btn" disabled={!loaded} aria-haspopup="dialog" aria-expanded={transferring} aria-label={t("transfer.open")} title={t("transfer.open")} onClick={() => { setTransferring(true); setCreating(false); select(null); }}>⇅</button>
    </div>
    {error && <div role="alert">{t(error === "load" ? "loadError" : "saveError")} <button className="pixel-btn" onClick={() => void (error === "load" ? load() : useWorkItemStore.getState().flush())}>{t("retry")}</button></div>}
    {!items.length && loaded && <p className="work-muted">{t("emptyWork")}</p>}
    {groups.map(({ key, name, items: owned }) => {
      const project = agents[key] ? projectOf(key) : undefined;
      // 상태는 소제목 대신 카드의 칩·색 띠로 보여 주고, 완료만 맨 아래에 접어 둔다.
      const open = WORK_STATUS_ORDER.filter((status) => status !== "done").flatMap((status) => owned.filter((i) => i.status === status).sort((a, b) => a.order - b.order));
      const done = owned.filter((i) => i.status === "done").sort((a, b) => a.order - b.order);
      const remaining = open.length + open.reduce((sum, item) => sum + (plans.members.get(item.id)?.filter((member) => member.status !== "done").length ?? 0), 0);
      return <details className={`work-group work-agent-group${owned.length ? "" : " empty"}`} key={key} open={!collapsedGroups.includes(key)}
        onToggle={(e) => setGroupCollapsed(key, !e.currentTarget.open)}>
        <summary>{agents[key] && <AgentFace agentId={key} />}
          <span className="work-group-name"><strong>{name}</strong>
            {project && <small className={`work-group-project${project.name ? "" : " none"}`} title={project.cwd}>{project.name ?? t("homeFolder")}</small>}</span>
          {/* 완료는 아래 접힌 묶음에 따로 세므로 머리에는 남은 업무만 센다. */}
          <span className={`work-group-count${remaining ? "" : " zero"}`} aria-label={t("workCount", { count: remaining })}>{remaining}</span>
          {/* summary 안의 버튼이라 기본 동작(묶음 접기·펴기)을 막는다. */}
          {agents[key] && <button type="button" className="pixel-btn work-group-add" disabled={!loaded} aria-label={t("newWorkFor", { name })} title={t("newWorkFor", { name })}
            onClick={(e) => { e.preventDefault(); e.stopPropagation(); startNew(key); }}>＋</button>}</summary>
        {open.map((item) => plans.members.has(item.id) ? planBlock(item, key) : card(item))}
        {/* 끝나지 않은 자동 진행을 쥔 완료 업무가 있으면 접어 두지 않는다. 진행자를 붙잡고 있어서다. */}
        {done.length > 0 && <details className="work-done-group" open={done.some((item) => item.id === selectedId || (item.autoRun && item.autoRun.state !== "finished"))}>
          <summary>{t("status.done")} ({done.length})</summary>
          {done.map((item) => card(item))}
        </details>}
      </details>;
    })}
  </WorkPanelFrame>
    {creating && draft && <WorkDialog title={t("newWorkTitle")} onClose={() => setCreating(false)}>
      <WorkEditor creating value={draft} dirty={hasWorkDraftChanges(EMPTY_WORK_DRAFT, draft)} onChange={(patch) => setDraft({ ...draft, ...patch })}
        onSave={() => { if (add(draft)) { setDraft(null); setCreating(false); select(null); void useWorkItemStore.getState().flush(); } }}
        onDiscard={() => { setDraft(null); setCreating(false); }} />
    </WorkDialog>}
    {transferring && <WorkTransferDialog onClose={() => setTransferring(false)} />}
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
