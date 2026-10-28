// src/renderer/usage/OpenRouterBadge.tsx
//
// OpenRouter 키 사용액 뱃지(`OR $0.29`). BottomBar 위젯과 filled 모드
// 플로팅 패널이 함께 쓴다 — 둘 다 provider 뱃지 옆에 같은 모양으로 붙고
// 바깥 클래스만 다르다. 뱃지에는 **오늘** 사용액만, 툴팁에는 오늘·이번 주·
// 이번 달과 키 한도를 싣는다. 색은 키에 한도를 걸어 둔 경우에만 그 한도
// 대비 사용률로 매긴다(usageLevel 70/90 임계).
import { useTranslation } from "react-i18next";
import type { OpenRouterUsage } from "@shared/types";
import { renderText } from "../shared/textKey";
import {
  OPENROUTER_SHORT,
  formatUsd,
  openRouterLimitLabel,
  openRouterLimitPercent,
  usageLevel,
  type LiveStatusNote,
} from "./usageView";

export function OpenRouterBadge({
  usage,
  note,
  className,
}: {
  usage: OpenRouterUsage | null;
  note: LiveStatusNote | null;
  /** `usage-badge`(BottomBar) 또는 `usage-float-badge`(플로팅 패널). */
  className: string;
}) {
  const { t } = useTranslation("activity");
  const short = OPENROUTER_SHORT;
  const degraded = note && note.level !== "ok" ? ` usage-badge-${note.level}` : "";
  const noteSuffix =
    note && note.level !== "ok"
      ? t("usage.widget.noteSuffix", { note: renderText(note.short, t) })
      : "";
  if (!usage) {
    return (
      <span
        className={`${className} usage-badge-or usage-badge-empty${degraded}`}
        title={t("usage.widget.empty", { short, suffix: noteSuffix })}
      >
        <span className="usage-badge-label">{short}</span> <span className="usage-badge-pct">—</span>
      </span>
    );
  }
  const pct = openRouterLimitPercent(usage);
  const parts = [
    t("usage.openrouter.amount", { label: t("usage.openrouter.today"), amount: formatUsd(usage.usageDaily) }),
    t("usage.openrouter.amount", { label: t("usage.openrouter.week"), amount: formatUsd(usage.usageWeekly) }),
    t("usage.openrouter.amount", { label: t("usage.openrouter.month"), amount: formatUsd(usage.usageMonthly) }),
  ];
  if (pct !== null) {
    parts.push(
      t("usage.widget.windowPct", {
        label: renderText(openRouterLimitLabel(usage), t),
        pct: Math.round(pct),
      }),
    );
  }
  const title = t("usage.widget.title", { short, windows: parts.join(" · "), suffix: noteSuffix });
  const level = pct === null ? "" : ` usage-level-${usageLevel(pct)}`;
  return (
    <span className={`${className} usage-badge-or${degraded}`} title={title}>
      <span className="usage-badge-label">{short}</span>{" "}
      <span className={`usage-badge-pct${level}`}>{formatUsd(usage.usageDaily)}</span>
    </span>
  );
}
