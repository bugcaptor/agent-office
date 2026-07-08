// src/renderer/labels/TaskLabelLayer.tsx
//
// 머리 위 작업 라벨 레이어. 캔버스 위 절대배치 DOM. 텍스트/표시
// 조건은 React(store 셀렉터)로, 위치는 bus의 per-frame 앵커 콜백에서
// style.transform 직접 갱신(리렌더 없음)으로 나눈다.
import { useEffect, useRef } from "react";
import { useAppStore } from "../store/appStore";
import type { LabelAnchor, OfficeBus } from "../office/bus";
import { firstLine, projectNameFromCwd } from "./labelText";
import "./labels.css";

const GOAL_FALLBACK_MAX = 24; // 원문 폴백 절단(1줄 목표 자리)
const CURRENT_FALLBACK_MAX = 30; // 원문 폴백 절단(2줄)

export function TaskLabelLayer({ bus }: { bus: OfficeBus }) {
  const agents = useAppStore((s) => s.agents);
  const sessions = useAppStore((s) => s.sessions);
  const taskLabels = useAppStore((s) => s.taskLabels);
  const timeTracking = useAppStore((s) => s.timeTracking);
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
    const project = projectNameFromCwd(agent.cwd);
    const goal = label?.goal ?? firstLine(label?.firstPromptText, GOAL_FALLBACK_MAX);
    const line1 = [project, goal].filter(Boolean).join(" · ");
    const line2 = label?.currentSummary ?? firstLine(label?.latestPromptText, CURRENT_FALLBACK_MAX);
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
