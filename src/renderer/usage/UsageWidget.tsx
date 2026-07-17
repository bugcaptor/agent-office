// src/renderer/usage/UsageWidget.tsx
//
// BottomBar 상시 컴팩트 뱃지. provider별(Claude/Codex) 가장 절박한 윈도의
// 사용률을 `CL 61%` `CX 11%`로 보이고, 색상은 임계 70/90(tokens.css 토큰),
// 데이터 없으면 dim `—`. 클릭하면 상세 모달을 연다.
//
// 폴링: 마운트 시 1회 + 60초 간격으로 loadUsageSnapshot을 invoke해 스토어에
// 저장한다(설계 docs/usage-limits-design.md §3). 파일 읽기가 저비용이라
// 백엔드 타이머/파일 워처 없이 단순 폴링으로 충분.
import { useEffect } from "react";
import { useAppStore } from "../store/appStore";
import { tauriApi } from "../ipc/tauriApi";
import type { ProviderUsage } from "@shared/types";
import { PROVIDER_SHORT, mostUrgentWindow, usageLevel } from "./usageView";

/** 폴링 주기(ms). */
const POLL_INTERVAL_MS = 60_000;

const PROVIDERS = ["claude", "codex"] as const;

function ProviderBadge({
  provider,
  usage,
}: {
  provider: "claude" | "codex";
  usage: ProviderUsage | null;
}) {
  const short = PROVIDER_SHORT[provider];
  const win = mostUrgentWindow(usage);
  if (!win) {
    return (
      <span className="usage-badge usage-badge-empty" title={`${short}: 데이터 없음`}>
        {short} <span className="usage-badge-pct">—</span>
      </span>
    );
  }
  const pct = Math.round(win.usedPercent);
  return (
    <span
      className={`usage-badge usage-level-${usageLevel(win.usedPercent)}`}
      title={`${short}: ${pct}% 사용`}
    >
      {short} <span className="usage-badge-pct">{pct}%</span>
    </span>
  );
}

export function UsageWidget() {
  const usage = useAppStore((s) => s.usage);
  const setUsage = useAppStore((s) => s.setUsage);
  const openModal = useAppStore((s) => s.openModal);

  useEffect(() => {
    let cancelled = false;
    const poll = async () => {
      try {
        const snap = await tauriApi.loadUsageSnapshot();
        if (!cancelled) setUsage(snap);
      } catch (err) {
        // 실패는 콘솔 경고로만 — 다음 폴링이 재시도한다(이전 값 유지).
        console.warn("usage: 스냅샷 로드 실패", err);
      }
    };
    void poll();
    const id = window.setInterval(() => void poll(), POLL_INTERVAL_MS);
    return () => {
      cancelled = true;
      window.clearInterval(id);
    };
  }, [setUsage]);

  return (
    <button
      type="button"
      className="pixel-btn usage-widget"
      aria-label="구독 사용량"
      title="구독 사용량 상세 보기"
      onClick={() => openModal({ kind: "usage" })}
    >
      {PROVIDERS.map((p) => (
        <ProviderBadge key={p} provider={p} usage={usage ? usage[p] : null} />
      ))}
    </button>
  );
}
