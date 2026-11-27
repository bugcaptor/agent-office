import { useId } from "react";
import { useTranslation } from "react-i18next";
import type { WorkStatus } from "@shared/types";
import { useAppStore } from "../store/appStore";
import { useWorkItemStore, WORK_STATUS_ORDER } from "./workItemStore";
import { dependencyCandidates } from "./workDependencies";
import { WorkAgentFields } from "./WorkAgentFields";
import { AUTO_RUN_LIMITS, validWorkLimits } from "./autoRunLimits";
import { hasWorkContent, workItemTitle, type WorkItemDraft } from "./workItemDraft";
import { findRemote, useRemoteFloors } from "../link/remoteFloors";
import { repoIssue } from "../link/remoteWork";
import type { WorkRepo } from "@shared/types";

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
  const office = useRemoteFloors((s) => s.office);
  const remote = value.assigneeAgentId ? findRemote(office, value.assigneeAgentId) : null;
  const dependsOn = value.dependsOn ?? [];
  const candidates = dependencyCandidates(itemId, dependsOn, items);
  const setDependsOn = (next: string[]) => onChange({ dependsOn: next.length ? next : undefined });
  const canSave = !busy && dirty && hasWorkContent(value) && validWorkLimits(value);
  return <form className="work-editor" onSubmit={(e) => { e.preventDefault(); if (canSave) onSave(); }}>
    <fieldset disabled={busy}>
      <label>{t("title")}<input value={value.title} maxLength={500} placeholder={workItemTitle({ title: "", body: value.body }) || t("titlePlaceholder")} onChange={(e) => onChange({ title: e.target.value })} /></label>
      <p className="work-muted">{t("titleHint")}</p>
      <label>{t("body")}<textarea value={value.body} rows={4} onChange={(e) => onChange({ body: e.target.value })} /></label>
      <label>{t("assignee")}<select value={value.assigneeAgentId ?? ""} onChange={(e) => onChange({ assigneeAgentId: e.target.value || undefined })}>
        <option value="">{t("unassigned")}</option>
        {value.assigneeAgentId && !agents[value.assigneeAgentId] && !remote && <option value={value.assigneeAgentId}>{t("missingAgent")}</option>}
        {Object.values(agents).map((agent) => <option key={agent.id} value={agent.id}>{agent.name}</option>)}
        {/* 머신 연동 호스트: 붙은(또는 끊긴) 머신의 캐릭터에게도 맡긴다. */}
        {office.machines.filter((m) => m.characters.length).map((m) => <optgroup key={m.deviceId} label={t("remoteGroup", { machine: m.name })}>
          {m.characters.map((c) => <option key={c.id} value={c.id}>{c.name}</option>)}
        </optgroup>)}
      </select></label>
      {remote && <RepoFields value={value.repo} machine={remote.machine} onChange={(repo) => onChange({ repo })} />}
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
      <details className="work-run-limits">
        <summary>{t("autoRun.limits.title")}</summary>
        <p className="work-muted">{t("autoRun.limits.hint", { countDefault: AUTO_RUN_LIMITS.maxFollowUps.defaultValue, minutes: AUTO_RUN_LIMITS.permissionWaitMinutes.defaultValue })}</p>
        {(["maxFollowUps", "permissionWaitMinutes"] as const).map((key) => {
          const { min, max } = AUTO_RUN_LIMITS[key];
          return <label key={key}>{t(`autoRun.limits.${key}`, { min, max })}
            <input type="number" min={min} max={max} step={1} value={value[key] ?? ""} placeholder={t("autoRun.limits.inherit")}
              onChange={(e) => onChange({ [key]: e.target.value === "" ? undefined : e.target.valueAsNumber })} />
          </label>;
        })}
      </details>
    </fieldset>
    {/* 되돌리기는 저장과 떨어진 왼쪽 끝에 두고, 변경이 없을 때도 자리를 지켜 저장 버튼이 움직이지 않게 한다. */}
    <div className="work-dialog-actions">
      <button type="button" className="pixel-btn" style={dirty ? undefined : { visibility: "hidden" }} disabled={busy} onClick={onDiscard}>{t(creating ? "discardDraft" : "revertChanges")}</button>
      <span className="work-dialog-spacer" />
      <button type="submit" className="pixel-btn primary" disabled={!canSave}>{t(creating ? "add" : "save")}</button>
    </div>
  </form>;
}

/** 다른 머신 캐릭터에게 맡길 때의 저장소. 그 머신이 알린 클론 목록에서 고르거나 주소를 넣는다. */
function RepoFields({ value, machine, onChange }: { value?: WorkRepo; machine: import("@shared/types").RemoteMachine; onChange(repo: WorkRepo | undefined): void }) {
  const { t } = useTranslation("workpanels");
  const id = useId();
  const repo = value ?? { url: "" };
  const set = (patch: Partial<WorkRepo>) => {
    const next: WorkRepo = { ...repo, ...patch };
    // 빈 값은 저장하지 않는다. 주소가 비면 저장소 없음(그 캐릭터의 시작 폴더)이다.
    for (const key of ["subpath", "branch"] as const) if (!next[key]?.trim()) delete next[key];
    if (!next.worktree) delete next.worktree;
    onChange(next.url.trim() || next.subpath || next.branch || next.worktree ? next : undefined);
  };
  const issue = repoIssue(value, machine);
  return <fieldset className="work-repo-fields">
    <legend>{t("repo.title")}</legend>
    <label>{t("repo.url")}<input list={id} value={repo.url} placeholder={t("repo.urlPlaceholder")} spellCheck={false} onChange={(e) => set({ url: e.target.value })} /></label>
    <datalist id={id} aria-label={t("repo.known", { machine: machine.name })}>{machine.repos.map((url) => <option key={url} value={url} />)}</datalist>
    <label>{t("repo.subpath")}<input value={repo.subpath ?? ""} spellCheck={false} onChange={(e) => set({ subpath: e.target.value })} /></label>
    <label>{t("repo.branch")}<input value={repo.branch ?? ""} spellCheck={false} onChange={(e) => set({ branch: e.target.value })} /></label>
    <label className="work-plan-root-toggle"><input type="checkbox" checked={!!repo.worktree} onChange={(e) => set({ worktree: e.target.checked })} />{t("repo.worktree")}</label>
    <p className="work-muted">{t("repo.hint")}</p>
    {!machine.online && <p className="work-send-hint">{t("repo.offline", { machine: machine.name })}</p>}
    {issue === "link-repo-missing" && <p className="work-send-hint" role="alert">{t("repo.missing", { machine: machine.name })}</p>}
    {issue === "link-repo-invalid" && <p className="work-send-hint" role="alert">{t("sendErrors.repoInvalid")}</p>}
  </fieldset>;
}
