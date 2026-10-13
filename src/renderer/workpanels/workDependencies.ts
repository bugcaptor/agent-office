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

/** 목록에 있는 id만, 자기 자신·중복·순환을 빼고 남긴다. 비면 undefined. */
export function normalizeDependsOn(id: string | undefined, dependsOn: readonly string[] | undefined, items: Linked[]): string[] | undefined {
  const known = new Set(items.map((item) => item.id));
  const kept: string[] = [];
  for (const prerequisite of dependsOn ?? []) {
    if (prerequisite === id || !known.has(prerequisite) || kept.includes(prerequisite)) continue;
    if (id && reaches(prerequisite, id, items)) continue;
    kept.push(prerequisite);
  }
  return kept.length ? kept : undefined;
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
