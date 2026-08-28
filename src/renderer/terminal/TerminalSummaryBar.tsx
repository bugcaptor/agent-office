// src/renderer/terminal/TerminalSummaryBar.tsx
//
// 활성 탭 요약 바(이슈 #44 T1). 터미널을 열었을 때도 캐릭터가 "지금 무슨
// 일을 하는지"를 머리 위 라벨과 같은 데이터(taskLabels)로 한 줄 보여준다.
// TerminalOverlay 패널 안, 탭 스트립과 keep-alive 호스트 사이에 마운트되며
// 오버레이 닫힘(display:none)과 무관하게 항상 마운트된다 — 표시 토글은 부모
// 담당(키프얼라이브 불변식). 파생 규칙은 labelText.deriveTaskLabelLines로
// 머리 위 라벨과 공유한다.
//
// useShallow 함정 주의: 스토어 구독은 원본 참조 그대로 가져오고(새 객체
// 리터럴 금지) 파생은 렌더에서 한다(AgentTabStrip 헤더 주석 참조).
//
// 오른쪽 끝의 사용량 스팬(토큰·비용, docs/session-analytics-design.md §11)은
// 라벨과 별개 데이터라 라벨 유무와 무관하게 뜬다 — 라벨 없이 사용량만 있어도
// 바는 숨지 않는다(자리 유지 불변식은 "라벨도 사용량도 둘 다 없을 때"만).
import { useTranslation } from "react-i18next";
import { useAppStore } from "../store/appStore";
import { deriveTaskLabelLines, effectiveCwd } from "../labels/labelText";
import { formatTokens, formatUsd } from "../analytics/pricing";
import { mergeTotals, type SessionUsageTotals } from "../usage/sessionCost";
import { useSessionUsageSeed } from "../usage/useSessionUsageSeed";
import type { Translate } from "../shared/textKey";
import "./terminal.css";

const SUMMARY_GOAL_MAX = 60; // 요약 바는 폭이 넉넉하다 — 머리 위 라벨보다 크게.
const SUMMARY_CURRENT_MAX = 90;

/**
 * 사용량 스팬의 title 툴팁: 입력/출력/캐시읽기/캐시기록(0인 항목은 생략) ·
 * 턴 수 · 대표 모델, `~` 표시가 있으면 그 뜻(단가를 모르는 모델 N턴은 제외)을
 * 한 줄 더 얹고, 마지막 줄에 추정치 각주. analytics 패널의 tokenBreakdown·
 * costUnknownHint와 같은 결.
 */
function usageTooltip(totals: SessionUsageTotals, t: Translate): string {
  const parts: string[] = [];
  if (totals.input > 0) parts.push(t("summary.usage.tokenIn", { value: formatTokens(totals.input) }));
  if (totals.output > 0) parts.push(t("summary.usage.tokenOut", { value: formatTokens(totals.output) }));
  if (totals.cacheRead > 0)
    parts.push(t("summary.usage.tokenCacheRead", { value: formatTokens(totals.cacheRead) }));
  if (totals.cacheWrite > 0)
    parts.push(t("summary.usage.tokenCacheWrite", { value: formatTokens(totals.cacheWrite) }));
  parts.push(t("summary.usage.turns", { count: totals.turns }));
  if (totals.model) parts.push(t("summary.usage.model", { model: totals.model }));
  const lines = [parts.join(" · ")];
  // costUnknownTurns > 0이면 셀에 "~"가 붙는데, 그 뜻이 어디에도 안 나오면
  // 사용자가 그게 뭔지 알 길이 없다 — 여기 한 줄로 명시.
  if (totals.costUnknownTurns > 0) {
    lines.push(t("summary.usage.costUnknownHint", { count: totals.costUnknownTurns }));
  }
  lines.push(t("summary.usage.footnote"));
  return lines.join("\n");
}

/**
 * 토큰 셀 텍스트. `input + output === 0`인데 `cacheRead > 0`이면("캐시만 있는
 * 턴") "0"이 아니라 "—"로 떨군다 — analytics 패널의 `tokenTotal > 0 ? … : "—"`
 * 와 같은 결.
 */
function usageTokenText(totals: SessionUsageTotals): string {
  const total = totals.input + totals.output;
  if (total === 0 && totals.cacheRead > 0) return "—";
  return formatTokens(total);
}

