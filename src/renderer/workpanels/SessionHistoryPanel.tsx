import { useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import type { SessionEventRecord } from "@shared/types";
import { tauriApi } from "../ipc/tauriApi";
import { useSessionLogStore } from "../sessionlog/sessionLogStore";
import { formatDuration } from "../timeline/format";
import { formatTokens, formatUsd } from "../analytics/pricing";
import { useSessionSummaryStore, SUMMARY_RETENTION_MS } from "./sessionSummaryStore";
import { useWorkItemStore } from "./workItemStore";
import { usePanelLayoutStore } from "./panelLayoutStore";
import { buildSessionRows, filterSessionRows, folderName, type SessionRow } from "./sessionRows";
import { WorkPanelFrame } from "./WorkPanelFrame";
import { AgentFace } from "./AgentFace";

function SessionDetails({ row }: { row: SessionRow }) {
  const { t } = useTranslation("workpanels");
  const [log, setLog] = useState<{ path: string; index: number } | null>(null);
  useEffect(() => {
    let cancelled = false;
    async function findLog() {
      if ((row.endedAt ?? Date.now()) < Date.now() - 30 * 86_400_000) return;
      for (let offset = 0; !cancelled; offset += 100) {
        const page = await tauriApi.listSessionLogs(row.agentId, offset, 100);
        if (cancelled) return;
        const index = page.items.findIndex((i) => i.sessionId === row.sessionId);
        if (index >= 0) { setLog({ path: page.items[index].path, index: offset + index }); return; }
        if (!page.items.length || offset + page.items.length >= page.total) return;
      }
    }
    void findLog().catch((err) => console.warn("work-panels: transcript lookup failed", err));
    return () => { cancelled = true; };
  }, [row.agentId, row.sessionId, row.endedAt]);
  const time = (at: number) => new Date(at).toLocaleTimeString();
  return <div className="session-history-detail">
    <strong>{t("prompts")}</strong>
    {!row.prompts.length && <p className="work-muted">{t("noPrompts")}</p>}
    <ol>{row.prompts.map((p, n) => <li key={`${p.at}-${n}`}><small>{time(p.at)} {p.origin && t("origin", { origin: p.origin })}</small><p>{p.text}</p></li>)}</ol>
    <strong>{t("turnsDetail")}</strong>
    <ol>{row.turns.map((turn, n) => <li key={`${turn.startAt}-${n}`}>{time(turn.startAt)} · {formatDuration(turn.endAt - turn.startAt)}</li>)}</ol>
    <strong>{t("usageDetail")}</strong>
    <ul>{row.usage.map((usage, n) => <li key={`${usage.model}-${n}`}>{usage.model ?? t("unknownModel")} · {t("tokens", { tokens: formatTokens(usage.tokens) })} · {formatUsd(usage.costUsd)}{usage.unknown && "*"}</li>)}</ul>
    {row.workItemId && row.workTitle && <button className="pixel-btn" onClick={() => {
      usePanelLayoutStore.getState().update("left", { collapsed: false });
      useWorkItemStore.getState().select(row.workItemId!);
      requestAnimationFrame(() => {
        const card = document.getElementById(`work-item-${row.workItemId}`);
        const group = card?.closest("details");
        if (group) group.open = true;
        card?.scrollIntoView({ block: "nearest" });
      });
    }}>{row.workTitle}</button>}
    {log && <button className="pixel-btn" onClick={async () => {
      const store = useSessionLogStore.getState();
      store.open(row.agentId, row.agentName);
      await store.setPage(Math.floor(log.index / 10));
      if (useSessionLogStore.getState().overlay?.agentId === row.agentId) store.select(log.path);
    }}>{t("viewTranscript")}</button>}
  </div>;
}
export function SessionHistoryPanel() {
  const { t } = useTranslation("workpanels");
  const { summaries, liveIds, error: summaryError } = useSessionSummaryStore();
  const items = useWorkItemStore((s) => s.items);
  const [events, setEvents] = useState<SessionEventRecord[]>([]);
  const [now, setNow] = useState(Date.now);
  const [query, setQuery] = useState("");
  const [period, setPeriod] = useState("7");
  const [agentIds, setAgentIds] = useState<string[]>([]);
  const [cwds, setCwds] = useState<string[]>([]);
  const [expanded, setExpanded] = useState<string | null>(null);
  const [showEmpty, setShowEmpty] = useState(false);
  const [error, setError] = useState(false);
  const [loading, setLoading] = useState(true);
  useEffect(() => {
    let disposed = false;
    let timer: ReturnType<typeof setTimeout>;
    let cursor = Date.now() - SUMMARY_RETENTION_MS;
    const records = new Map<string, SessionEventRecord>();
    async function refresh() {
      const toAt = Date.now();
      try {
        const fresh = await tauriApi.loadSessionEvents(cursor, toAt, ["session_started", "session_state", "prompt", "stop", "usage"]);
        if (disposed) return;
        for (const event of fresh) records.set(`${event.runId}:${event.seq}`, event);
        for (const [key, event] of records) if (event.at < toAt - SUMMARY_RETENTION_MS) records.delete(key);
        cursor = Math.max(cursor, toAt - 1000); // writer의 짧은 flush 지연은 중복 읽기·키 병합으로 흡수.
        setEvents([...records.values()]); setError(false);
      } catch (err) {
        if (disposed) return;
        console.warn("work-panels: event history load failed", err); setError(true);
      }
      if (!disposed) { setNow(toAt); setLoading(false); timer = setTimeout(() => void refresh(), 5000); }
    }
    void refresh();
    return () => { disposed = true; clearTimeout(timer); };
  }, []);
  const allRows = useMemo(() => buildSessionRows(events, summaries, items, liveIds, now), [events, summaries, items, liveIds, now]);
  // 앱을 켤 때마다 캐릭터 수만큼 빈 셸 세션이 생긴다. 기본은 숨기고 필요하면 펼친다.
  const rows = showEmpty ? allRows : allRows.filter((r) => !r.empty);
  const fromAt = period === "today" ? new Date(new Date(now).setHours(0, 0, 0, 0)).getTime() : now - Number(period) * 86_400_000;
  const inPeriod = rows.filter((r) => r.live || (r.endedAt ?? r.startedAt) >= fromAt);
  const agentOptions = [...new Map(rows.map((r) => [r.agentId, r.agentName])).entries()];
  const folderOptions = [...new Set(inPeriod.map((r) => r.cwd ?? ""))].sort();
  const visible = filterSessionRows(rows, { query, agentIds, cwds, fromAt });
  const emptyCount = filterSessionRows(allRows.filter((r) => r.empty), { query, agentIds, cwds, fromAt }).length;
  function toggle(list: string[], value: string) { return list.includes(value) ? list.filter((v) => v !== value) : [...list, value]; }
  return <WorkPanelFrame side="right" title={t("historyTitle")}>
    <div className="session-history-filters">
      <input aria-label={t("search")} placeholder={t("search")} value={query} onChange={(e) => setQuery(e.target.value)} />
      <label>{t("period")}<select value={period} onChange={(e) => { setPeriod(e.target.value); setCwds([]); }}>
        {["today", "7", "30", "300"].map((p) => <option key={p} value={p}>{t(`periods.${p}`)}</option>)}
      </select></label>
      <details><summary>{t("characters")}</summary><button className="pixel-btn" onClick={() => setAgentIds([])}>{t("all")}</button>
        {agentOptions.map(([id, name]) => <label key={id}><input type="checkbox" checked={agentIds.includes(id)} onChange={() => setAgentIds(toggle(agentIds, id))} />{name}</label>)}
      </details>
      <details><summary>{t("folders")}</summary><button className="pixel-btn" onClick={() => setCwds([])}>{t("all")}</button>
        {folderOptions.map((cwd) => <label key={cwd} title={cwd}><input type="checkbox" checked={cwds.includes(cwd)} onChange={() => setCwds(toggle(cwds, cwd))} />{folderName(cwd) || t("unknownFolder")}</label>)}
      </details>
    </div>
    {(error || summaryError) && <p role="alert">{t("historyError")}</p>}
    {loading && <p className="work-muted">{t("loading")}</p>}
    {!loading && !visible.length && <p className="work-muted">{t("emptyHistory")}</p>}
    {visible.map((row) => <article className={`work-card session-history-row${row.live ? " live" : ""}`} key={row.sessionId}>
      <button className="work-card-heading" aria-expanded={expanded === row.sessionId} onClick={() => setExpanded(expanded === row.sessionId ? null : row.sessionId)}>
        <AgentFace agentId={row.agentId} />
        <span><strong>{row.agentName}</strong><small>{new Date(row.startedAt).toLocaleString()} · {row.live ? t("live") : row.endedAt ? (row.endedApprox ? t("endedApprox", { time: new Date(row.endedAt).toLocaleTimeString() }) : new Date(row.endedAt).toLocaleTimeString()) : t("endedUnknown")}</small></span>
      </button>
      <p className="session-history-goal">{row.goal ?? row.prompts[0]?.text ?? t("noSummary")}</p>
      <small title={row.cwd}>{folderName(row.cwd) || t("unknownFolder")}{!!row.stats?.workedMs && ` · ${t("worked", { time: formatDuration(row.stats.workedMs) })}`} · {t("elapsed", { time: row.endedAt || row.live ? formatDuration(Math.max(0, (row.endedAt ?? now) - row.startedAt)) : t("endedUnknown") })}</small>
      <small>{t("stats", { turns: row.stats?.turns ?? 0, tokens: formatTokens((row.stats?.tokensIn ?? 0) + (row.stats?.tokensOut ?? 0) + (row.stats?.tokensCacheRead ?? 0) + (row.stats?.tokensCacheWrite ?? 0)), cost: formatUsd(row.stats?.costUsd ?? 0) })}{!!row.stats?.costUnknownTurns && "*"}</small>
      {row.workTitle && <small className="work-link-tag">{row.workTitle}</small>}
      {expanded === row.sessionId && <SessionDetails row={row} />}
    </article>)}
    {emptyCount > 0 && <label className="work-muted"><input type="checkbox" checked={showEmpty} onChange={(e) => setShowEmpty(e.target.checked)} />{t("showEmpty", { count: emptyCount })}</label>}
    <small className="work-muted">{t("costHint")}</small>
  </WorkPanelFrame>;
}
