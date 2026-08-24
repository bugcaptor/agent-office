// src/renderer/analytics/DailyBarChart.tsx
//
// 일별 작업시간 스택 막대(SVG 자체 구현 — 차트 라이브러리 비도입, 설계 비목표).
// 가로 = 로컬 일, 세로 = 작업시간, 스택 = 에이전트. 세로축 단위(분/시간)는
// 최댓값에 따라 자동. 막대 조각 hover 시 <title>로 상세를 보여준다.
import { useTranslation } from "react-i18next";
import { renderText, type TextKey } from "../shared/textKey";
import type { AgentDailyStat, AgentMeta } from "./aggregate";

interface Props {
  /** x축 로컬 날짜 키("YYYY-MM-DD") 오름차순. */
  days: string[];
  /** 스택 순서(아래→위). 보통 작업시간 내림차순 요약 순서. */
  agents: AgentMeta[];
  /** date → agentId → 셀. */
  daily: Record<string, Record<string, AgentDailyStat>>;
}

const HOUR_MS = 3_600_000;
const MIN_MS = 60_000;

/**
 * 작업시간 표기 키: 1시간 미만은 분, 이상은 시간(소수 1자리). 반올림·자릿수
 * 규칙은 그대로 두고 문구만 카탈로그로 옮겼다 — 표 셀과 차트 y축 눈금이 같은
 * 규칙을 써야 눈금과 값이 어긋나지 않는다.
 */
export function formatDuration(ms: number): TextKey {
  if (ms < HOUR_MS) return { key: "analytics.durMinutes", params: { minutes: Math.round(ms / MIN_MS) } };
  return { key: "analytics.durHours", params: { hours: (ms / HOUR_MS).toFixed(1) } };
}

// SVG 좌표계(px). width는 CSS로 100% 스케일, viewBox로 비율 유지.
const PAD_L = 48;
const PAD_R = 12;
const PAD_T = 10;
const PAD_B = 30;
const PLOT_H = 170;
const BAR_W = 22;
const GAP = 12;

export function DailyBarChart({ days, agents, daily }: Props) {
  const { t } = useTranslation("activity");
  const totals = days.map((date) => {
    const perAgent = daily[date] ?? {};
    return agents.reduce((sum, a) => sum + (perAgent[a.agentId]?.workedMs ?? 0), 0);
  });
  const maxTotal = Math.max(0, ...totals);
  const plotW = days.length * (BAR_W + GAP) + GAP;
  const width = PAD_L + plotW + PAD_R;
  const height = PAD_T + PLOT_H + PAD_B;
  const baseY = PAD_T + PLOT_H;

  // 눈금 4등분. 최댓값이 0이면(방어) 축만 그린다.
  const ticks = maxTotal > 0 ? [0, 0.25, 0.5, 0.75, 1] : [0];

  const scale = (ms: number): number => (maxTotal > 0 ? (PLOT_H * ms) / maxTotal : 0);

  /** 표기 키(TextKey) → 문구. y축 눈금과 막대 툴팁이 같이 쓴다. */
  const t2 = (d: TextKey): string => renderText(d, t);

  return (
    <div className="analytics-chart-scroll">
      <svg
        className="analytics-chart"
        viewBox={`0 0 ${width} ${height}`}
        role="img"
        aria-label={t("analytics.chartAria")}
        preserveAspectRatio="xMinYMin meet"
      >
        {/* y축 눈금선 + 라벨 */}
        {ticks.map((tick) => {
          const y = baseY - PLOT_H * tick;
          return (
            <g key={tick}>
              <line
                className="analytics-chart-grid"
                x1={PAD_L}
                y1={y}
                x2={width - PAD_R}
                y2={y}
              />
              <text className="analytics-chart-ytick" x={PAD_L - 6} y={y + 3} textAnchor="end">
                {t2(formatDuration(maxTotal * tick))}
              </text>
            </g>
          );
        })}

        {/* 막대: 날짜별 스택 */}
        {days.map((date, i) => {
          const x = PAD_L + GAP + i * (BAR_W + GAP);
          const perAgent = daily[date] ?? {};
          let cursorY = baseY;
          return (
            <g key={date}>
              {agents.map((a) => {
                const ms = perAgent[a.agentId]?.workedMs ?? 0;
                if (ms <= 0) return null;
                const h = scale(ms);
                cursorY -= h;
                return (
                  <rect
                    key={a.agentId}
                    x={x}
                    y={cursorY}
                    width={BAR_W}
                    height={h}
                    fill={a.color}
                  >
                    <title>
                      {t("analytics.chartBar", {
                        name: a.name,
                        duration: t2(formatDuration(ms)),
                        date,
                      })}
                    </title>
                  </rect>
                );
              })}
              {/* x축 라벨: 일(DD) */}
              <text
                className="analytics-chart-xtick"
                x={x + BAR_W / 2}
                y={baseY + 16}
                textAnchor="middle"
              >
                {date.slice(8)}
              </text>
            </g>
          );
        })}

        {/* x축 baseline */}
        <line
          className="analytics-chart-axis"
          x1={PAD_L}
          y1={baseY}
          x2={width - PAD_R}
          y2={baseY}
        />
      </svg>
    </div>
  );
}
