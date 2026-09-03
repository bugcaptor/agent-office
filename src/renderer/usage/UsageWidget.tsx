// src/renderer/usage/UsageWidget.tsx
//
// BottomBar 상시 컴팩트 뱃지. provider별(Claude/Codex)로 badgeWindows가 고른
// 윈도(최대 2개: 5시간 창 + 나머지 중 가장 절박한 창)를 `CL 12%·61%`처럼
// 가운뎃점으로 병기한다(이슈 #36 — 주간 창이 더 절박해도 5시간 창 변동이
// 보이게). Codex의 Spark 같은 모델별 특수 한도는 대표값에서 제외한다.
// 퍼센트마다 자기 usedPercent 기준 색(임계 70/90, tokens.css 토큰),
// 데이터 없으면 dim `—`. 클릭하면 상세 모달을 연다.
//
// 하루 넘게 갱신되지 못한 provider(또는 시도했는데 값이 하나도 없는
// provider)는 흐리게가 아니라 **뱃지에서 통째로 빠진다**(kbm #2j4,
// usageView.isProviderGone). 셋 다 빠지면 위젯 버튼 자체를 그리지 않는다 —
// 빈 버튼은 BottomBar에서 폭만 먹는다.
//
// 폭에 따른 병기 규칙(PR #37 봇 P2 — 800px 기본 폭에서 두 번째 퍼센트가
// 항상 보이면 .bottom-bar-status가 말줄임으로 잘림): 좁은 폭(<960px)에서는
// 첫 번째(5시간) 창만 보이고, 두 번째 이후 창은 warn/danger(≥70%)일 때만
// 예외적으로 보인다 — 한도 경고가 폭 절약보다 우선. 상시 병기는 라벨
// 경계(900px)보다 늦은 960px부터다(동시에 켜지면 900px 직후가 다시 빠듯
// — usage.css 미디어 쿼리 주석 참고). 두 번째 이후 래퍼에 붙는
// `usage-badge-extra` 클래스가 이 숨김을 담당한다. 툴팁(title)은 폭과
// 무관하게 항상 전체 창 정보를 포함한다.
//
// 폴링: 마운트 시 1회 + 60초 간격으로 loadUsageSnapshot을 invoke해 스토어에
// 저장한다(설계 docs/usage-design.md §3). 파일 읽기가 저비용이라
// 백엔드 타이머/파일 워처 없이 단순 폴링으로 충분. 응답의 provider별 null은
// mergeUsageSnapshot으로 이전 값 위에 덮어써(일시 파싱 실패가 유효 값을
// 지우지 않게) 저장한다.
import { useEffect } from "react";
import { useTranslation } from "react-i18next";
import { renderText } from "../shared/textKey";
import { useAppStore } from "../store/appStore";
import { tauriApi } from "../ipc/tauriApi";
import type { ProviderUsage } from "@shared/types";
import {
  PROVIDER_SHORT,
  badgeWindows,
  describeProviderLive,
  mergeUsageSnapshot,
  providerUsage,
  usageLevel,
  visibleUsageProviders,
  windowLabel,
  type LiveStatusNote,
  type UsageProvider,
} from "./usageView";

/** 폴링 주기(ms). */
const POLL_INTERVAL_MS = 60_000;

function ProviderBadge({
  provider,
  usage,
  note,
}: {
  provider: UsageProvider;
  usage: ProviderUsage | null;
  /** 실시간 조회 진단(있으면 툴팁에 덧붙이고 뱃지를 표시색으로 물들인다). */
  note: LiveStatusNote | null;
}) {
  const { t } = useTranslation("activity");
  const short = PROVIDER_SHORT[provider];
  const windows = badgeWindows(usage);
  // 폭이 빠듯한 BottomBar(설계 §BottomBar 800px)라 글자를 늘리지 않는다 —
  // 사유는 툴팁과 상세 모달에, 여기서는 색 힌트만 준다.
  const degraded = note && note.level !== "ok" ? ` usage-badge-${note.level}` : "";
  const noteSuffix =
    note && note.level !== "ok"
      ? t("usage.widget.noteSuffix", { note: renderText(note.short, t) })
      : "";
  if (windows.length === 0) {
    return (
      <span
        className={`usage-badge usage-badge-empty${degraded}`}
        title={t("usage.widget.empty", { short, suffix: noteSuffix })}
      >
        <span className="usage-badge-label">{short}</span> <span className="usage-badge-pct">—</span>
      </span>
    );
  }
  const title = t("usage.widget.title", {
    short,
    windows: windows
      .map((w) =>
        t("usage.widget.windowPct", {
          label: renderText(windowLabel(w), t),
          pct: Math.round(w.usedPercent),
        }),
      )
      .join(" · "),
    suffix: noteSuffix,
  });
  return (
    <span className={`usage-badge${degraded}`} title={title}>
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
  const { t } = useTranslation("activity");
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
        console.warn("usage: failed to load snapshot", err);
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

  // 숨김 판정은 렌더 시각 기준이다. 별도 tick을 두지 않는 이유: 폴링이
  // 60초마다 새 스냅샷 객체를 스토어에 넣어 어차피 다시 그려지고, 임계값이
  // 24시간이라 60초 해상도로 충분하다.
  const visible = visibleUsageProviders(usage, Date.now());
  if (visible.length === 0) return null;

  return (
    <button
      type="button"
      className="pixel-btn usage-widget"
      aria-label={t("usage.widget.aria")}
      title={t("usage.widget.tooltip")}
      onClick={() => openModal({ kind: "usage" })}
    >
      {visible.map((p) => (
        <ProviderBadge
          key={p}
          provider={p}
          usage={providerUsage(usage, p)}
          note={describeProviderLive(usage, p)}
        />
      ))}
    </button>
  );
}
