// src/renderer/analytics/AnalyticsDialog.tsx
//
// 세션 활동 분석 패널(BottomBar 📊로 열림). ModalState가 analytics일 때만
// 렌더한다(SettingsDialog와 동일한 셀프 게이팅). 열릴 때와 기간(7/14/30일)
// 변경 시 loadSessionEvents를 1회 호출하고, 렌더러 순수 함수(aggregate.ts)로
// 집계해 일별 스택 막대 + 캐릭터 요약 표를 보여준다.
// 설계: docs/session-analytics-design.md §4.4. 실시간 갱신은 비목표 —
// 열려 있는 동안 새 이벤트를 반영하지 않는다.
import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { useTranslation } from "react-i18next";
import type { SessionEventRecord } from "@shared/types";
import { renderText, type Translate } from "../shared/textKey";
import { useAppStore } from "../store/appStore";
import { tauriApi } from "../ipc/tauriApi";
import type { AgentSummary } from "./aggregate";
import { aggregate, dayRange, localDayCalendar } from "./aggregate";
import { DailyBarChart, formatDuration } from "./DailyBarChart";
import { formatTokens, formatUsd } from "./pricing";

/** 기간 셀렉터 후보(일). */
const PERIODS = [7, 14, 30] as const;
type Period = (typeof PERIODS)[number];

/**
 * 표시 창 `fromAt`보다 이만큼 앞선 이벤트까지 함께 조회한다. 경계를 걸친 턴
 * (prompt가 fromAt 직전, stop이 창 안)의 시작 이벤트를 놓치지 않기 위한
 * lookback이다. 집계는 표시 창으로 다시 클립하므로 lookback은 화면에 새지 않는다.
 */
const LOOKBACK_MS = 24 * 60 * 60 * 1000;

/**
 * 최근 `days` 로컬 일을 덮는 표시 창 [fromAt, toAt](epoch ms). 오늘을 마지막
 * 날로. 자정은 달력 산술로 계산해 DST 전환 주에도 어긋나지 않는다(고정
 * `DAY_MS` 뺄셈은 23/25시간 날에 자정이 깨진다). 시스템 TZ 의존이라 DST
 * 유닛테스트는 결정적이지 않아 생략한다.
 */
function rangeFor(days: number, now: number): { fromAt: number; toAt: number } {
  const d = new Date(now);
  const fromAt = new Date(d.getFullYear(), d.getMonth(), d.getDate() - (days - 1)).getTime();
  return { fromAt, toAt: now };
}

/**
 * 토큰 셀 툴팁: 0인 항목은 빼고 나열한다. 전부 0이면 빈 문자열(툴팁 생략).
 * 예: "입력 12.3K · 출력 4.5K · 캐시읽기 1.2M". 항목 수가 0~4개로 달라지는
 * 자리라 잇는 가운뎃점은 카탈로그가 아니라 여기서 붙인다.
 */
function tokenBreakdown(row: AgentSummary, t: Translate): string {
  const parts: string[] = [];
  if (row.tokensIn > 0) parts.push(t("analytics.tokenIn", { value: formatTokens(row.tokensIn) }));
  if (row.tokensOut > 0) parts.push(t("analytics.tokenOut", { value: formatTokens(row.tokensOut) }));
  if (row.tokensCacheRead > 0)
    parts.push(t("analytics.tokenCacheRead", { value: formatTokens(row.tokensCacheRead) }));
  if (row.tokensCacheWrite > 0)
    parts.push(t("analytics.tokenCacheWrite", { value: formatTokens(row.tokensCacheWrite) }));
  return parts.join(" · ");
}

type LoadState =
  | { status: "loading" }
  | { status: "error" }
  | { status: "ready"; records: SessionEventRecord[]; fromAt: number; toAt: number };

