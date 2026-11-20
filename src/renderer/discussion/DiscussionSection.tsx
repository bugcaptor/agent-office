// src/renderer/discussion/DiscussionSection.tsx
//
// 업무 패널의 토의 카드 묶음과 토의 대화상자(열기·보기·고치기).
// 토의가 하나도 없으면 묶음은 숨기고 대화상자만 띄운다. 시작·멈춤·계속·끝내기와 사회자 질문의 답은
// 보기 대화상자에서 한다(진행은 discussionRun.ts). 정리가 끝나면 보기에서 결정 메모를 읽고(브라우저로 열기도) 결론을 확인한다.
import { useEffect, useMemo, useState, type MouseEvent } from "react";
import { openUrl } from "@tauri-apps/plugin-opener";
import { useTranslation } from "react-i18next";

import type { Discussion } from "@shared/types";
import { useAppStore } from "../store/appStore";
import { tauriApi } from "../ipc/tauriApi";
import { renderMarkdown } from "../markdown/renderMarkdown";
import { useWorkGroupStore } from "../workpanels/panelLayoutStore";
import { WorkDialog } from "../workpanels/WorkDialog";
import { AgentFace } from "../workpanels/AgentFace";
import { useRoleStore } from "./roleStore";
import { sortDiscussions, useDiscussionStore } from "./discussionStore";
import {
  discussionDeletable, discussionEditable, discussionElapsedMs, discussionTicking, isActiveDiscussion,
  participantRoleLabel, participantsModeratorFirst,
} from "./discussionModel";
import { useDiscussionSessionStore } from "./discussionSessionStore";
import { workAgentLabel } from "../profile/generate";
import { usageBlockText, useNow } from "../workpanels/AutoRunUsage";
import { DiscussionEditor } from "./DiscussionEditor";
import {
  answerDiscussion, confirmDiscussion, discussionStartBlockedReason, endDiscussion, pauseDiscussion, resumeDiscussion,
  startDiscussion,
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
    {dialog?.kind === "edit" && target && discussionEditable(target) && <DiscussionEditor key={target.id} base={target} onClose={() => openDialog({ kind: "view", id: target.id })} />}
    {dialog?.kind === "view" && target && <DiscussionDetail key={target.id} discussion={target} onClose={close} />}
  </>;
}

function limitsText(t: (key: string, options?: Record<string, unknown>) => string, d: Discussion): string {
  return t("board.limits", { rounds: d.limits.maxRounds, minutes: d.limits.maxMinutes, cost: d.limits.maxCostUsd });
}

/** 진행 라운드·진행 시간·추산 비용과 각 상한. 진행 중이면 시간이 흐르게 다시 그린다. */
function RunMeter({ discussion: d }: { discussion: Discussion }) {
  const { t } = useTranslation("discussion");
  const now = useNow(discussionTicking(d.state) ? 15_000 : 3_600_000);
  return <small className="discussion-run-meter">{t("board.meter", {
    round: d.round, maxRounds: d.limits.maxRounds,
    minutes: Math.floor(discussionElapsedMs(d, now) / 60_000), maxMinutes: d.limits.maxMinutes,
    cost: d.costUsd.toFixed(2), maxCost: d.limits.maxCostUsd,
  })}</small>;
}

/** 진행 중인 단계 한 줄. 시작 전·끝난 뒤에는 없다. */
function RunStatus({ discussion: d }: { discussion: Discussion }) {
  const { t } = useTranslation("discussion");
  const agents = useAppStore((s) => s.agents);
  const run = d.run;
  if (d.state === "review") return <small className="discussion-run-status">{t("run.phase.review")}</small>;
  if (!run) return null;
  const name = (id: string | undefined) => (id && agents[id]?.name) || t("board.missingAgent");
  let text: string;
  if (run.phase === "opening") text = t("run.phase.opening", { done: run.turns.filter((x) => x.text !== undefined).length, total: run.turns.length });
  else if (run.phase === "speakers") {
    const index = run.turns.findIndex((x) => x.text === undefined);
    text = t("run.phase.speakers", { name: name(run.turns[index]?.agentId), index: index + 1, total: run.turns.length });
  } else if (run.phase === "moderator") text = t("run.phase.moderator", { name: name(d.moderatorAgentId) });
  else if (run.phase === "wrap_up" && run.forced) text = t("run.phase.wrapForced", { reason: t(`run.limitName.${run.forced}`) });
  else text = t(`run.phase.${run.phase}`);
  return <small className="discussion-run-status">{text}</small>;
}

