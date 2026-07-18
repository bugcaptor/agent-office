// src/renderer/usage/UsageWidget.tsx
//
// BottomBar 상시 컴팩트 뱃지. provider별(Claude/Codex)로 badgeWindows가 고른
// 윈도(최대 2개: 5시간 창 + 나머지 중 가장 절박한 창)를 `CL 12%·61%`처럼
// 가운뎃점으로 병기한다(이슈 #36 — 주간 창이 더 절박해도 5시간 창 변동이
// 보이게). 퍼센트마다 자기 usedPercent 기준 색(임계 70/90, tokens.css 토큰),
// 데이터 없으면 dim `—`. 클릭하면 상세 모달을 연다.
//
// 폭에 따른 병기 규칙(PR #37 봇 P2 — 800px 기본 폭에서 두 번째 퍼센트가
// 항상 보이면 .bottom-bar-status가 말줄임으로 잘림): 좁은 폭(<900px)에서는
// 첫 번째(5시간) 창만 보이고, 두 번째 이후 창은 warn/danger(≥70%)일 때만
// 예외적으로 보인다 — 한도 경고가 폭 절약보다 우선. 900px 이상에서는 항상
// 상시 병기(usage.css 미디어 쿼리로 처리). 두 번째 이후 래퍼에 붙는
// `usage-badge-extra` 클래스가 이 숨김을 담당한다. 툴팁(title)은 폭과
// 무관하게 항상 전체 창 정보를 포함한다.
//
// 폴링: 마운트 시 1회 + 60초 간격으로 loadUsageSnapshot을 invoke해 스토어에
// 저장한다(설계 docs/usage-limits-design.md §3). 파일 읽기가 저비용이라
// 백엔드 타이머/파일 워처 없이 단순 폴링으로 충분. 응답의 provider별 null은
// mergeUsageSnapshot으로 이전 값 위에 덮어써(일시 파싱 실패가 유효 값을
// 지우지 않게) 저장한다.
import { useEffect } from "react";
import { useAppStore } from "../store/appStore";
import { tauriApi } from "../ipc/tauriApi";
import type { ProviderUsage } from "@shared/types";
import {
  PROVIDER_SHORT,
  badgeWindows,
  mergeUsageSnapshot,
  usageLevel,
  windowLabel,
} from "./usageView";

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
  const windows = badgeWindows(usage);
  if (windows.length === 0) {
    return (
      <span className="usage-badge usage-badge-empty" title={`${short}: 데이터 없음`}>
        <span className="usage-badge-label">{short}</span> <span className="usage-badge-pct">—</span>
      </span>
    );
  }
  const title = `${short}: ${windows.map((w) => `${windowLabel(w)} ${Math.round(w.usedPercent)}%`).join(" · ")}`;
  return (
    <span className="usage-badge" title={title}>
      {/* usage-badge-label은 BottomBar가 좁을 때 usage.css 미디어 쿼리로
          숨겨진다 — 좁은 폭에서는 퍼센트 숫자만 남긴다(레이아웃 §BottomBar 800px). */}
      <span className="usage-badge-label">{short}</span>{" "}
      {windows.map((w, i) => (
        <span key={i} className={i > 0 ? `usage-badge-extra usage-level-${usageLevel(w.usedPercent)}` : undefined}>
          {i > 0 && <span className="usage-badge-sep">·</span>}
          <span className={`usage-badge-pct usage-level-${usageLevel(w.usedPercent)}`}>
            {Math.round(w.usedPercent)}%
          </span>
        </span>
      ))}
    </span>
  );
}

export function UsageWidget() {
  const usage = useAppStore((s) => s.usage);
  const setUsage = useAppStore((s) => s.setUsage);
  const openModal = useAppStore((s) => s.openModal);

  useEffect(() => {
    let cancelled = false;
    // 이전 폴링이 아직 진행 중이면(예: 첫 스캔이 60초를 넘김) 새 폴링을
    // 건너뛴다 — 스캔이 겹쳐 쌓이는 것을 막는다. 응답 순서 역전 자체는
    // mergeUsageSnapshot의 fetchedAtMs 비교가 막는다.
    let inFlight = false;
    const poll = async () => {
      if (inFlight) return;
      inFlight = true;
      try {
        const snap = await tauriApi.loadUsageSnapshot();
        if (!cancelled) setUsage(mergeUsageSnapshot(useAppStore.getState().usage, snap));
      } catch (err) {
        // 실패는 콘솔 경고로만 — 다음 폴링이 재시도한다(이전 값 유지).
        console.warn("usage: 스냅샷 로드 실패", err);
      } finally {
        inFlight = false;
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
