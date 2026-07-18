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
import { useAppStore } from "../store/appStore";
import { deriveTaskLabelLines } from "../labels/labelText";
import "./terminal.css";

const SUMMARY_GOAL_MAX = 60; // 요약 바는 폭이 넉넉하다 — 머리 위 라벨보다 크게.
const SUMMARY_CURRENT_MAX = 90;

export function TerminalSummaryBar() {
  const activeId = useAppStore((s) => s.activeTerminalAgentId);
  const taskLabels = useAppStore((s) => s.taskLabels);
  const sessions = useAppStore((s) => s.sessions);
  const timeTracking = useAppStore((s) => s.timeTracking);
  const agents = useAppStore((s) => s.agents);

  if (!activeId) return null;
  const label = taskLabels[activeId];
  const { line1, line2 } = deriveTaskLabelLines(label, agents[activeId]?.cwd, {
    goalMax: SUMMARY_GOAL_MAX,
    currentMax: SUMMARY_CURRENT_MAX,
  });
  // 표시할 게 없으면(라벨 없음) 바 자체를 렌더하지 않아 레이아웃 점프를 피한다.
  if (!line1 && !line2) return null;

  // 세션이 starting/running이 아니면 실황(line2)은 stale이다 — 목표(line1)만
  // 흐리게 남긴다(완료 세션 탭도 스트립에 남아있다).
  const status = sessions[activeId]?.status;
  const live = status === "starting" || status === "running";
  const phase = timeTracking[activeId]?.phase ?? "idle";
  const shownLine2 = live ? line2 : undefined;

  return (
    <div
      className={`terminal-summary-bar phase-${phase}${live ? "" : " terminal-summary-stale"}`}
    >
      {line1 && <span className="terminal-summary-line1">{line1}</span>}
      {line1 && shownLine2 && <span className="terminal-summary-sep"> — </span>}
      {shownLine2 && <span className="terminal-summary-line2">{shownLine2}</span>}
    </div>
  );
}
