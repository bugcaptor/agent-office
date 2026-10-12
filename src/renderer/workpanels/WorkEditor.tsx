import { useTranslation } from "react-i18next";
import type { WorkStatus } from "@shared/types";
import { useAppStore } from "../store/appStore";
import { WORK_STATUS_ORDER } from "./workItemStore";
import { WorkAgentFields } from "./WorkAgentFields";
import { hasWorkContent, workItemTitle, type WorkItemDraft } from "./workItemDraft";

export function WorkEditor({ value, busy = false, creating = false, onChange, onSave, onCancel, onCollapse }: {
  value: WorkItemDraft;
  busy?: boolean;
  creating?: boolean;
  onChange(patch: Partial<WorkItemDraft>): void;
  onSave(): void;
  onCancel(): void;
  onCollapse(): void;
}) {
  const { t } = useTranslation("workpanels");
  const agents = useAppStore((s) => s.agents);
  return <form className="work-editor" onSubmit={(e) => { e.preventDefault(); if (!busy && hasWorkContent(value)) onSave(); }}
    onKeyDown={(e) => { if (e.key === "Escape") { e.stopPropagation(); onCollapse(); } }}>
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
    </fieldset>
    <div className="work-card-actions">
      <button type="submit" className="pixel-btn" disabled={busy || !hasWorkContent(value)}>{t(creating ? "add" : "save")}</button>
      <button type="button" className="pixel-btn" disabled={busy} onClick={onCancel}>{t("cancel")}</button>
      <button type="button" className="pixel-btn" onClick={onCollapse}>{t("collapseWork")}</button>
    </div>
  </form>;
}
