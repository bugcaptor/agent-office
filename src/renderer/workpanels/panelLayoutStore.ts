import { create } from "zustand";
export type PanelSide = "left" | "right";
interface PanelLayout { width: number; collapsed: boolean }
const KEY = "agent-office.work-panels.v1";
export function normalizePanelLayout(raw: unknown): PanelLayout {
  const p = raw as Partial<PanelLayout> | null;
  return { width: typeof p?.width === "number" && Number.isFinite(p.width) ? Math.max(220, Math.min(480, p.width)) : 280, collapsed: p?.collapsed === true };
}
function load(): Record<PanelSide, PanelLayout> {
  try { const value = JSON.parse(localStorage.getItem(KEY) ?? "{}"); return { left: normalizePanelLayout(value?.left), right: normalizePanelLayout(value?.right) }; }
  catch { return { left: normalizePanelLayout(null), right: normalizePanelLayout(null) }; }
}
export const usePanelLayoutStore = create<{ panels: Record<PanelSide, PanelLayout>; update(side: PanelSide, patch: Partial<PanelLayout>): void }>((set, get) => ({
  panels: load(),
  update(side, patch) {
    const panels = { ...get().panels, [side]: normalizePanelLayout({ ...get().panels[side], ...patch }) };
    set({ panels });
    try { localStorage.setItem(KEY, JSON.stringify(panels)); } catch { /* 기억 불가 환경에서도 조절 가능 */ }
  },
}));
export function panelExtent(panel: PanelLayout): number { return panel.collapsed ? 24 : panel.width; }

/** 업무 목록의 캐릭터 묶음 중 접어 둔 것. 키는 `assigneeKey`(미배정 "", 삭제된 캐릭터 묶음 포함). */
const GROUPS_KEY = "agent-office.work-groups.v1";
function loadCollapsedGroups(): string[] {
  try { const value: unknown = JSON.parse(localStorage.getItem(GROUPS_KEY) ?? "[]"); return Array.isArray(value) ? value.filter((key): key is string => typeof key === "string") : []; }
  catch { return []; }
}
export const useWorkGroupStore = create<{ collapsed: string[]; setCollapsed(key: string, collapsed: boolean): void }>((set, get) => ({
  collapsed: loadCollapsedGroups(),
  setCollapsed(key, collapsed) {
    if (get().collapsed.includes(key) === collapsed) return;
    const next = collapsed ? [...get().collapsed, key] : get().collapsed.filter((k) => k !== key);
    set({ collapsed: next });
    try { localStorage.setItem(GROUPS_KEY, JSON.stringify(next)); } catch { /* 기억 불가 환경에서도 접기 가능 */ }
  },
}));