/**
 * 비용 셀 텍스트. 실린 턴 전부가 단가를 모르면(`costUnknownTurns === turns`)
 * "$0.0000"처럼 공짜로 보이는 대신 "—"로 떨군다. 일부만 모르면(0 < unknown <
 * turns) 지금처럼 `~` 접두 — 뜻은 `usageTooltip`의 `costUnknownHint`로.
 */
function usageCostText(totals: SessionUsageTotals): string {
  if (totals.costUnknownTurns === totals.turns) return "—";
  return `${totals.costUnknownTurns > 0 ? "~" : ""}${formatUsd(totals.costUsd)}`;
}

export function TerminalSummaryBar() {
  const { t } = useTranslation("terminal");
  const activeId = useAppStore((s) => s.activeTerminalAgentId);
  const taskLabels = useAppStore((s) => s.taskLabels);
  const sessions = useAppStore((s) => s.sessions);
  const timeTracking = useAppStore((s) => s.timeTracking);
  const agents = useAppStore((s) => s.agents);
  // 머리 위 라벨과 같은 cwd→브랜치 맵(gitBranchWatcher가 채운다).
  const gitBranches = useAppStore((s) => s.gitBranches);
  const sessionCostEnabled = useAppStore((s) => s.appSettings.sessionCostEnabled);
  const sessionUsage = useAppStore((s) => s.sessionUsage);
  const sessionUsageSeed = useAppStore((s) => s.sessionUsageSeed);
  useSessionUsageSeed(); // 앱 수명당 1회, 설정 꺼짐이면 내부에서 no-op.

  if (!activeId) return null;
  const label = taskLabels[activeId];
  const cwd = effectiveCwd(label, agents[activeId]?.cwd);
  const { line1, line2 } = deriveTaskLabelLines(label, agents[activeId]?.cwd, {
    goalMax: SUMMARY_GOAL_MAX,
    currentMax: SUMMARY_CURRENT_MAX,
    branch: cwd ? gitBranches[cwd] : undefined,
  });
  // 세션이 starting/running이 아니면 실황(line2)은 stale이다 — 목표(line1)만
  // 흐리게 남긴다(완료 세션 탭도 스트립에 남아있다).
  const status = sessions[activeId]?.status;
  const live = status === "starting" || status === "running";
  const phase = timeTracking[activeId]?.phase ?? "idle";
  const shownLine2 = live ? line2 : undefined;

  // 사용량: 실시간 누계(sessionUsage) + 과거 시드(sessionUsageSeed, 같은
  // sessionId일 때만)를 합친다. turns===0(집계된 턴 없음)이면 표시하지 않는다.
  const usageEntry = sessionUsage[activeId];
  const seedTotals = usageEntry ? sessionUsageSeed?.bySession[usageEntry.sessionId] : undefined;
  const totals = usageEntry
    ? seedTotals
      ? mergeTotals(seedTotals, usageEntry.totals)
      : usageEntry.totals
    : undefined;
  const hasUsage = sessionCostEnabled && !!totals && totals.turns > 0;

  // 표시할 게 없어도(라벨·사용량 둘 다 없음) 바를 *자리만* 남긴 채 숨긴다 —
  // 예전에는 null을 반환했는데, 그러면 첫 프롬프트로 라벨이 생기는 순간 패널
  // flex 열의 높이가 22px 바뀌어 xterm rows가 줄고 PTY resize가 나간다. pi의
  // 기본 TUI(regular)는 resize마다 `ESC[2J ESC[H ESC[3J`로 화면+**스크롤백**을
  // 지우고 다시 그리므로(pi v0.84.2 PTY 실측), 하필 pi가 막 일을 시작한
  // 시점에 터미널 히스토리가 통째로 날아간다. 자리를 항상 잡아 두면 그
  // resize 자체가 사라진다.
  if (!line1 && !line2 && !hasUsage) {
    return <div className="terminal-summary-bar terminal-summary-hidden" aria-hidden="true" />;
  }

  return (
    <div
      className={`terminal-summary-bar phase-${phase}${live ? "" : " terminal-summary-stale"}`}
    >
      {line1 && <span className="terminal-summary-line1">{line1}</span>}
      {line1 && shownLine2 && <span className="terminal-summary-sep"> — </span>}
      {shownLine2 && <span className="terminal-summary-line2">{shownLine2}</span>}
      {hasUsage && (
        <span className="terminal-summary-usage" title={usageTooltip(totals!, t)}>
          {`${usageTokenText(totals!)} · ${usageCostText(totals!)}`}
        </span>
      )}
    </div>
  );
}
