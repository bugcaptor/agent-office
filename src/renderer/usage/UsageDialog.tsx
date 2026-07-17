// src/renderer/usage/UsageDialog.tsx
//
// 구독 사용량 상세 모달(BottomBar 위젯 클릭으로 열림). ModalState가 usage일
// 때만 렌더한다(AnalyticsDialog와 동일한 셀프 게이팅). provider별로 플랜 라벨·
// 신선도와 각 윈도의 픽셀 바(사용률)·리셋 카운트다운을 보여준다. 카운트다운·
// 신선도는 SessionTimePanel의 1초 tick 패턴(로컬 시계, 재조회 아님)으로 갱신하고,
// stale(>30분)이면 provider 블록을 흐리게 + 표시한다.
// 설계: docs/usage-limits-design.md §3. 폴링·스토어 갱신은 UsageWidget 소관.
import { useEffect, useState } from "react";
import type { ProviderUsage } from "@shared/types";
import { useAppStore } from "../store/appStore";
import {
  PROVIDER_SHORT,
  formatCountdown,
  formatFreshness,
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
        {stale && " · 오래됨"} · {PROVIDER_NAME[usage.provider]} 실행 중에만 갱신됨
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
            if (!pu) {
              return (
                <section key={p} className="usage-provider usage-provider-empty">
                  <div className="usage-provider-head">
                    <span className="usage-provider-name">{PROVIDER_NAME[p]}</span>
                    <span className="usage-badge-empty">{PROVIDER_SHORT[p]}</span>
                  </div>
                  <p className="usage-empty-msg">
                    사용량 데이터가 없습니다. {PROVIDER_NAME[p]}를 한 번 실행하면 로컬 캐시가
                    생깁니다.
                  </p>
                </section>
              );
            }
            return <ProviderSection key={p} usage={pu} now={now} />;
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
