// src/renderer/discussion/DiscussionTranscript.tsx
//
// 토의 보기 창의 속기록 탭. 기록을 라운드별로 시간 순으로 보여 주고, 진행 중이면 새 발언이 아래에 붙는다
// (맨 아래를 보고 있을 때만 따라 내려간다). 긴 발언은 앞부분만 보이고 펼칠 수 있다. 종류·화자로 거르고,
// 내보내기는 필터와 상관없이 전체를 임시 HTML로 브라우저에 연다. 회의록 보드에서 들어오면 그 발언으로 스크롤한다.
import { useEffect, useLayoutEffect, useMemo, useRef, useState, type MouseEvent } from "react";
import { openUrl } from "@tauri-apps/plugin-opener";
import { useTranslation } from "react-i18next";

import type { Discussion, DiscussionUtterance } from "@shared/types";
import { useParticipantLabel } from "./participants";
import { tauriApi } from "../ipc/tauriApi";
import { renderMarkdown } from "../markdown/renderMarkdown";
import { useNow } from "../workpanels/AutoRunUsage";
import { useRoleStore } from "./roleStore";
import { participantsModeratorFirst } from "./discussionModel";
import { speakingSince } from "./officeStage";
import {
  TRANSCRIPT_GROUPS, clockTime, composeTranscriptMarkdown, isLongUtterance, openingProgress, speakerName, speakerRole,
  transcriptGroup, transcriptRows, type TranscriptGroup,
} from "./transcriptModel";

/** 맨 아래에서 이 거리(px) 안이면 새 발언을 따라 내려간다. */
const STICK_PX = 48;
const FOCUS_FLASH_MS = 2000;

