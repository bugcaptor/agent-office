import type { WorkItem } from "@shared/types";
import { planItems } from "./autoRun";

/** 업무 목록에서 플랜 머리로 묶는 루트와 그 아래에 들여 보일 업무. */
export interface PlanLayout {
  /** 루트 id → 그 아래에 보일 플랜 업무(진행 순서). */
  members: Map<string, WorkItem[]>;
  /** 어느 루트 아래로 옮겨져 제 묶음에서는 빠지는 업무 id. */
  claimed: Set<string>;
}

/**
 * 플랜 루트: 완료되지 않았고, 선행 작업이 있고, 이 업무를 기다리는 업무가 없는(사슬 맨 끝) 업무.
 * 루트 아래에는 같은 묶음(`keyOf`)이거나 미배정("")인 플랜 업무만 옮긴다. 자동 진행이 보낼 때
 * 진행자에게 배정하는 업무가 그 둘이기 때문이다. 다른 캐릭터 업무는 제 묶음에 남는다.
 * 한 업무가 여러 플랜에 걸리면 패널 순서가 앞선 루트가 가져간다.
 */
export function planLayout(items: WorkItem[], keyOf: (item: WorkItem) => string): PlanLayout {
  const waitedOn = new Set(items.flatMap((item) => item.dependsOn ?? []));
  const roots = items.filter((item) => item.status !== "done" && item.dependsOn?.length && !waitedOn.has(item.id)).sort((a, b) => a.order - b.order);
  const members = new Map<string, WorkItem[]>();
  const claimed = new Set<string>();
  for (const root of roots) {
    const plan = planItems(root, items);
    if (!plan.length) continue;
    const key = keyOf(root);
    const moved = plan.filter((item) => !claimed.has(item.id) && (keyOf(item) === key || keyOf(item) === ""));
    for (const item of moved) claimed.add(item.id);
    members.set(root.id, moved);
  }
  return { members, claimed };
}
