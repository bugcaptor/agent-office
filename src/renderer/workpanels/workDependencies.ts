import type { WorkItem } from "@shared/types";

type Linked = Pick<WorkItem, "id" | "dependsOn">;

/** `from`이 선행 작업을 따라가 `target`에 닿는지. 이미 들어간 순환에도 멈춘다. */
function reaches(from: string, target: string, items: Linked[]): boolean {
  const byId = new Map(items.map((item) => [item.id, item]));
  const seen = new Set<string>();
  const stack = [from];
  while (stack.length) {
    const id = stack.pop()!;
    if (id === target) return true;
    if (seen.has(id)) continue;
    seen.add(id);
    stack.push(...(byId.get(id)?.dependsOn ?? []));
  }
  return false;
}

export type DroppedDependency = { id: string; reason: "self" | "unknown" | "cycle" };

/** `normalizeDependsOn`과 같지만 뺀 선행 작업과 사유도 돌려준다. 중복은 조용히 합친다. */
export function explainDependsOn(id: string | undefined, dependsOn: readonly string[] | undefined, items: Linked[]): { kept?: string[]; dropped: DroppedDependency[] } {
  const known = new Set(items.map((item) => item.id));
  const kept: string[] = [];
  const dropped: DroppedDependency[] = [];
  for (const prerequisite of dependsOn ?? []) {
    if (kept.includes(prerequisite) || dropped.some((d) => d.id === prerequisite)) continue;
    if (prerequisite === id) dropped.push({ id: prerequisite, reason: "self" });
    else if (!known.has(prerequisite)) dropped.push({ id: prerequisite, reason: "unknown" });
    else if (id && reaches(prerequisite, id, items)) dropped.push({ id: prerequisite, reason: "cycle" });
    else kept.push(prerequisite);
  }
  return { kept: kept.length ? kept : undefined, dropped };
}

/** 목록에 있는 id만, 자기 자신·중복·순환을 빼고 남긴다. 비면 undefined. */
export function normalizeDependsOn(id: string | undefined, dependsOn: readonly string[] | undefined, items: Linked[]): string[] | undefined {
  return explainDependsOn(id, dependsOn, items).kept;
}

/** 아직 완료되지 않은 선행 작업. 지워진 업무는 막지 않는다. */
export function pendingPrerequisites(item: Pick<WorkItem, "dependsOn">, items: WorkItem[]): WorkItem[] {
  return (item.dependsOn ?? []).flatMap((id) => items.find((other) => other.id === id) ?? []).filter((other) => other.status !== "done");
}

/** 편집기의 선행 작업 후보: 자신·이미 고른 것·완료된 업무·순환을 만드는 업무를 뺀다. */
export function dependencyCandidates(id: string | undefined, dependsOn: readonly string[] | undefined, items: WorkItem[]): WorkItem[] {
  return items.filter((item) => item.id !== id && item.status !== "done" && !dependsOn?.includes(item.id) && !(id && reaches(item.id, id, items)));
}

/** 이 업무를 기다리는 후속 업무 수. */
export function dependentCount(id: string, items: Linked[]): number {
  return items.filter((item) => item.dependsOn?.includes(id)).length;
}
