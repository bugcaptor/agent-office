// src/renderer/discussion/DiscussionSection.tsx
//
// 업무 패널의 토의 카드 묶음과 토의 대화상자(열기·보기·고치기).
// 토의가 하나도 없으면 묶음은 숨기고 대화상자만 띄운다. 실행(시작)은 3·4단계에서 붙는다.
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";

import type { Discussion } from "@shared/types";
import { useAppStore } from "../store/appStore";
import { useWorkGroupStore } from "../workpanels/panelLayoutStore";
import { WorkDialog } from "../workpanels/WorkDialog";
import { AgentFace } from "../workpanels/AgentFace";
import { useRoleStore } from "./roleStore";
import { sortDiscussions, useDiscussionStore } from "./discussionStore";
import { discussionDeletable, discussionEditable } from "./discussionModel";
import { DiscussionEditor } from "./DiscussionEditor";
import "./discussion.css";

/** 접힘 상태 키(캐릭터 묶음과 같은 저장소). */
const GROUP_KEY = "discussions";

export function DiscussionSection() {
  const { t } = useTranslation("discussion");
  const discussions = useDiscussionStore((s) => s.discussions);
  const status = useDiscussionStore((s) => s.status);
  const dialog = useDiscussionStore((s) => s.dialog);
  const openDialog = useDiscussionStore((s) => s.openDialog);
  const collapsed = useWorkGroupStore((s) => s.collapsed.includes(GROUP_KEY));
  const setCollapsed = useWorkGroupStore((s) => s.setCollapsed);
  useEffect(() => { void useDiscussionStore.getState().ensureLoaded(); }, []);
  // 카드의 역할 이름을 보이려면 역할 목록이 있어야 한다.
  useEffect(() => { if (discussions.length) void useRoleStore.getState().ensureLoaded(); }, [discussions.length]);

  const close = () => openDialog(null);
  const target = dialog && dialog.kind !== "new" ? discussions.find((d) => d.id === dialog.id) : undefined;
  return <>
    {status === "error" && <div role="alert">{t("board.loadError")}{" "}
      <button className="pixel-btn" onClick={() => void useDiscussionStore.getState().load()}>{t("roles.retry")}</button></div>}
    {discussions.length > 0 && <details className="work-group discussion-group" open={!collapsed} onToggle={(e) => setCollapsed(GROUP_KEY, !e.currentTarget.open)}>
      <summary><strong>{t("board.title")}</strong> <span className="work-group-count">{discussions.length}</span></summary>
      {sortDiscussions(discussions).map((d) => <DiscussionCard key={d.id} discussion={d} selected={target?.id === d.id} onOpen={() => openDialog({ kind: "view", id: d.id })} />)}
    </details>}
    {dialog?.kind === "new" && <DiscussionEditor onClose={close} />}
    {dialog?.kind === "edit" && target && <DiscussionEditor key={target.id} base={target} onClose={() => openDialog({ kind: "view", id: target.id })} />}
    {dialog?.kind === "view" && target && <DiscussionDetail key={target.id} discussion={target} onClose={close} />}
  </>;
}

function limitsText(t: (key: string, options?: Record<string, unknown>) => string, d: Discussion): string {
  return t("board.limits", { rounds: d.limits.maxRounds, minutes: d.limits.maxMinutes, cost: d.limits.maxCostUsd });
}

