import { useTranslation } from "react-i18next";
import type { WorkStatus } from "@shared/types";
import { useAppStore } from "../store/appStore";
import { useWorkItemStore, WORK_STATUS_ORDER } from "./workItemStore";
import { dependencyCandidates } from "./workDependencies";
import { WorkAgentFields } from "./WorkAgentFields";
import { hasWorkContent, workItemTitle, type WorkItemDraft } from "./workItemDraft";

export function WorkEditor({ itemId, value, busy = false, creating = false, dirty, onChange, onSave, onDiscard }: {
  /** 편집 중인 업무. 새 업무면 없다. 선행 작업 후보에서 자신과 순환을 뺄 때 쓴다. */
  itemId?: string;
  value: WorkItemDraft;
  busy?: boolean;
  creating?: boolean;
  /** 저장·되돌릴 변경이 있는지. 없으면 저장을 막고 되돌리기를 감춘다. */
  dirty: boolean;
  onChange(patch: Partial<WorkItemDraft>): void;
  onSave(): void;
  /** 초안을 버린다. 닫기는 팝업(헤더 ×·Esc)이 맡는다. */
  onDiscard(): void;
}) {
  const { t } = useTranslation("workpanels");
  const agents = useAppStore((s) => s.agents);
  const items = useWorkItemStore((s) => s.items);
  const dependsOn = value.dependsOn ?? [];
  const candidates = dependencyCandidates(itemId, dependsOn, items);
  const setDependsOn = (next: string[]) => onChange({ dependsOn: next.length ? next : undefined });
  const canSave = !busy && dirty && hasWorkContent(value);
  return <form className="work-editor" onSubmit={(e) => { e.preventDefault(); if (canSave) onSave(); }}>
    <fieldset disabled={busy}>
      <label>{t("title")}<input value={value.title} maxLength={500} placeholder={workItemTitle({ title: "", body: value.body }) || t("titlePlaceholder")} onChange={(e) => onChange({ title: e.target.value })} /></label>
      <p className="work-muted">{t("titleHint")}</p>
      <label>{t("body")}<textarea value={value.body} rows={4} onChange={(e) => onChange({ body: e.target.value })} /></label>
      <label>{t("assignee")}<select value={value.assigneeAgentId ?? ""} onChange={(e) => onChange({ assigneeAgentId: e.target.value || undefined })}>
        <option value="">{t("unassigned")}</option>
        {value.assigneeAgentId && !agents[value.assigneeAgentId] && <option value={value.assigneeAgentId}>{t("missingAgent")}</option>}
        {Object.values(agents).map((agent) => <option key={agent.id} value={agent.id}>{agent.name}</option>)}
      </select></label>
      <WorkAgentFields value={value.agent} inherited={value.assigneeAgentId ? agents[value.assigneeAgentId]?.defaultAgent : undefined} onChange={(agent) => onChange({ agent })} />
      <label>{t("statusLabel")}<select value={value.status} onChange={(e) => onChange({ status: e.target.value as WorkStatus })}>
        {WORK_STATUS_ORDER.map((status) => <option key={status} value={status}>{t(`status.${status}`)}</option>)}
      </select></label>
      <div className="work-dependencies" role="group" aria-label={t("dependsOn")}>
        <span>{t("dependsOn")}</span>
        {dependsOn.length ? <ul>{dependsOn.map((id) => {
          const other = items.find((item) => item.id === id);
          const title = other ? workItemTitle(other) : t("missingWork");
          return <li key={id}>
            <span className={other?.status === "done" ? "work-muted" : undefined}>{title}{other && <small>{t(`status.${other.status}`)}</small>}</span>
            <button type="button" className="pixel-btn" aria-label={t("removeDependency", { title })} onClick={() => setDependsOn(dependsOn.filter((d) => d !== id))}>×</button>
          </li>;
        })}</ul> : <p className="work-muted">{t("noDependencies")}</p>}
        <select value="" aria-label={t("addDependency")} disabled={!candidates.length} onChange={(e) => { if (e.target.value) setDependsOn([...dependsOn, e.target.value]); }}>
          <option value="">{t(candidates.length ? "addDependency" : "noDependencyCandidates")}</option>
          {candidates.map((item) => <option key={item.id} value={item.id}>{workItemTitle(item)}</option>)}
        </select>
      </div>
      {/* 플랜 루트 표시는 기다리는 태스크가 있을 때만 뜻이 있다. 켜져 있으면 끌 수 있게 계속 보인다. */}
      {(dependsOn.length > 0 || value.plan) && <label className="work-plan-root-toggle">
        <input type="checkbox" checked={!!value.plan} onChange={(e) => onChange({ plan: e.target.checked || undefined })} />
        {t("planRoot")}
      </label>}
      {(dependsOn.length > 0 || value.plan) && <p className="work-muted">{t(value.plan ? "planRootOn" : "planRootOff")}</p>}
    </fieldset>
    {/* 되돌리기는 저장과 떨어진 왼쪽 끝에 두고, 변경이 없을 때도 자리를 지켜 저장 버튼이 움직이지 않게 한다. */}
    <div className="work-dialog-actions">
      <button type="button" className="pixel-btn" style={dirty ? undefined : { visibility: "hidden" }} disabled={busy} onClick={onDiscard}>{t(creating ? "discardDraft" : "revertChanges")}</button>
      <span className="work-dialog-spacer" />
      <button type="submit" className="pixel-btn primary" disabled={!canSave}>{t(creating ? "add" : "save")}</button>
    </div>
  </form>;
}
