import type { WorkItem } from "@shared/types";

export type WorkItemDraft = Pick<WorkItem, "title" | "body" | "assigneeAgentId" | "agent" | "status">;
export const EMPTY_WORK_DRAFT: WorkItemDraft = { title: "", body: "", status: "todo" };

export function hasWorkContent(item: Pick<WorkItem, "title" | "body">): boolean {
  return !!(item.title.trim() || item.body.trim());
}

/** 자동 제목은 표시할 때만 만든다. 빈 제목을 저장해 본문 수정도 따라간다. */
export function workItemTitle(item: Pick<WorkItem, "title" | "body">): string {
  if (item.title.trim()) return item.title.trim();
  const chars = Array.from(item.body.trim().replace(/\s+/gu, " "));
  return chars.slice(0, 40).join("") + (chars.length > 40 ? "…" : "");
}

export function hasWorkDraftChanges(item: WorkItemDraft, patch: Partial<WorkItemDraft>): boolean {
  return (Object.keys(patch) as (keyof WorkItemDraft)[]).some((key) =>
    JSON.stringify(item[key]) !== JSON.stringify(patch[key]));
}
