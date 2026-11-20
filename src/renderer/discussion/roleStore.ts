// src/renderer/discussion/roleStore.ts
//
// 토의 역할 목록(`roles.json`)의 렌더러 캐시. 목록 전체를 한 번에 저장한다.
//
// 프리셋은 파일이 **없을 때 한 번만** 그 순간의 화면 언어로 실체화해 저장한다.
// 이후에는 일반 역할과 같아 고치거나 지울 수 있고, 다 지운 빈 목록에도 되살아나지
// 않는다. 손상 파일(로드 reject)이면 시드·저장 모두 하지 않는다.
import { create } from "zustand";
import { nanoid } from "nanoid";
import { t } from "@renderer/i18n";
import { ROLES_SCHEMA_VERSION, type DiscussionRole, type DiscussionStance } from "@shared/types";
import { tauriApi } from "../ipc/tauriApi";

export const ROLE_PRESETS: readonly { key: string; stance: DiscussionStance }[] = [
  { key: "moderator", stance: "moderator" },
  { key: "critic", stance: "critical" },
  { key: "devil", stance: "devils_advocate" },
  { key: "user", stance: "free" },
  { key: "maintainer", stance: "free" },
  { key: "ideation", stance: "divergent" },
];

export const presetRoleId = (key: string) => `preset-${key}`;

/** 프리셋 하나를 현재 언어 문구로 만든다. `id`를 주지 않으면 프리셋 id. */
export function presetRole(key: string, id = presetRoleId(key)): DiscussionRole {
  const preset = ROLE_PRESETS.find((p) => p.key === key);
  if (!preset) throw new Error(`unknown role preset: ${key}`);
  return {
    id,
    name: t(`discussion:presets.${key}.name`),
    perspective: t(`discussion:presets.${key}.perspective`),
    stance: preset.stance,
    criteria: t(`discussion:presets.${key}.criteria`),
  };
}

export function presetRoles(): DiscussionRole[] {
  return ROLE_PRESETS.map((p) => presetRole(p.key));
}

/** 목록에 새로 넣을 고유 id. */
export const newRoleId = () => `role-${nanoid(8)}`;

/** 저장 전에 공백을 다듬는다. 빈 이름이 있으면 null(저장 불가). */
export function normalizeRoles(roles: DiscussionRole[]): DiscussionRole[] | null {
  const out = roles.map((r) => ({
    ...r,
    name: r.name.trim(),
    perspective: r.perspective.trim(),
    criteria: r.criteria.trim(),
  }));
  return out.some((r) => !r.name) ? null : out;
}

type Status = "idle" | "loading" | "ready" | "error";

interface RoleState {
  roles: DiscussionRole[];
  status: Status;
  /** 마지막 로드·저장 실패 메시지. */
  error?: string;
  /** 아직 불러오지 않았거나 실패했으면 불러온다. 진행 중이면 같은 약속을 기다린다. */
  ensureLoaded(): Promise<void>;
  load(): Promise<void>;
  save(roles: DiscussionRole[]): Promise<void>;
}

let inflight: Promise<void> | null = null;

export const useRoleStore = create<RoleState>((set, get) => ({
  roles: [],
  status: "idle",
  ensureLoaded: () => (get().status === "ready" ? Promise.resolve() : get().load()),
  load: () => {
    if (inflight) return inflight;
    set({ status: "loading", error: undefined });
    inflight = (async () => {
      try {
        const file = await tauriApi.loadRoles();
        if (file) {
          set({ roles: file.roles, status: "ready" });
          return;
        }
        const roles = presetRoles();
        await tauriApi.saveRoles({ version: ROLES_SCHEMA_VERSION, roles });
        set({ roles, status: "ready" });
      } catch (e) {
        set({ status: "error", error: String(e) });
      } finally {
        inflight = null;
      }
    })();
    return inflight;
  },
  save: async (roles) => {
    await tauriApi.saveRoles({ version: ROLES_SCHEMA_VERSION, roles });
    set({ roles, status: "ready", error: undefined });
  },
}));

/** 테스트용 초기화. */
export function resetRoleStoreForTest(): void {
  inflight = null;
  useRoleStore.setState({ roles: [], status: "idle", error: undefined });
}
