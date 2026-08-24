// src/renderer/awards/AwardsDialog.tsx
//
// "이 달의 우수사원" 시상 화면(docs/employee-of-the-month-design.md §6).
// TalkLogDialog와 같은 관례: ModalState(`kind: "awards"`)로 self-gate,
// useEscapeToClose로 Esc 닫기, modal-backdrop 클릭으로 닫기.
//
// 헤더는 월 네비게이션(◀/▶, 확정된 달만) + 드롭다운 + 진행 중인 달 배너.
// 좌측은 트로피 프레임 초상 + 이름/역할/통산 수상, 우측은 통계 배지 + 순위표
// top5 + 수상 소감. `winner: null`인 달은 빈 상태로, 레코드가 아예 없으면
// 다이얼로그 전체가 빈 상태로 바뀐다.
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";

import { useAppStore } from "../store/appStore";
import { useAwardsStore } from "./awardsStore";
import type { ProvisionalAward } from "./awardsStore";
import { tauriApi } from "../ipc/tauriApi";
import { pngBase64ToDataUrl } from "../portrait/portraitCache";
import { useEscapeToClose } from "../shared/useEscapeToClose";
import { formatWhen } from "../diary/diaryExport";
import { formatTokens, formatUsd } from "../analytics/pricing";
import { DEFAULT_MIN_ACTIVE_DAYS, DEFAULT_MIN_WORKED_MS } from "./selection";
import { formatWorkedHm, speechButtonState } from "./awardsView";
import "./awards.css";

/** 트로피 프레임 안 기본 실루엣(초상 없음 + 스냅샷 없음일 때). 순수 렌더. */
function PortraitFallback() {
  return (
    <div className="awards-portrait-fallback" aria-hidden="true">
      <div className="awards-silhouette-head" />
      <div className="awards-silhouette-body" />
    </div>
  );
}

