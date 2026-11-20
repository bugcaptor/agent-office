// src/renderer/discussion/DiscussionSection.tsx
//
// 업무 패널의 토의 카드 묶음과 토의 대화상자(열기·보기·고치기).
// 토의가 하나도 없으면 묶음은 숨기고 대화상자만 띄운다. 시작·멈춤·계속·끝내기와 사회자 질문의 답은
// 보기 대화상자에서 한다(진행은 discussionRun.ts).
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";

import type { Discussion } from "@shared/types";
import { useAppStore } from "../store/appStore";
import { useWorkGroupStore } from "../workpanels/panelLayoutStore";
import { WorkDialog } from "../workpanels/WorkDialog";
import { AgentFace } from "../workpanels/AgentFace";
import { useRoleStore } from "./roleStore";
import { sortDiscussions, useDiscussionStore } from "./discussionStore";
import { discussionDeletable, discussionEditable, discussionReadOnlySummary } from "./discussionModel";
import { discussionReadOnlyMode } from "@shared/types";
import { DiscussionEditor } from "./DiscussionEditor";
import {
  answerDiscussion, discussionStartBlockedReason, endDiscussion, pauseDiscussion, resumeDiscussion, startDiscussion,
} from "./discussionRun";
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

/** 진행 중인 단계 한 줄. 시작 전·끝난 뒤에는 없다. */
function RunStatus({ discussion: d }: { discussion: Discussion }) {
  const { t } = useTranslation("discussion");
  const agents = useAppStore((s) => s.agents);
  const run = d.run;
  if (!run) return null;
  const name = (id: string | undefined) => (id && agents[id]?.name) || t("board.missingAgent");
  let text: string;
  if (run.phase === "opening") text = t("run.phase.opening", { done: run.turns.filter((x) => x.text !== undefined).length, total: run.turns.length });
  else if (run.phase === "speakers") {
    const index = run.turns.findIndex((x) => x.text === undefined);
    text = t("run.phase.speakers", { name: name(run.turns[index]?.agentId), index: index + 1, total: run.turns.length });
  } else if (run.phase === "moderator") text = t("run.phase.moderator", { name: name(d.moderatorAgentId) });
  else text = t(`run.phase.${run.phase}`);
  return <small className="discussion-run-status">{text}</small>;
}

function DiscussionCard({ discussion: d, selected, onOpen }: { discussion: Discussion; selected: boolean; onOpen(): void }) {
  const { t } = useTranslation("discussion");
  const agents = useAppStore((s) => s.agents);
  const debaters = d.participants.filter((p) => p.agentId !== d.moderatorAgentId);
  // 사회자가 사람에게 물으면 카드를 강조한다.
  return <article className={`discussion-card state-${d.state}${d.state === "waiting_user" ? " attention" : ""}${selected ? " selected" : ""}`}>
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
      <RunStatus discussion={d} />
      <ReadOnlyBadge discussion={d} />
    </button>
  </article>;
}

