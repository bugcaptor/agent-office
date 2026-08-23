// src/renderer/usage/UsageDialog.tsx
//
// 구독 사용량 상세 모달(BottomBar 위젯 클릭으로 열림). ModalState가 usage일
// 때만 렌더한다(AnalyticsDialog와 동일한 셀프 게이팅). provider별로 플랜 라벨·
// 신선도와 각 윈도의 픽셀 바(사용률)·리셋 카운트다운을 보여준다. 카운트다운·
// 신선도는 SessionTimePanel의 1초 tick 패턴(로컬 시계, 재조회 아님)으로 갱신하고,
// stale(>30분)이면 provider 블록을 흐리게 + 표시한다.
// 설계: docs/usage-limits-design.md §3. 폴링·스토어 갱신은 UsageWidget 소관.
import { useEffect, useState, type ReactNode } from "react";
import type { ClaudeLiveStatus, CodexLiveStatus, ProviderUsage } from "@shared/types";
import { useAppStore } from "../store/appStore";
import {
  PROVIDER_SHORT,
  describeCodexLiveStatus,
  describeLiveStatus,
  formatCountdown,
  formatFreshness,
  formatLiveAttempts,
  isStale,
  usageLevel,
  windowLabel,
} from "./usageView";

const PROVIDER_NAME: Record<"claude" | "codex", string> = {
  claude: "Claude Code",
  codex: "Codex CLI",
};

const PROVIDERS = ["claude", "codex"] as const;

/** 표시용 1초 tick(로컬 시계). 모달이 열려 있을 때만 돈다. */
function useOneSecondTick(active: boolean): number {
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    if (!active) return;
    setNow(Date.now());
    const id = window.setInterval(() => setNow(Date.now()), 1000);
    return () => window.clearInterval(id);
  }, [active]);
  return now;
}

/**
 * provider 블록 + (Claude만) 실시간 조회 진단 줄. 진단은 `.usage-stale`
 * (opacity 0.5) 바깥에 둔다 — 값이 낡아서 흐려진 블록 안에 "왜 낡았는지"를
 * 넣으면 정작 읽어야 할 설명이 같이 흐려진다(opacity는 자식이 되돌릴 수 없다).
 */
function ProviderBlock({ children, note }: { children: ReactNode; note: ReactNode }) {
  return (
    <div className="usage-provider-block">
      {children}
      {note}
    </div>
  );
}

function ProviderSection({ usage, now }: { usage: ProviderUsage; now: number }) {
  const stale = isStale(usage.fetchedAtMs, now);
  return (
    <section className={`usage-provider${stale ? " usage-stale" : ""}`}>
      <div className="usage-provider-head">
        <span className="usage-provider-name">{PROVIDER_NAME[usage.provider]}</span>
        {usage.planLabel && <span className="usage-plan">{usage.planLabel}</span>}
      </div>
      <ul className="usage-windows">
        {usage.windows.map((w, i) => {
          const pct = Math.round(w.usedPercent);
          const countdown = formatCountdown(w.resetsAtMs, now);
          return (
            <li key={`${w.kind}-${w.label ?? ""}-${i}`} className="usage-window">
              <div className="usage-window-row">
                <span className="usage-window-label">
                  {windowLabel(w)}
                  {w.isActive === true && <span className="usage-active-tag">지금 적용 중</span>}
                </span>
                <span className="usage-window-pct">{pct}%</span>
              </div>
              <div
                className="usage-bar"
                role="progressbar"
                aria-valuenow={pct}
                aria-valuemin={0}
                aria-valuemax={100}
                aria-label={`${windowLabel(w)} 사용률`}
              >
                <span
                  className={`usage-bar-fill usage-level-${usageLevel(w.usedPercent)}`}
                  style={{ width: `${Math.min(100, Math.max(0, w.usedPercent))}%` }}
                />
              </div>
              {countdown && <span className="usage-countdown">{countdown}</span>}
            </li>
          );
        })}
      </ul>
      <div className="usage-freshness">
        {formatFreshness(usage.fetchedAtMs, now)}
        {stale && " · 오래됨"}
        {/* 두 provider 모두 갱신 조건이 상황에 따라 달라졌다(실시간 조회
            성공 여부) — 고정 문구 대신 진단 줄이 설명한다. */}
      </div>
    </section>
  );
}

export function UsageDialog() {
  const modal = useAppStore((s) => s.modal);
  const closeModal = useAppStore((s) => s.closeModal);
  const usage = useAppStore((s) => s.usage);

  const open = modal.kind === "usage";
  const now = useOneSecondTick(open);

  if (!open) return null;

  // 실시간 조회 진단 — "왜 이 숫자가 안 움직이는지"의 답. 성공 중일 때도 한
  // 줄 남겨 둔다(정상임을 확인할 수 있어야 진단으로 쓸모가 있다). provider
  // 마다 조회 경로가 달라(Claude=HTTPS 직접 조회, Codex=codex CLI RPC)
  // 진단도 각자 것을 그린다.
  const liveNote = (
    status: ClaudeLiveStatus | CodexLiveStatus | null | undefined,
    described: ReturnType<typeof describeLiveStatus>,
  ): ReactNode =>
    status && described ? (
      <p className={`usage-live-note usage-live-${described.level}`}>
        {described.text}
        {(() => {
          const attempts = formatLiveAttempts(status, now);
          return attempts ? <span className="usage-live-attempts">{attempts}</span> : null;
        })()}
      </p>
    ) : null;
  const notes: Record<"claude" | "codex", ReactNode> = {
    claude: liveNote(usage?.claudeLive, describeLiveStatus(usage?.claudeLive)),
    codex: liveNote(usage?.codexLive, describeCodexLiveStatus(usage?.codexLive)),
  };

  return (
    <div
      className="modal-backdrop"
      onMouseDown={(e) => {
        if (e.button === 0 && e.target === e.currentTarget) closeModal();
      }}
    >
      <div className="pixel-panel usage-dialog">
        <div className="usage-head">
          <h2 className="pixel-title">구독 사용량</h2>
        </div>

        <div className="usage-body">
          {PROVIDERS.map((p) => {
            const pu = usage ? usage[p] : null;
            const note = notes[p];
            const body = !pu ? (
              <section className="usage-provider usage-provider-empty">
                <div className="usage-provider-head">
                  <span className="usage-provider-name">{PROVIDER_NAME[p]}</span>
                  <span className="usage-badge-empty">{PROVIDER_SHORT[p]}</span>
                </div>
                <p className="usage-empty-msg">
                  {p === "claude"
                    ? "사용량 데이터가 없습니다. Claude Code에서 /usage를 한 번 열면 로컬 캐시가 생깁니다."
                    : "사용량 데이터가 없습니다. codex login 후 앱이 직접 조회하거나, Codex CLI를 한 번 실행하면 로컬 기록이 생깁니다."}
                </p>
              </section>
            ) : (
              <ProviderSection usage={pu} now={now} />
            );
            return (
              <ProviderBlock key={p} note={note}>
                {body}
              </ProviderBlock>
            );
          })}
        </div>

        <div className="dialog-actions">
          <button className="pixel-btn" onClick={closeModal}>
            닫기
          </button>
        </div>
      </div>
    </div>
  );
}