function DiscussionCard({ discussion: d, selected, onOpen }: { discussion: Discussion; selected: boolean; onOpen(): void }) {
  const { t } = useTranslation("discussion");
  const agents = useAppStore((s) => s.agents);
  const debaters = d.participants.filter((p) => p.agentId !== d.moderatorAgentId);
  // 사회자가 사람에게 묻거나 결론 확인을 기다리면 카드를 강조한다.
  const attention = d.state === "waiting_user" || d.state === "review";
  return <article className={`discussion-card state-${d.state}${attention ? " attention" : ""}${selected ? " selected" : ""}`}>
    <button type="button" className="discussion-card-heading" aria-haspopup="dialog" onClick={onOpen}>
      <span className="discussion-card-title">
        <span className={`work-status-chip discussion-state-chip state-${d.state}`}>{t(`states.${d.state}`)}</span>
        <strong>{d.topic}</strong>
      </span>
      <span className="discussion-card-faces" aria-label={t("board.participants", { names: d.participants.map((p) => agents[p.agentId]?.name ?? t("board.missingAgent")).join(", ") })}>
        <span className="discussion-card-moderator" title={t("board.moderatorOf", { name: agents[d.moderatorAgentId]?.name ?? t("board.missingAgent") })}><AgentFace agentId={d.moderatorAgentId} /></span>
        {debaters.map((p) => <span key={p.agentId} title={agents[p.agentId]?.name ?? t("board.missingAgent")}><AgentFace agentId={p.agentId} /></span>)}
      </span>
      {d.state === "ready" ? <small>{limitsText(t, d)}</small> : <RunMeter discussion={d} />}
      <RunStatus discussion={d} />
      <ReadOnlyBadge />
    </button>
  </article>;
}

/** 읽기 전용은 CLI 권한이 아니라 프롬프트 규칙으로 지킨다. */
function ReadOnlyBadge() {
  const { t } = useTranslation("discussion");
  return <small className="discussion-readonly" title={t("readOnly.hint")}>🔒 {t("readOnly.label")}</small>;
}