/** 에이전트가 쓴 글 안의 링크는 웹뷰가 아니라 OS로 연다(http·https만). */
export function openMarkdownLink(e: MouseEvent<HTMLElement>): void {
  const anchor = (e.target as HTMLElement).closest("a");
  if (!anchor) return;
  e.preventDefault();
  const href = anchor.getAttribute("href") ?? "";
  if (/^https?:\/\//i.test(href)) void openUrl(href).catch((error: unknown) => console.warn("discussion: link open failed", error));
}

export function DiscussionTranscript({ discussion: d, focus }: { discussion: Discussion; focus?: number }) {
  const { t } = useTranslation("discussion");
  const label = useParticipantLabel();
  // 역할 이름이 바뀌면 다시 그린다.
  useRoleStore((s) => s.roles);
  const [groups, setGroups] = useState<ReadonlySet<TranscriptGroup>>(() => new Set(TRANSCRIPT_GROUPS));
  const [speaker, setSpeaker] = useState<string>();
  const [exportFailed, setExportFailed] = useState(false);
  const rows = useMemo(() => transcriptRows(d.transcript, { groups, speaker }), [d.transcript, groups, speaker]);
  const listRef = useRef<HTMLOListElement>(null);
  const stick = useRef(true);
  const [flash, setFlash] = useState<number | undefined>(focus);

  // 처음: 가리킨 발언이 있으면 그리로, 아니면 맨 아래로.
  useLayoutEffect(() => {
    const list = listRef.current;
    if (!list) return;
    const target = focus !== undefined ? list.querySelector<HTMLElement>(`[data-index="${focus}"]`) : null;
    if (target) {
      // 목록이 offsetParent다(position: relative).
      list.scrollTop = Math.max(0, target.offsetTop - 4);
      stick.current = false;
    } else list.scrollTop = list.scrollHeight;
  }, [focus]);
  useEffect(() => {
    if (flash === undefined) return;
    const timer = window.setTimeout(() => setFlash(undefined), FOCUS_FLASH_MS);
    return () => window.clearTimeout(timer);
  }, [flash]);
  // 새 발언: 맨 아래를 보고 있었으면 따라 내려간다.
  useLayoutEffect(() => {
    const list = listRef.current;
    if (list && stick.current) list.scrollTop = list.scrollHeight;
  }, [d.transcript.length, rows.length]);

  const toggleGroup = (g: TranscriptGroup) => setGroups((current) => {
    const next = new Set(current);
    if (next.has(g)) next.delete(g); else next.add(g);
    return next;
  });
  const exportAll = () => {
    setExportFailed(false);
    tauriApi.openDiscussionTranscript(d.id, composeTranscriptMarkdown(d)).catch((error: unknown) => {
      console.warn("discussion: transcript export failed", error);
      setExportFailed(true);
    });
  };
  const people = [...new Set(participantsModeratorFirst(d).map((p) => p.agentId))];

  return <section className="discussion-transcript" aria-label={t("transcript.tabTranscript")}>
    <div className="discussion-transcript-tools">
      <div className="discussion-transcript-chips" role="group" aria-label={t("transcript.kindFilter")}>
        {TRANSCRIPT_GROUPS.map((g) => <button key={g} type="button" className={`pixel-btn transcript-chip group-${g}`}
          aria-pressed={groups.has(g)} onClick={() => toggleGroup(g)}>{t(`transcript.group.${g}`)}</button>)}
      </div>
      <span className="work-dialog-spacer" />
      <button type="button" className="pixel-btn" title={t("transcript.exportHint")} disabled={!d.transcript.length} onClick={exportAll}>{t("transcript.export")}</button>
    </div>
    <div className="discussion-transcript-chips" role="group" aria-label={t("transcript.speakerFilter")}>
      <button type="button" className="pixel-btn transcript-chip" aria-pressed={speaker === undefined} onClick={() => setSpeaker(undefined)}>{t("transcript.speakerAll")}</button>
      {people.map((id) => <button key={id} type="button" className="pixel-btn transcript-chip" aria-pressed={speaker === id}
        onClick={() => setSpeaker(speaker === id ? undefined : id)}>{label(id)}</button>)}
    </div>
    {exportFailed && <p className="work-send-hint" role="alert">{t("transcript.exportError")}</p>}
    <ol className="discussion-transcript-list" ref={listRef} onScroll={(e) => {
      const el = e.currentTarget;
      stick.current = el.scrollHeight - el.scrollTop - el.clientHeight < STICK_PX;
    }}>
      {rows.length === 0 && <li className="transcript-empty">{t(d.transcript.length ? "transcript.emptyFiltered" : "transcript.empty")}</li>}
      {rows.flatMap((row) => [
        ...(row.roundStart ? [<li key={`r${row.index}`} className="transcript-round" aria-hidden="true">{t("transcript.round", { round: row.utterance.round })}</li>] : []),
        <TranscriptEntry key={row.index} discussion={d} index={row.index} utterance={row.utterance} flash={flash === row.index} />,
      ])}
      <TranscriptLive discussion={d} />
    </ol>
  </section>;
}

function TranscriptEntry({ discussion: d, index, utterance: u, flash }: { discussion: Discussion; index: number; utterance: DiscussionUtterance; flash: boolean }) {
  const { t } = useTranslation("discussion");
  const html = useMemo(() => renderMarkdown(u.text), [u.text]);
  const long = useMemo(() => isLongUtterance(u.text), [u.text]);
  const [open, setOpen] = useState(false);
  const moderator = !!u.agentId && u.agentId === d.moderatorAgentId;
  const role = moderator ? undefined : speakerRole(d, u.agentId);
  return <li data-index={index} className={`transcript-entry kind-${u.kind} group-${transcriptGroup(u.kind)}${flash ? " focused" : ""}`}>
    <div className="transcript-entry-head">
      <strong>{speakerName(u)}</strong>
      {moderator && <small className="discussion-moderator-badge">{t("detail.moderator")}</small>}
      {role && <small>{role}</small>}
      <span className="transcript-kind">{t(`transcript.kind.${u.kind}`)}</span>
      <span className="work-dialog-spacer" />
      <time dateTime={new Date(u.at).toISOString()}>{clockTime(u.at)}</time>
    </div>
    {/* marked+DOMPurify로 거른 HTML만 넣는다. */}
    <div className={`transcript-entry-body md-editor-preview${long && !open ? " clamped" : ""}`} onClick={openMarkdownLink}
      dangerouslySetInnerHTML={{ __html: html }} />
    {long && <button type="button" className="transcript-more" aria-expanded={open} onClick={() => setOpen(!open)}>
      {t(open ? "transcript.less" : "transcript.more")}</button>}
  </li>;
}

/** 진행 중 표시: 비공개 첫 라운드의 제출 수와 말하는 중인 참가자. */
function TranscriptLive({ discussion: d }: { discussion: Discussion }) {
  const { t } = useTranslation("discussion");
  const label = useParticipantLabel();
  const now = useNow(30_000);
  const pending = openingProgress(d);
  const speaking = [...speakingSince(d)];
  if (!pending && !speaking.length) return null;
  return <li className="transcript-live" role="status">
    {pending && <span>{t("transcript.openingPending", pending)}</span>}
    {speaking.map(([agentId, since]) => {
      const name = label(agentId);
      const minutes = Math.floor(Math.max(0, now - since) / 60_000);
      return <span key={agentId}>{minutes < 1 ? t("transcript.speaking", { name }) : t("transcript.speakingFor", { name, minutes })}</span>;
    })}
  </li>;
}