function DiscussionCard({ discussion: d, selected, onOpen }: { discussion: Discussion; selected: boolean; onOpen(): void }) {
  const { t } = useTranslation("discussion");
  const agents = useAppStore((s) => s.agents);
  const debaters = d.participants.filter((p) => p.agentId !== d.moderatorAgentId);
  return <article className={`discussion-card state-${d.state}${selected ? " selected" : ""}`}>
    <button type="button" className="discussion-card-heading" aria-haspopup="dialog" onClick={onOpen}>
      <span className="discussion-card-title">
        <span className={`work-status-chip discussion-state-chip state-${d.state}`}>{t(`states.${d.state}`)}</span>
        <strong>{d.topic}</strong>
      </span>
      <span className="discussion-card-faces" aria-label={t("board.participants", { names: d.participants.map((p) => agents[p.agentId]?.name ?? t("board.missingAgent")).join(", ") })}>
        <span className="discussion-card-moderator" title={t("board.moderatorOf", { name: agents[d.moderatorAgentId]?.name ?? t("board.missingAgent") })}><AgentFace agentId={d.moderatorAgentId} /></span>
        {debaters.map((p) => <span key={p.agentId} title={agents[p.agentId]?.name ?? t("board.missingAgent")}><AgentFace agentId={p.agentId} /></span>)}
      </span>
      <small>{d.state === "ready" ? limitsText(t, d) : t("board.progress", { round: d.round, max: d.limits.maxRounds })}</small>
    </button>
  </article>;
}

function DiscussionDetail({ discussion: d, onClose }: { discussion: Discussion; onClose(): void }) {
  const { t } = useTranslation("discussion");
  const agents = useAppStore((s) => s.agents);
  const roles = useRoleStore((s) => s.roles);
  const openDialog = useDiscussionStore((s) => s.openDialog);
  const [confirming, setConfirming] = useState(false);
  const [failed, setFailed] = useState(false);
  const roleName = (roleId?: string) => !roleId ? t("board.noRole") : roles.find((r) => r.id === roleId)?.name ?? t("board.missingRole");
  const remove = async () => {
    setFailed(false);
    try { await useDiscussionStore.getState().remove(d.id); } catch { setFailed(true); }
  };
  const ordered = [...d.participants].sort((a, b) => Number(b.agentId === d.moderatorAgentId) - Number(a.agentId === d.moderatorAgentId));
  return <WorkDialog title={d.topic} id={d.id} onClose={onClose} className="discussion-detail">
    <p><span className={`work-status-chip discussion-state-chip state-${d.state}`}>{t(`states.${d.state}`)}</span>
      {d.state !== "ready" && t("board.progress", { round: d.round, max: d.limits.maxRounds })}</p>
    {d.context && <p className="discussion-context">{d.context}</p>}
    <dl className="discussion-facts">
      <dt>{t("detail.cwd")}</dt><dd>{d.cwd ?? t("detail.homeFolder")}</dd>
      <dt>{t("detail.limits")}</dt><dd>{limitsText(t, d)}{d.limits.usageGuard ? ` · ${t("detail.usageGuard")}` : ""}</dd>
      <dt>{t("detail.cost")}</dt><dd>{t("detail.costValue", { cost: d.costUsd.toFixed(2) })}</dd>
      <dt>{t("detail.memo")}</dt><dd>{d.decisionMemoPath ?? t("detail.memoNone")}</dd>
    </dl>
    <h3 className="discussion-subtitle">{t("detail.participants")}</h3>
    <ul className="discussion-people">
      {ordered.map((p) => <li key={p.agentId}>
        <AgentFace agentId={p.agentId} />
        <span><strong>{agents[p.agentId]?.name ?? t("board.missingAgent")}</strong>
          {p.agentId === d.moderatorAgentId && <small className="discussion-moderator-badge">{t("detail.moderator")}</small>}
          <small>{roleName(p.roleId)} · {[p.agent.cli, p.agent.model, p.agent.effort].filter(Boolean).join(" · ")}</small></span>
      </li>)}
    </ul>
    <div className="work-dialog-actions work-dialog-run">
      {discussionDeletable(d) && (confirming
        ? <button type="button" className="pixel-btn danger" onClick={() => void remove()}>{t("detail.confirmDelete")}</button>
        : <button type="button" className="pixel-btn danger" onClick={() => setConfirming(true)}>{t("detail.delete")}</button>)}
      <span className="work-dialog-spacer" />
      {discussionEditable(d) && <button type="button" className="pixel-btn" onClick={() => openDialog({ kind: "edit", id: d.id })}>{t("detail.edit")}</button>}
    </div>
    {failed && <p className="work-send-hint" role="alert">{t("detail.deleteError")}</p>}
  </WorkDialog>;
}