function DiscussionDetail({ discussion: d, onClose }: { discussion: Discussion; onClose(): void }) {
  const { t } = useTranslation("discussion");
  const agents = useAppStore((s) => s.agents);
  const roles = useRoleStore((s) => s.roles);
  const openDialog = useDiscussionStore((s) => s.openDialog);
  const [confirming, setConfirming] = useState(false);
  const [failed, setFailed] = useState(false);
  // 준비 상태에서 눌러 둔 삭제 확인이 진행을 지나 끝난 뒤 한 번의 클릭으로 지우지 않게, 상태가 바뀌면 되돌린다.
  useEffect(() => { setConfirming(false); }, [d.state]);
  const remove = async () => {
    setFailed(false);
    try { await useDiscussionStore.getState().remove(d.id); } catch { setFailed(true); }
  };
  return <WorkDialog title={d.topic} id={d.id} onClose={onClose} className="discussion-detail">
    <p><span className={`work-status-chip discussion-state-chip state-${d.state}`}>{t(`states.${d.state}`)}</span>
      {d.state !== "ready" && <RunMeter discussion={d} />}</p>
    <RunStatus discussion={d} />
    {d.context && <p className="discussion-context">{d.context}</p>}
    <RunPanel discussion={d} />
    {d.decisionMemoPath && <MemoPanel discussion={d} />}
    <dl className="discussion-facts">
      <dt>{t("detail.cwd")}</dt><dd>{d.cwd ?? t("detail.homeFolder")}</dd>
      <dt>{t("detail.limits")}</dt><dd>{limitsText(t, d)}{d.limits.usageGuard ? ` · ${t("detail.usageGuard")}` : ""}</dd>
      <dt>{t("detail.cost")}</dt><dd>{t("detail.costValue", { cost: d.costUsd.toFixed(2) })}</dd>
      <dt>{t("detail.memo")}</dt><dd>{d.decisionMemoPath ?? t("detail.memoNone")}</dd>
      <dt>{t("detail.readOnly")}</dt><dd><ReadOnlyBadge /></dd>
    </dl>
    <h3 className="discussion-subtitle">{t("detail.participants")}</h3>
    <ul className="discussion-people">
      {participantsModeratorFirst(d).map((p) => <li key={p.agentId}>
        <AgentFace agentId={p.agentId} />
        <span><strong>{agents[p.agentId]?.name ?? t("board.missingAgent")}</strong>
          {p.agentId === d.moderatorAgentId && <small className="discussion-moderator-badge">{t("detail.moderator")}</small>}
          <small>{participantRoleLabel(p.roleId, roles)} · {workAgentLabel(p.agent)}</small></span>
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
  // 시작 막힘 판정이 살아 있는 토의 탭을 본다. 앞 토의를 끝낸 뒤 탭이 닫히면 다시 그린다.
  useDiscussionSessionStore((s) => s.sessions);
  const [answer, setAnswer] = useState("");
  const [ending, setEnding] = useState(false);
  const [error, setError] = useState<string>();
  const run = d.run;
  const blocked = d.state === "ready" ? discussionStartBlockedReason(d, discussions, agents, observerEnabled) : undefined;
  const fail = (code: string | undefined) => setError(code);
  const active = isActiveDiscussion(d) && !run?.pendingMemo;
  const usageWaiting = d.state === "paused" && run?.pauseReason === "usage-wait";
  return <div className="discussion-run">
    {d.state === "paused" && run?.pauseReason && <p className="discussion-run-paused" role="status">
      <strong>{t("run.pausedBecause")}</strong> {t(`run.pause.${run.pauseReason}`)}{run.pauseDetail ? ` (${run.pauseDetail})` : ""}
      {usageWaiting && run.usageWait && <UsageWaitLine discussion={d} />}
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
    {d.state === "review" && <div className="work-dialog-actions discussion-run-actions">
      <span className="work-send-hint">{t("detail.reviewHint")}</span>
      <span className="work-dialog-spacer" />
      <button type="button" className="pixel-btn primary" onClick={() => confirmDiscussion(d.id)}>{t("run.actions.confirm")}</button>
    </div>}
    {(d.state === "ready" || active) && <div className="work-dialog-actions discussion-run-actions">
      {d.state === "ready" && <button type="button" className="pixel-btn primary" disabled={!!blocked} title={blocked ? t(`run.blocked.${blocked}`) : undefined}
        onClick={() => fail(startDiscussion(d.id))}>{t("run.actions.start")}</button>}
      {(d.state === "running" || d.state === "wrapping_up" || usageWaiting) && <button type="button" className="pixel-btn" onClick={() => pauseDiscussion(d.id)}>{t("run.actions.pause")}</button>}
      {d.state === "paused" && <button type="button" className="pixel-btn primary" onClick={() => fail(resumeDiscussion(d.id))}>{t(usageWaiting ? "run.actions.skipUsageWait" : "run.actions.resume")}</button>}
      {active && (ending
        ? <button type="button" className="pixel-btn danger" onClick={() => { setEnding(false); void endDiscussion(d.id); }}>{t("run.actions.confirmEnd")}</button>
        : <button type="button" className="pixel-btn danger" onClick={() => setEnding(true)}>{t("run.actions.end")}</button>)}
    </div>}
    {d.state === "ready" && blocked && <p className="work-send-hint">{t(`run.blocked.${blocked}`)}</p>}
    {error && <p className="work-send-hint" role="alert">{t(`run.blocked.${error}`)}</p>}
  </div>;
}

/** 사용량 대기 이유와 언제 이어 가는지(자동 진행과 같은 문장). */
function UsageWaitLine({ discussion: d }: { discussion: Discussion }) {
  const { t } = useTranslation("discussion");
  const { t: tWork } = useTranslation("workpanels");
  const { t: tActivity } = useTranslation("activity");
  const agents = useAppStore((s) => s.agents);
  const now = useNow(30_000);
  const wait = d.run?.usageWait;
  if (!wait) return null;
  const stage = t("run.usageStage", { name: agents[wait.agentId]?.name ?? t("board.missingAgent") });
  return <span className="discussion-run-usage"> {usageBlockText(tWork, tActivity, wait, stage, now)}</span>;
}

/** 결정 메모 보기와 브라우저로 열기. 메모는 에이전트가 쓴 글이라 거른 HTML만 넣고, 링크는 웹뷰가 아니라 OS로 연다. */
function MemoPanel({ discussion: d }: { discussion: Discussion }) {
  const { t } = useTranslation("discussion");
  // undefined = 읽는 중, null = 없거나 못 읽음.
  const [memo, setMemo] = useState<string | null | undefined>(undefined);
  const [openFailed, setOpenFailed] = useState(false);
  useEffect(() => {
    let alive = true;
    setMemo(undefined);
    tauriApi.loadDiscussionMemo(d.id).then((text) => { if (alive) setMemo(text); }, () => { if (alive) setMemo(null); });
    return () => { alive = false; };
  }, [d.id, d.decisionMemoPath]);
  const html = useMemo(() => (memo ? renderMarkdown(memo) : ""), [memo]);
  const open = () => {
    setOpenFailed(false);
    tauriApi.openDiscussionMemo(d.id).catch((error: unknown) => { console.warn("discussion: memo open failed", error); setOpenFailed(true); });
  };
  const onClick = (e: MouseEvent<HTMLDivElement>) => {
    const anchor = (e.target as HTMLElement).closest("a");
    if (!anchor) return;
    e.preventDefault();
    const href = anchor.getAttribute("href") ?? "";
    if (/^https?:\/\//i.test(href)) void openUrl(href).catch((error: unknown) => console.warn("discussion: memo link open failed", error));
  };
  return <section className="discussion-memo">
    <div className="discussion-memo-head">
      <h3 className="discussion-subtitle">{t("detail.memo")}</h3>
      <span className="work-dialog-spacer" />
      <button type="button" className="pixel-btn" disabled={!memo} onClick={open}>{t("run.actions.openMemo")}</button>
    </div>
    {memo === undefined ? <p className="work-send-hint">{t("detail.memoLoading")}</p>
      : memo === null ? <p className="work-send-hint" role="alert">{t("detail.memoMissing")}</p>
      // marked+DOMPurify로 거른 HTML만 넣는다.
      : <div className="discussion-memo-body md-editor-preview" onClick={onClick} dangerouslySetInnerHTML={{ __html: html }} />}
    {openFailed && <p className="work-send-hint" role="alert">{t("detail.memoOpenError")}</p>}
  </section>;
}
