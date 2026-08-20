// src/renderer/labels/TaskLabelLayer.tsx
//
// 머리 위 작업 라벨 레이어. 캔버스 위 절대배치 DOM. 텍스트/표시
// 조건은 React(store 셀렉터)로, 위치는 bus의 per-frame 앵커 콜백에서
// style.transform 직접 갱신(리렌더 없음)으로 나눈다.
import { useEffect, useRef } from "react";
import { useAppStore } from "../store/appStore";
import type { LabelAnchor, OfficeBus } from "../office/bus";
import { deriveTaskLabelLines, effectiveCwd } from "./labelText";
import "./labels.css";

const GOAL_FALLBACK_MAX = 24; // 원문 폴백 절단(1줄 목표 자리)
const CURRENT_FALLBACK_MAX = 30; // 원문 폴백 절단(2줄)

export function TaskLabelLayer({ bus }: { bus: OfficeBus }) {
  const agents = useAppStore((s) => s.agents);
  const sessions = useAppStore((s) => s.sessions);
  const taskLabels = useAppStore((s) => s.taskLabels);
  const timeTracking = useAppStore((s) => s.timeTracking);
  // cwd→브랜치(gitBranchWatcher가 30초마다 채운다). 키가 있을 때만 line1의
  // 프로젝트명 뒤에 "(브랜치)"가 붙는다 — 비저장소/detached는 키가 없다.
  const gitBranches = useAppStore((s) => s.gitBranches);
  const elems = useRef(new Map<string, HTMLDivElement>());

  useEffect(
    () =>
      bus.onLabelAnchorsChanged((anchors: ReadonlyMap<string, LabelAnchor>) => {
        for (const [id, el] of elems.current) {
          const a = anchors.get(id);
          if (!a) {
            el.style.visibility = "hidden";
            continue;
          }
          el.style.visibility = "visible";
          el.style.transform = `translate(${Math.round(a.x)}px, ${Math.round(a.y)}px) translate(-50%, -100%)`;
        }
      }),
    [bus]
  );

  const rows = Object.values(agents).flatMap((agent) => {
    const status = sessions[agent.id]?.status;
    if (status !== "starting" && status !== "running") return [];
    const label = taskLabels[agent.id];
    // 두 줄 파생 규칙은 labelText.deriveTaskLabelLines로 일원화(이슈 #44 T1).
    // 표시 결과는 종전과 동일하되 터미널 요약 표시와 규칙을 공유한다.
    const cwd = effectiveCwd(label, agent.cwd);
    const { line1, line2 } = deriveTaskLabelLines(label, agent.cwd, {
      goalMax: GOAL_FALLBACK_MAX,
      currentMax: CURRENT_FALLBACK_MAX,
      branch: cwd ? gitBranches[cwd] : undefined,
    });
    if (!line1 && !line2) return [];
    const phase = timeTracking[agent.id]?.phase ?? "idle";
    return [{ id: agent.id, line1, line2, phase }];
  });

  return (
    <div className="task-label-layer">
      {rows.map((r) => (
        <div
          key={r.id}
          className={`task-label phase-${r.phase}`}
          ref={(el) => {
            if (el) elems.current.set(r.id, el);
            else elems.current.delete(r.id);
          }}
        >
          {r.line1 && <div className="task-label-line1">{r.line1}</div>}
          {r.line2 && <div className="task-label-line2">{r.line2}</div>}
        </div>
      ))}
    </div>
  );
}