export function AwardsDialog() {
  const { t } = useTranslation("journal");
  const modal = useAppStore((s) => s.modal);
  const closeModal = useAppStore((s) => s.closeModal);
  const agents = useAppStore((s) => s.agents);
  const portraits = useAppStore((s) => s.portraits);
  const summarizerEnabled = useAppStore((s) => s.appSettings.summarizerEnabled);

  const awards = useAwardsStore((s) => s.awards);
  const loaded = useAwardsStore((s) => s.loaded);
  const generating = useAwardsStore((s) => s.generating);
  const error = useAwardsStore((s) => s.error);
  const load = useAwardsStore((s) => s.load);
  const ensureFinalized = useAwardsStore((s) => s.ensureFinalized);
  const provisionalWinner = useAwardsStore((s) => s.provisionalWinner);
  const generateSpeechFor = useAwardsStore((s) => s.generateSpeechFor);
  const awardCountFor = useAwardsStore((s) => s.awardCountFor);

  const open = modal.kind === "awards";
  useEscapeToClose(open, closeModal);

  // 선택된 달. "최신 확정 월 따라가기" 모드(pinned)일 때만 latestMonth를
  // 반영한다 — 소감 재생성 등으로 awards가 갱신돼도 사용자가 과거 달을 보고
  // 있으면 최신 달로 튕기지 않는다.
  const [selectedMonth, setSelectedMonth] = useState<string | null>(null);
  const [pinned, setPinned] = useState(true);
  const months = awards.map((a) => a.month); // month 오름차순(스토어 계약)
  const latestMonth = months.length > 0 ? months[months.length - 1] : null;

  // 열릴 때: 보정 확정(§3, 내부에서 필요하면 load()도 부른다) + 최신 확정
  // 월로 리셋. `loaded`를 의존성에 넣지 않는다 — 열려 있는 동안 로드가
  // 끝나 `loaded`가 바뀌어도 이 효과를 다시 돌릴 이유가 없다(ensureFinalized
  // 자체가 멱등: `finalizing` 가드로 중복 실행을 막는다).
  useEffect(() => {
    if (!open) return;
    void ensureFinalized();
    setPinned(true);
  }, [open, ensureFinalized]);

  useEffect(() => {
    if (pinned) setSelectedMonth(latestMonth);
  }, [pinned, latestMonth]);

  const selectMonth = (m: string) => {
    setPinned(m === latestMonth);
    setSelectedMonth(m);
  };

  const idx = selectedMonth ? months.indexOf(selectedMonth) : -1;
  const canPrev = idx > 0;
  const canNext = idx >= 0 && idx < months.length - 1;
  const isLatest = idx >= 0 && idx === months.length - 1;
  const record = idx >= 0 ? awards[idx] : undefined;

  // 진행 중인 달의 잠정 1위 — 최신 확정 월을 보고 있을 때만 라이브로 계산.
  const [provisional, setProvisional] = useState<ProvisionalAward | null>(null);
  useEffect(() => {
    if (!open || !isLatest) {
      setProvisional(null);
      return;
    }
    let cancelled = false;
    void provisionalWinner().then((p) => {
      if (!cancelled) setProvisional(p);
    });
    return () => {
      cancelled = true;
    };
  }, [open, isLatest, provisionalWinner]);

  // 수상자 초상 스냅샷(확정 시점 PNG). 달이 바뀔 때마다 다시 읽는다.
  const [snapshotUrl, setSnapshotUrl] = useState<string | null>(null);
  useEffect(() => {
    if (!open || !record?.winner?.hasPortrait) {
      setSnapshotUrl(null);
      return;
    }
    let cancelled = false;
    void tauriApi
      .loadAwardPortrait(record.month)
      .then((b64) => {
        if (!cancelled) setSnapshotUrl(b64 ? pngBase64ToDataUrl(b64) : null);
      })
      .catch((err) => {
        console.warn(`awards: portrait snapshot load failed (month=${record.month})`, err);
        if (!cancelled) setSnapshotUrl(null);
      });
    return () => {
      cancelled = true;
    };
  }, [open, record?.month, record?.winner?.hasPortrait]);

  if (!open) return null;

  const winner = record?.winner ?? null;
  const currentPortraitUrl = winner ? portraits[winner.agentId] : undefined;
  const portraitSrc = snapshotUrl ?? currentPortraitUrl ?? null;
  const profileExists = winner ? winner.agentId in agents : false;
  const btnState = speechButtonState({
    winner,
    profileExists,
    summarizerEnabled,
    generating: record ? (generating[record.month] ?? false) : false,
  });
  const speeches = record?.speeches ?? [];
  const lastSpeech = speeches.length > 0 ? speeches[speeches.length - 1] : undefined;
  const previousSpeeches = speeches.length > 1 ? speeches.slice(0, -1).reverse() : [];

  return (
    <div
      className="modal-backdrop"
      onMouseDown={(e) => {
        if (e.button === 0 && e.target === e.currentTarget) closeModal();
      }}
    >
      <div className="pixel-panel awards-dialog" role="dialog" aria-label={t("awards.dialogAria")}>
        <div className="awards-head">
          <div className="awards-nav">
            <button
              type="button"
              className="pixel-btn"
              aria-label={t("awards.prevMonth")}
              disabled={!canPrev}
              onClick={() => canPrev && selectMonth(months[idx - 1])}
            >
              ◀
            </button>
            <h2 className="pixel-title">{t("awards.title", { month: selectedMonth ?? "—" })}</h2>
            <button
              type="button"
              className="pixel-btn"
              aria-label={t("awards.nextMonth")}
              disabled={!canNext}
              onClick={() => canNext && selectMonth(months[idx + 1])}
            >
              ▶
            </button>
          </div>
          {months.length > 0 && (
            <select
              className="awards-month-select"
              aria-label={t("awards.monthListAria")}
              value={selectedMonth ?? ""}
              onChange={(e) => selectMonth(e.target.value)}
            >
              {months
                .slice()
                .reverse()
                .map((m) => (
                  <option key={m} value={m}>
                    {m}
                  </option>
                ))}
            </select>
          )}
          <button type="button" className="pixel-btn" onClick={closeModal}>
            {t("awards.close")}
          </button>
        </div>

        {isLatest && provisional?.winner && (
          <p className="awards-provisional-banner">
            {t("awards.provisional", { name: provisional.winner.name })}
          </p>
        )}

        {!loaded ? (
          <p className="awards-msg">{t("awards.loading")}</p>
        ) : awards.length === 0 ? (
          <div className="awards-msg">
            <p>{t("awards.emptyAll")}</p>
            {error && (
              <p className="awards-msg-error">
                {error}{" "}
                <button type="button" className="pixel-btn" onClick={() => void load()}>
                  {t("awards.retry")}
                </button>
              </p>
            )}
          </div>
        ) : !record ? (
          <p className="awards-msg">{t("awards.loading")}</p>
        ) : winner === null ? (
          <div className="awards-empty">
            <p>{t("awards.noWinner")}</p>
            <p className="awards-empty-hint">
              {t("awards.noWinnerHint", {
                days: DEFAULT_MIN_ACTIVE_DAYS,
                minutes: Math.round(DEFAULT_MIN_WORKED_MS / 60_000),
              })}
            </p>
          </div>
        ) : (
          <div className="awards-body">
            <div className="awards-left">
              <div className="awards-trophy-frame">
                {portraitSrc ? (
                  <img src={portraitSrc} alt={winner.name} />
                ) : (
                  <PortraitFallback />
                )}
                <span className="awards-trophy-badge" aria-hidden="true">
                  🏆
                </span>
              </div>
              <div className="awards-winner-name">{winner.name}</div>
              <div className="awards-winner-role">{winner.role}</div>
              <div className="awards-award-count">
                {t("awards.awardCount", { count: awardCountFor(winner.agentId) })}
              </div>
            </div>

            <div className="awards-right">
              <dl className="awards-stats">
                <div className="awards-stat">
                  <dt>{t("awards.statWorked")}</dt>
                  <dd>{formatWorkedHm(winner.stats.workedMs)}</dd>
                </div>
                <div className="awards-stat">
                  <dt>{t("awards.statTurns")}</dt>
                  <dd>{winner.stats.turns}</dd>
                </div>
                <div className="awards-stat">
                  <dt>{t("awards.statTools")}</dt>
                  <dd>{winner.stats.toolEvents}</dd>
                </div>
                <div className="awards-stat">
                  <dt>{t("awards.statActiveDays")}</dt>
                  <dd>{t("awards.days", { count: winner.stats.activeDays })}</dd>
                </div>
                <div className="awards-stat">
                  <dt>{t("awards.statTokens")}</dt>
                  <dd>{formatTokens(winner.stats.tokensIn + winner.stats.tokensOut)}</dd>
                </div>
                <div className="awards-stat">
                  <dt>{t("awards.statCost")}</dt>
                  <dd>{formatUsd(winner.stats.costUsd)}</dd>
                </div>
              </dl>

              <table className="awards-table">
                <thead>
                  <tr>
                    <th scope="col">{t("awards.colRank")}</th>
                    <th scope="col">{t("awards.colName")}</th>
                    <th scope="col">{t("awards.colWorked")}</th>
                    <th scope="col">{t("awards.colTurns")}</th>
                    <th scope="col">{t("awards.colActiveDays")}</th>
                  </tr>
                </thead>
                <tbody>
                  {record.leaderboard.map((row, i) => (
                    <tr
                      key={row.agentId}
                      className={
                        row.agentId === winner.agentId ? "awards-table-winner-row" : undefined
                      }
                    >
                      <td>{i + 1}</td>
                      <td>{row.name}</td>
                      <td>{formatWorkedHm(row.workedMs)}</td>
                      <td>{row.turns}</td>
                      <td>{t("awards.days", { count: row.activeDays })}</td>
                    </tr>
                  ))}
                </tbody>
              </table>

              <section className="awards-speech">
                <h3 className="awards-section-title">{t("awards.speechTitle")}</h3>
                {error && (
                  <div className="awards-error">
                    <p>{error}</p>
                    <button
                      type="button"
                      className="pixel-btn"
                      disabled={btnState.disabled}
                      onClick={() => void generateSpeechFor(record.month)}
                    >
                      {t("awards.retry")}
                    </button>
                  </div>
                )}
                {!lastSpeech ? (
                  <button
                    type="button"
                    className="pixel-btn primary"
                    disabled={btnState.disabled}
                    title={btnState.disabled ? t(btnState.reasonKey) : undefined}
                    onClick={() => void generateSpeechFor(record.month)}
                  >
                    {generating[record.month]
                      ? t("awards.speechGenerating")
                      : t("awards.speechListen")}
                  </button>
                ) : (
                  <>
                    <div className="awards-speech-card">
                      <p className="awards-speech-text">{lastSpeech.text}</p>
                      <p className="awards-speech-meta">
                        {formatWhen(lastSpeech.at)} · {lastSpeech.provider}
                      </p>
                    </div>
                    <button
                      type="button"
                      className="pixel-btn"
                      disabled={btnState.disabled}
                      title={btnState.disabled ? t(btnState.reasonKey) : undefined}
                      onClick={() => void generateSpeechFor(record.month)}
                    >
                      {generating[record.month]
                        ? t("awards.speechGenerating")
                        : t("awards.speechAgain")}
                    </button>
                    {previousSpeeches.length > 0 && (
                      <details className="awards-speech-prev">
                        <summary>
                          {t("awards.speechPrev", { count: previousSpeeches.length })}
                        </summary>
                        <ul>
                          {previousSpeeches.map((s, i) => (
                            <li key={`${s.at}-${i}`} className="awards-speech-prev-item">
                              <p className="awards-speech-text">{s.text}</p>
                              <p className="awards-speech-meta">
                                {formatWhen(s.at)} · {s.provider}
                              </p>
                            </li>
                          ))}
                        </ul>
                      </details>
                    )}
                  </>
                )}
              </section>
            </div>
          </div>
        )}
      </div>
    </div>
  );
}
