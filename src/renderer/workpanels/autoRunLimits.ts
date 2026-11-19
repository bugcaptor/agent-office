import type { WorkItem } from "@shared/types";

export const AUTO_RUN_LIMITS = {
  maxFollowUps: { min: 0, max: 99, defaultValue: 3 },
  permissionWaitMinutes: { min: 1, max: 1440, defaultValue: 10 },
} as const;
type LimitKey = keyof typeof AUTO_RUN_LIMITS;

export function validLimit(key: LimitKey, value: unknown): value is number {
  const { min, max } = AUTO_RUN_LIMITS[key];
  return typeof value === "number" && Number.isInteger(value) && value >= min && value <= max;
}

export function validWorkLimits(item: Pick<WorkItem, LimitKey>): boolean {
  return (item.maxFollowUps === undefined || validLimit("maxFollowUps", item.maxFollowUps))
    && (item.permissionWaitMinutes === undefined || validLimit("permissionWaitMinutes", item.permissionWaitMinutes));
}

export function effectiveLimit(key: LimitKey, root: WorkItem, task?: WorkItem): number {
  const override = task?.[key], plan = root[key];
  if (validLimit(key, override)) return override;
  return validLimit(key, plan) ? plan : AUTO_RUN_LIMITS[key].defaultValue;
}
