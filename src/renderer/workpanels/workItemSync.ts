import type { WorkItem } from "@shared/types";

/** JSON 값 비교. 키 순서와 생략된 undefined는 저장 형식의 차이일 뿐이다. */
export function sameWorkValue(a: unknown, b: unknown): boolean {
  if (a === b) return true;
  if (!a || !b || typeof a !== "object" || typeof b !== "object" || Array.isArray(a) !== Array.isArray(b)) return false;
  if (Array.isArray(a) && Array.isArray(b)) return a.length === b.length && a.every((value, index) => sameWorkValue(value, b[index]));
  const left = a as Record<string, unknown>, right = b as Record<string, unknown>;
  return [...new Set([...Object.keys(left), ...Object.keys(right)])].every((key) => sameWorkValue(left[key], right[key]));
}

/**
 * 마지막으로 읽거나 저장한 스냅샷을 기준으로 외부 변경과 아직 저장하지 않은 앱 편집을 합친다.
 * 앱에서 바꾼 필드만 우선하므로 자동 진행 상태 저장이 외부의 새 태스크·플랜 연결을 지우지 않는다.
 * 삭제도 반영하되, 외부에서 삭제하는 동안 앱에서 편집한 태스크는 보존한다.
 */
export function mergeWorkItems(base: WorkItem[], local: WorkItem[], remote: WorkItem[]): WorkItem[] {
  const before = new Map(base.map((item) => [item.id, item]));
  const current = new Map(local.map((item) => [item.id, item]));
  const incoming = new Set(remote.map((item) => item.id));
  const merged: WorkItem[] = [];
  for (const item of remote) {
    const old = before.get(item.id), edited = current.get(item.id);
    if (!edited) { if (!old) merged.push(item); continue; }
    let next = item;
    if (!old) next = edited;
    else if (!sameWorkValue(old, edited)) {
      next = { ...item };
      for (const key of new Set([...Object.keys(old), ...Object.keys(edited)]) as Set<keyof WorkItem>) {
        if (!sameWorkValue(old[key], edited[key])) Object.assign(next, { [key]: edited[key] });
      }
      next.updatedAt = Math.max(item.updatedAt, edited.updatedAt);
    }
    merged.push(sameWorkValue(next, edited) ? edited : next);
  }
  for (const item of local) {
    if (!incoming.has(item.id) && (!before.has(item.id) || !sameWorkValue(before.get(item.id), item))) merged.push(item);
  }
  return sameWorkValue(merged, local) ? local : merged;
}