export function AnalyticsDialog() {
  const { t } = useTranslation("activity");
  const modal = useAppStore((s) => s.modal);
  const closeModal = useAppStore((s) => s.closeModal);
  const profiles = useAppStore((s) => s.agents);

  const [period, setPeriod] = useState<Period>(7);
  const [load, setLoad] = useState<LoadState>({ status: "loading" });
  // 요청 세대 카운터: 기간을 빠르게 바꾸면 이전 조회의 늦은 응답이 최신 결과를
  // 덮을 수 있어(레이스), 응답 도착 시 최신 세대가 아니면 상태를 갱신하지 않는다.
  const genRef = useRef(0);

  const open = modal.kind === "analytics";

  const fetchRange = useCallback(async (days: number) => {
    const gen = ++genRef.current;
    setLoad({ status: "loading" });
    const { fromAt, toAt } = rangeFor(days, Date.now());
    try {
      // lookback만큼 앞서 조회하고, 표시 창은 [fromAt, toAt]로 aggregate에 넘긴다.
      const records = await tauriApi.loadSessionEvents(fromAt - LOOKBACK_MS, toAt);
      if (gen !== genRef.current) return; // 낡은 응답 — 최신 결과를 덮지 않는다
      setLoad({ status: "ready", records, fromAt, toAt });
    } catch (err) {
      if (gen !== genRef.current) return; // 낡은 실패도 무시
      console.warn("analytics: failed to load session events", err);
      setLoad({ status: "error" });
    }
  }, []);

  // 열릴 때 + 기간 변경 시 재조회. 닫혀 있으면 아무것도 하지 않는다.
  useEffect(() => {
    if (open) void fetchRange(period);
  }, [open, period, fetchRange]);

  const analytics = useMemo(() => {
    if (load.status !== "ready") return null;
    const data = aggregate(load.records, profiles, localDayCalendar, {
      fromAt: load.fromAt,
      toAt: load.toAt,
    });
    const days = dayRange(load.fromAt, load.toAt, localDayCalendar);
    return { data, days };
  }, [load, profiles]);

  if (!open) return null;

  return (
    <div
      className="modal-backdrop"
      onMouseDown={(e) => {
        if (e.button === 0 && e.target === e.currentTarget) closeModal();
      }}
    >
      <div className="pixel-panel analytics-dialog">
        <div className="analytics-head">
          <h2 className="pixel-title">{t("analytics.title")}</h2>
          <div className="analytics-period" role="group" aria-label={t("analytics.periodAria")}>
            {PERIODS.map((p) => (
              <button
                key={p}
                type="button"
                className={`pixel-btn analytics-period-btn${p === period ? " active" : ""}`}
                aria-pressed={p === period}
                onClick={() => setPeriod(p)}
              >
                {t("analytics.periodDays", { days: p })}
              </button>
            ))}
          </div>
        </div>

        <div className="analytics-body">
          {load.status === "loading" && (
            <p className="analytics-msg">{t("analytics.loading")}</p>
          )}
          {load.status === "error" && (
            <div className="analytics-msg analytics-error">
              <p>{t("analytics.loadError")}</p>
              <button
                type="button"
                className="pixel-btn"
                onClick={() => void fetchRange(period)}
              >
                {t("analytics.retry")}
              </button>
            </div>
          )}
          {load.status === "ready" && analytics && analytics.data.summary.length === 0 && (
            <p className="analytics-msg">{t("analytics.empty")}</p>
          )}
          {load.status === "ready" && analytics && analytics.data.summary.length > 0 && (
            <>
              <DailyBarChart
                days={analytics.days}
                agents={analytics.data.summary}
                daily={analytics.data.daily}
              />
              <table className="analytics-table">
                <thead>
                  <tr>
                    <th scope="col">{t("analytics.colAgent")}</th>
                    <th scope="col">{t("analytics.colWorked")}</th>
                    <th scope="col">{t("analytics.colTurns")}</th>
                    <th scope="col">{t("analytics.colTools")}</th>
                    <th scope="col">{t("analytics.colTokens")}</th>
                    <th scope="col" title={t("analytics.costHint")}>
                      {t("analytics.colCost")}
                    </th>
                    <th scope="col">{t("analytics.colDays")}</th>
                  </tr>
                </thead>
                <tbody>
                  {analytics.data.summary.map((row) => {
                    const tokenTotal = row.tokensIn + row.tokensOut;
                    const breakdown = tokenBreakdown(row, t);
                    const hasCost = row.costUsd > 0 || row.costUnknownTurns > 0;
                    return (
                      <tr key={row.agentId}>
                        <th scope="row" className="analytics-agent">
                          <span
                            className="analytics-swatch"
                            style={{ background: row.color }}
                            aria-hidden="true"
                          />
                          <span className={row.deleted ? "analytics-deleted" : undefined}>
                            {row.name}
                            {row.deleted ? t("analytics.deletedSuffix") : ""}
                          </span>
                        </th>
                        <td>{renderText(formatDuration(row.workedMs), t)}</td>
                        <td>{row.turns}</td>
                        <td>{row.toolEvents}</td>
                        <td title={breakdown || undefined}>
                          {tokenTotal > 0 ? formatTokens(tokenTotal) : "—"}
                        </td>
                        <td
                          title={
                            row.costUnknownTurns > 0
                              ? t("analytics.costUnknownHint", { count: row.costUnknownTurns })
                              : undefined
                          }
                        >
                          {hasCost
                            ? `${row.costUnknownTurns > 0 ? "~" : ""}${formatUsd(row.costUsd)}`
                            : "—"}
                        </td>
                        <td>{row.activeDays}</td>
                      </tr>
                    );
                  })}
                </tbody>
              </table>
              <p className="analytics-footnote">{t("analytics.footnote")}</p>
            </>
          )}
        </div>

        <div className="dialog-actions">
          <button className="pixel-btn" onClick={closeModal}>
            {t("analytics.close")}
          </button>
        </div>
      </div>
    </div>
  );
}