/** 읽기 전용을 CLI가 강제하는지, 프롬프트로만 제한하는지. 참가자 중 가장 약한 쪽을 보인다. */
function ReadOnlyBadge({ discussion }: { discussion: Discussion }) {
  const { t } = useTranslation("discussion");
  const mode = discussionReadOnlySummary(discussion);
  return <small className={`discussion-readonly readonly-${mode}`} title={t(`readOnly.hint.${mode}`)}>🔒 {t(`readOnly.summary.${mode}`)}</small>;
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
    <RunStatus discussion={d} />
    {d.context && <p className="discussion-context">{d.context}</p>}
    <RunPanel discussion={d} />
    <dl className="discussion-facts">
      <dt>{t("detail.cwd")}</dt><dd>{d.cwd ?? t("detail.homeFolder")}</dd>
      <dt>{t("detail.limits")}</dt><dd>{limitsText(t, d)}{d.limits.usageGuard ? ` · ${t("detail.usageGuard")}` : ""}</dd>
      <dt>{t("detail.cost")}</dt><dd>{t("detail.costValue", { cost: d.costUsd.toFixed(2) })}</dd>
      <dt>{t("detail.memo")}</dt><dd>{d.decisionMemoPath ?? t("detail.memoNone")}</dd>
      <dt>{t("detail.readOnly")}</dt><dd><ReadOnlyBadge discussion={d} /></dd>
    </dl>
    <h3 className="discussion-subtitle">{t("detail.participants")}</h3>
    <ul className="discussion-people">
      {ordered.map((p) => <li key={p.agentId}>
        <AgentFace agentId={p.agentId} />
        <span><strong>{agents[p.agentId]?.name ?? t("board.missingAgent")}</strong>
          {p.agentId === d.moderatorAgentId && <small className="discussion-moderator-badge">{t("detail.moderator")}</small>}
          <small>{roleName(p.roleId)} · {[p.agent.cli, p.agent.model, p.agent.effort].filter(Boolean).join(" · ")}
            {" · "}<span className={`discussion-readonly readonly-${discussionReadOnlyMode(p.agent.cli)}`}>{t(`readOnly.mode.${discussionReadOnlyMode(p.agent.cli)}`)}</span></small></span>
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

/** 진행 조작(시작·멈춤·계속·끝내기), 멈춘 이유, 사회자 질문과 답 입력. */
function RunPanel({ discussion: d }: { discussion: Discussion }) {
  const { t } = useTranslation("discussion");
  const agents = useAppStore((s) => s.agents);
  const observerEnabled = useAppStore((s) => s.appSettings.observerEnabled);
  const discussions = useDiscussionStore((s) => s.discussions);
  const [answer, setAnswer] = useState("");
  const [ending, setEnding] = useState(false);
  const [error, setError] = useState<string>();
  const run = d.run;
  const blocked = d.state === "ready" ? discussionStartBlockedReason(d, discussions, agents, observerEnabled) : undefined;
  const fail = (code: string | undefined) => setError(code);
  const active = d.state !== "ready" && d.state !== "finished";
  return <div className="discussion-run">
    {d.state === "paused" && run?.pauseReason && <p className="discussion-run-paused" role="status">
      <strong>{t("run.pausedBecause")}</strong> {t(`run.pause.${run.pauseReason}`)}{run.pauseDetail ? ` (${run.pauseDetail})` : ""}
    </p>}
    {d.state === "waiting_user" && run?.question && <form className="discussion-run-question" onSubmit={(e) => {
      e.preventDefault();
      if (answerDiscussion(d.id, answer)) setAnswer("");
    }}>
      <strong>{t("run.question")}</strong>
      <p>{run.question}</p>
      <label>{t("run.answerLabel")}
        <textarea value={answer} rows={3} placeholder={t("run.answerPlaceholder")} onChange={(e) => setAnswer(e.target.value)} />
      </label>
      <button type="submit" className="pixel-btn primary" disabled={!answer.trim()}>{t("run.actions.answer")}</button>
    </form>}
    {(d.state === "ready" || active) && <div className="work-dialog-actions discussion-run-actions">
      {d.state === "ready" && <button type="button" className="pixel-btn primary" disabled={!!blocked} title={blocked ? t(`run.blocked.${blocked}`) : undefined}
        onClick={() => fail(startDiscussion(d.id))}>{t("run.actions.start")}</button>}
      {(d.state === "running" || d.state === "wrapping_up") && <button type="button" className="pixel-btn" onClick={() => pauseDiscussion(d.id)}>{t("run.actions.pause")}</button>}
      {d.state === "paused" && <button type="button" className="pixel-btn primary" onClick={() => fail(resumeDiscussion(d.id))}>{t("run.actions.resume")}</button>}
      {active && (ending
        ? <button type="button" className="pixel-btn danger" onClick={() => { setEnding(false); void endDiscussion(d.id); }}>{t("run.actions.confirmEnd")}</button>
        : <button type="button" className="pixel-btn danger" onClick={() => setEnding(true)}>{t("run.actions.end")}</button>)}
    </div>}
    {d.state === "ready" && blocked && <p className="work-send-hint">{t(`run.blocked.${blocked}`)}</p>}
    {error && <p className="work-send-hint" role="alert">{t(`run.blocked.${error}`)}</p>}
  </div>;
}
