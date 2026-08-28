// src/renderer/usage/UsageFloat.tsx
//
// filled 뷰 모드(이슈 #69)에서는 터미널 오버레이 패널이 창을 완전히 덮어
// BottomBar가 화면 밖으로 사라진다 — 거기 상주하는 UsageWidget(LLM 사용량
// 뱃지)도 함께 안 보인다. 이 컴포넌트는 그 상태에서만 앱 창 우측 아래에
// 반투명 패널로 같은 정보를 띄운다(터미널 내용을 최대한 덜 가리는 게 목적).
//
// 폴링은 하지 않는다 — UsageWidget이 BottomBar에 상시 마운트돼(오버레이가
// 열려 있든 아니든) 이미 60초 주기로 loadUsageSnapshot을 돌려 스토어(s.usage)
// 를 채운다. 여기서는 그 스토어를 읽기만 한다 — 폴링을 중복 실행하면 백엔드
// 파일 읽기가 두 배로 돌 뿐 얻는 게 없다.
//
// 표현은 BottomBar 뱃지를 그대로 따른다: provider별 `CL 12%·61%`를 **한 줄에**
// 나란히 붙인다(예전에는 provider마다 줄을 나누고 창 이름까지 적어 판때기가
// 커져 터미널을 그만큼 가렸다). 창 이름을 포함한 전문은 title 툴팁과 상세
// 모달에 그대로 남아 있어 정보 손실이 아니라 표시 축약이다.
import { useTranslation } from "react-i18next";
import { renderText } from "../shared/textKey";
import { useAppStore } from "../store/appStore";
import type { ProviderUsage } from "@shared/types";
import {
  PROVIDER_SHORT,
  badgeWindows,
  describeProviderLive,
  providerUsage,
  usageLevel,
  visibleUsageProviders,
  windowLabel,
  type LiveStatusNote,
  type UsageProvider,
} from "./usageView";
import "./usage.css";

function FloatBadge({
  provider,
  usage,
  note,
}: {
  provider: UsageProvider;
  usage: ProviderUsage | null;
  /** 실시간 조회 진단(있으면 색 힌트 + 툴팁 접미사를 붙인다, BottomBar와 동일 규칙). */
  note: LiveStatusNote | null;
}) {
  const { t } = useTranslation("activity");
  const short = PROVIDER_SHORT[provider];
  // BottomBar 뱃지와 달리 폭 제약이 없으니 badgeWindows가 고른 창(최대 2개)을
  // 전부 보여준다 — usage-badge-extra(좁을 때 두 번째 창 숨김)는 쓰지 않는다.
  const windows = badgeWindows(usage);
  const degraded = note && note.level !== "ok" ? ` usage-badge-${note.level}` : "";
  const noteSuffix =
    note && note.level !== "ok"
      ? t("usage.widget.noteSuffix", { note: renderText(note.short, t) })
      : "";

  if (windows.length === 0) {
    return (
      <span
        className={`usage-float-badge usage-badge-empty${degraded}`}
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
    <span className={`usage-float-badge${degraded}`} title={title}>
      <span className="usage-badge-label">{short}</span>{" "}
      {windows.map((w, i) => (
        <span key={i}>
          {i > 0 && <span className="usage-badge-sep">·</span>}
          <span className={`usage-badge-pct usage-level-${usageLevel(w.usedPercent)}`}>
            {Math.round(w.usedPercent)}%
          </span>
        </span>
      ))}
    </span>
  );
}

export function UsageFloat() {
  const { t } = useTranslation("activity");
  const enabled = useAppStore((s) => s.appSettings.usageFloatEnabled);
  const viewMode = useAppStore((s) => s.terminalViewMode);
  const isTerminalOpen = useAppStore((s) => s.activeTerminalAgentId !== null);
  const usage = useAppStore((s) => s.usage);
  const openModal = useAppStore((s) => s.openModal);

  // 설정으로 끄면(기본 켜짐) filled 모드에서도 뜨지 않는다 — 하단바 뱃지는
  // 이 컴포넌트와 별개로 상시 마운트라 계속 보인다.
  if (!enabled || viewMode !== "filled" || !isTerminalOpen) return null;

  // 숨김 판정은 렌더 시각 기준(UsageWidget과 동일 근거 — 60초 폴링 해상도로
  // 24시간 임계값을 재는 데 충분해 별도 tick을 두지 않는다).
  const visible = visibleUsageProviders(usage, Date.now());
  if (visible.length === 0) return null;

  return (
    <button
      type="button"
      className="usage-float"
      aria-label={t("usage.widget.aria")}
      title={t("usage.widget.tooltip")}
      onClick={() => openModal({ kind: "usage" })}
    >
      {visible.map((p) => (
        <FloatBadge
          key={p}
          provider={p}
          usage={providerUsage(usage, p)}
          note={describeProviderLive(usage, p)}
        />
      ))}
    </button>
  );
}
