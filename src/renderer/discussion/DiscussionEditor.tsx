// src/renderer/discussion/DiscussionEditor.tsx
//
// 토의 열기·고치기 대화상자. 주제·맥락·작업 폴더, 사회자 1 + 토론자 최대 4명의
// 팀원·역할·CLI/모델, 상한을 고른다. 저장만 하고 실행하지 않는다(준비 상태).
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";

import { DISCUSSION_LIMIT_RANGES, DISCUSSION_MAX_DEBATERS, discussionProblem, type Discussion, type DiscussionLimits } from "@shared/types";
import { useAppStore } from "../store/appStore";
import { WorkDialog } from "../workpanels/WorkDialog";
import { WorkAgentFields } from "../workpanels/WorkAgentFields";
import { workAgentLabel } from "../profile/generate";
import { useRoleStore } from "./roleStore";
import { useDiscussionStore } from "./discussionStore";
import {
  defaultAgent, discussionFromDraft, draftFromDiscussion, newDiscussionDraft, participantDraft, roleChoices, validDiscussionCwd, withRole,
  type DiscussionDraft, type ParticipantDraft,
} from "./discussionModel";
import "./discussion.css";

export function DiscussionEditor({ base, onClose }: { base?: Discussion; onClose(): void }) {
  const { t } = useTranslation("discussion");
  const agents = useAppStore((s) => s.agents);
  const agentOrder = useAppStore((s) => s.agentOrder);
  const roleStatus = useRoleStore((s) => s.status);
  const roles = useRoleStore((s) => s.roles);
  const [draft, setDraft] = useState<DiscussionDraft | null>(() => (base ? draftFromDiscussion(base) : null));
  const [saving, setSaving] = useState(false);
  const [saveError, setSaveError] = useState(false);

  useEffect(() => { void useRoleStore.getState().ensureLoaded(); }, []);
  // 새 토의는 역할 목록을 읽은 뒤(실패해도) 한 번 초안을 만든다 — 역할 기본값이 필요해서다.
  useEffect(() => {
    if (!draft && (roleStatus === "ready" || roleStatus === "error")) setDraft(newDiscussionDraft(agentOrder, agents, roles));
  }, [draft, roleStatus, agentOrder, agents, roles]);

  const order = [...new Set([...agentOrder.filter((id) => agents[id]), ...Object.keys(agents)])];
  const name = (id: string) => agents[id]?.name ?? t("board.missingAgent");

  if (!draft) {
    return <WorkDialog key="loading" title={t(base ? "editor.editTitle" : "editor.newTitle")} onClose={onClose} className="discussion-editor">
      <p className="form-hint">{t("editor.loading")}</p>
    </WorkDialog>;
  }

  const update = (patch: Partial<DiscussionDraft>) => setDraft({ ...draft, ...patch });
  const rows = [...(draft.moderator ? [draft.moderator] : []), ...draft.debaters];
  const used = new Set(rows.map((r) => r.agentId));
  const free = order.filter((id) => !used.has(id));
  const preview = discussionFromDraft(draft, 0, base);
  const problem = discussionProblem(preview);
  const invalidCwd = !validDiscussionCwd(preview.cwd);

  const setModerator = (moderator: ParticipantDraft | null) => update({ moderator });
  const setDebater = (index: number, row: ParticipantDraft) => update({ debaters: draft.debaters.map((r, i) => (i === index ? row : r)) });
  const addDebater = () => free[0] && update({ debaters: [...draft.debaters, participantDraft(free[0], agents, roles, false)] });
  const removeDebater = (index: number) => update({ debaters: draft.debaters.filter((_, i) => i !== index) });
  const setLimit = (key: keyof DiscussionLimits, value: number | boolean) => update({ limits: { ...draft.limits, [key]: value } });

  const save = async () => {
    if (problem || invalidCwd) return;
    setSaving(true); setSaveError(false);
    try {
      await useDiscussionStore.getState().save(discussionFromDraft(draft, Date.now(), base));
      onClose();
    } catch {
      setSaveError(true);
    } finally {
      setSaving(false);
    }
  };

  /** 한 사람 행: 팀원·역할 고르기와 CLI·모델. */
  const row = (value: ParticipantDraft, moderator: boolean, onChange: (row: ParticipantDraft) => void, onRemove?: () => void) => {
    const choices = roleChoices(roles, moderator);
    // 지워졌거나 스탠스가 바뀌어 이 칸에서 고를 수 없게 된 저장 역할도 그대로 보인다.
    const stray = value.roleId && !choices.some((r) => r.id === value.roleId) ? value.roleId : undefined;
    const memberOptions = [value.agentId, ...free];
    return <fieldset className="discussion-row" key={value.agentId}>
      <label><span className="form-label-text">{t("editor.member")}</span>
        <select value={value.agentId} onChange={(e) => onChange(participantDraft(e.target.value, agents, roles, moderator))}>
          {memberOptions.map((id) => <option key={id} value={id}>{name(id)}</option>)}
        </select></label>
      <label><span className="form-label-text">{t("editor.role")}</span>
        <select value={value.roleId ?? ""} onChange={(e) => onChange(withRole(value, e.target.value || undefined, agents, roles))}>
          <option value="">{t(moderator ? "editor.noRoleModerator" : "editor.noRoleDebater")}</option>
          {stray && <option value={stray}>{roles.find((r) => r.id === stray)?.name ?? t("board.missingRole")}</option>}
          {choices.map((r) => <option key={r.id} value={r.id}>{r.name}</option>)}
        </select></label>
      {onRemove && <button type="button" className="pixel-btn discussion-row-remove" aria-label={t("editor.removeDebater", { name: name(value.agentId) })} title={t("editor.removeDebater", { name: name(value.agentId) })} onClick={onRemove}>×</button>}
      <details className="discussion-agent-options">
        <summary>{workAgentLabel(value.agent)}</summary>
        <WorkAgentFields value={value.agent} labels={{ field: t("editor.agent"), empty: t("editor.agentDefault") }}
          onChange={(agent) => onChange(agent
            ? { ...value, agent, agentTouched: true }
            : { ...value, agent: defaultAgent(value.roleId, agents[value.agentId], roles), agentTouched: false })} />
      </details>
    </fieldset>;
  };

  const limitInput = (key: "maxRounds" | "maxMinutes" | "maxCostUsd", step: number) => {
    const { min, max } = DISCUSSION_LIMIT_RANGES[key];
    const v = draft.limits[key];
    return <label><span className="form-label-text">{t(`editor.${key}`)}</span>
      <input type="number" min={min} max={max} step={step} value={Number.isNaN(v) ? "" : v}
        onChange={(e) => setLimit(key, e.target.value === "" ? NaN : Number(e.target.value))} /></label>;
  };

  const { maxRounds, maxMinutes, maxCostUsd } = DISCUSSION_LIMIT_RANGES;
  // 불러오는 중 화면과 key를 나눠 폼이 새로 마운트되게 한다(첫 입력란 포커스·Esc 닫기).
  return <WorkDialog key="form" title={t(base ? "editor.editTitle" : "editor.newTitle")} onClose={onClose} className="discussion-editor">
    <label><span className="form-label-text">{t("editor.topic")}</span>
      <input value={draft.topic} placeholder={t("editor.topicPlaceholder")} onChange={(e) => update({ topic: e.target.value })} /></label>
    <label><span className="form-label-text">{t("editor.context")}</span>
      <textarea rows={4} value={draft.context} placeholder={t("editor.contextPlaceholder")} onChange={(e) => update({ context: e.target.value })} /></label>
    <label><span className="form-label-text">{t("editor.cwd")}</span>
      <input value={draft.cwd} aria-invalid={invalidCwd} aria-describedby={invalidCwd ? "discussion-cwd-error" : undefined} placeholder={t("editor.cwdPlaceholder")} onChange={(e) => update({ cwd: e.target.value })} /></label>

    {invalidCwd && <p id="discussion-cwd-error" className="work-send-hint" role="alert">{t("session.errors.invalidCwd")}</p>}

    <section className="discussion-section">
      <h3>{t("editor.moderator")}</h3>
      <p className="form-hint">{t("stanceHints.moderator")}</p>
      {draft.moderator
        ? row(draft.moderator, true, setModerator)
        : <p className="form-hint">{t("editor.noAgents")}</p>}
    </section>

    <section className="discussion-section">
      <h3>{t("editor.debaters", { count: draft.debaters.length, max: DISCUSSION_MAX_DEBATERS })}</h3>
      {draft.debaters.map((r, i) => row(r, false, (next) => setDebater(i, next), () => removeDebater(i)))}
      <button type="button" className="pixel-btn" disabled={!free.length || draft.debaters.length >= DISCUSSION_MAX_DEBATERS || !draft.moderator} onClick={addDebater}>{t("editor.addDebater")}</button>
    </section>

    <details className="discussion-section discussion-limit-options">
      <summary>{t("board.limits", {
        rounds: draft.limits.maxRounds || "–", minutes: draft.limits.maxMinutes || "–", cost: draft.limits.maxCostUsd || "–",
      })}</summary>
      <div className="discussion-limits">
        {limitInput("maxRounds", 1)}
        {limitInput("maxMinutes", 5)}
        {limitInput("maxCostUsd", 0.5)}
      </div>
      <label className="discussion-check"><input type="checkbox" checked={draft.limits.usageGuard} onChange={(e) => setLimit("usageGuard", e.target.checked)} /> {t("editor.usageGuard")}</label>
      <p className="form-hint">{t("editor.limitsHint")}</p>
    </details>
    <p className="form-hint">{t("editor.readOnlyHint")}</p>

    <div className="work-dialog-actions work-dialog-run">
      <button type="button" className="pixel-btn" onClick={onClose}>{t("editor.cancel")}</button>
      <span className="work-dialog-spacer" />
      <button type="button" className="pixel-btn primary" disabled={!!problem || invalidCwd || saving} aria-describedby={problem ? "discussion-problem" : undefined} onClick={() => void save()}>{t(base ? "editor.save" : "editor.create")}</button>
    </div>
    {problem && <p id="discussion-problem" className="work-send-hint">{t(`editor.problems.${problem}`, {
      rounds: `${maxRounds.min}–${maxRounds.max}`, minutes: `${maxMinutes.min}–${maxMinutes.max}`, cost: `${maxCostUsd.min}–${maxCostUsd.max}`, max: DISCUSSION_MAX_DEBATERS,
    })}</p>}
    {saveError && <p className="work-send-hint" role="alert">{t("editor.saveError")}</p>}
  </WorkDialog>;
}
