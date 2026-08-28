// src/renderer/store/mascotLights.ts
//
// 마스코트 신호등(docs/mascot-lights-design.md) 순수 집계. 마스코트는 "지금
// 알릴 1명"을 표면화하는 이벤트 표면이지만, 신호등은 여러 대상의 현재 상태를
// 상시 노출하는 상태 대시보드다 — 그래서 waiting 취급(§2 결정 1)과 대상 선정
// (결정 2)이 `pickMascotTarget`(selectors.ts)과 일부 다르다. 그 다름은 전부
// 의도된 것이다.
//
// 이 모듈은 스토어 런타임에 의존하지 않는 순수 모듈이다 — 아래 타입들은
// 타입만 필요하므로 반드시 `import type`으로만 가져와 순수성을 유지한다
// (labelText.ts:15-18과 같은 규칙).
import type { MascotLight, MascotLightAvatar, MascotLightState } from "../mascot/protocol";
import type { ColorOverrides } from "@shared/types";
import { effectiveCwd, isInsideCwd, projectNameFromCwd } from "../labels/labelText";
import type { AgentTaskLabel } from "./types";
import type { TurnPhase } from "../timeline/turnReducer";

export interface ComputeMascotLightsInput {
  mode: "off" | "agents" | "projects";
  /** 프로젝트 모드에서 칸을 받을 폴더 목록. 표시 순서 = 이 배열 순서. */
  projects: ReadonlyArray<string>;
  /** 근무 중 에이전트 순회 순서(생성 순). */
  agentOrder: ReadonlyArray<string>;
  agents: Record<
    string,
    | {
        name?: string;
        cwd?: string;
        clockedOut?: boolean;
        /** 칸에 띄울 얼굴을 다시 만드는 데 필요한 스프라이트 좌표(§6). */
        seed?: string;
        archetype?: string;
        colors?: ColorOverrides;
        spriteUpdatedAt?: number;
      }
    | undefined
  >;
  timeTracking: Record<
    string,
    { phase: TurnPhase; turnStartedAt: number | null; waitingSince: number | null } | undefined
  >;
  /** newest-first(appStore.pushNotification이 앞에 붙인다). */
  notifications: ReadonlyArray<{ agentId: string }>;
  taskLabels: Record<string, AgentTaskLabel | undefined>;
}

/**
 * 에이전트 하나의 원자 상태(§3). `attention`이 `working`보다 우선한다 —
 * waiting 중에도 백그라운드 tool이 돌 수 있으나, 사용자 응답 대기가 더 급한
 * 정보라는 사용자 확정(결정 1).
 */
function atomicState(
  id: string,
  timeTracking: ComputeMascotLightsInput["timeTracking"],
  pending: ReadonlySet<string>,
): MascotLightState {
  const tt = timeTracking[id];
  if (pending.has(id) || tt?.phase === "waiting") return "attention";
  if (tt?.phase === "working") return "working";
  return "off";
}

/**
 * project 폴더에 소속된(근무 중, 퇴근하지 않은) 에이전트 id들 — `agentOrder`
 * 순서 보존(대표 선정 ④단계가 "agentOrder 첫째"를 요구한다). 소속 판정은
 * `isInsideCwd`(labelText.ts) — 세션 실효 cwd가 project와 같거나 그 하위.
 */
function membersOf(project: string, input: ComputeMascotLightsInput): string[] {
  const members: string[] = [];
  for (const id of input.agentOrder) {
    const agent = input.agents[id];
    if (!agent || agent.clockedOut) continue;
    const cwd = effectiveCwd(input.taskLabels[id], agent.cwd);
    if (cwd === undefined) continue;
    if (isInsideCwd(cwd, project)) members.push(id);
  }
  return members;
}

/** 소속 에이전트들 상태의 max(attention > working > off, 결정 3). */
function aggregateState(
  members: readonly string[],
  timeTracking: ComputeMascotLightsInput["timeTracking"],
  pending: ReadonlySet<string>,
): MascotLightState {
  let best: MascotLightState = "off";
  for (const id of members) {
    const state = atomicState(id, timeTracking, pending);
    if (state === "attention") return "attention";
    if (state === "working") best = "working";
  }
  return best;
}

/**
 * 프로젝트 칸의 대표 에이전트(클릭 시 활성화 대상, 결정 7 4단계):
 * ① pending 최신(알림 newest-first에서 소속인 첫째) ② waiting 중
 * waitingSince 최신 ③ working 중 turnStartedAt 최신 ④ 소속 중 agentOrder
 * 첫째. 소속이 없으면 null(클릭 no-op).
 */
function representativeOf(
  members: readonly string[],
  input: ComputeMascotLightsInput,
): string | null {
  if (members.length === 0) return null;
  const memberSet = new Set(members);
  for (const n of input.notifications) {
    if (memberSet.has(n.agentId)) return n.agentId;
  }
  let bestWaiting: string | null = null;
  let bestWaitingAt = -Infinity;
  for (const id of members) {
    const tt = input.timeTracking[id];
    if (tt?.phase === "waiting") {
      const at = tt.waitingSince ?? 0;
      if (at > bestWaitingAt) {
        bestWaitingAt = at;
        bestWaiting = id;
      }
    }
  }
  if (bestWaiting !== null) return bestWaiting;
  let bestWorking: string | null = null;
  let bestWorkingAt = -Infinity;
  for (const id of members) {
    const tt = input.timeTracking[id];
    if (tt?.phase === "working") {
      const at = tt.turnStartedAt ?? 0;
      if (at > bestWorkingAt) {
        bestWorkingAt = at;
        bestWorking = id;
      }
    }
  }
  if (bestWorking !== null) return bestWorking;
  return members[0];
}

/**
 * 칸에 띄울 얼굴 좌표. 대표가 없거나(세션 없는 폴더) 프로필이 사라졌으면 null
 * — 그 칸은 이름 첫 글자 원판으로 대체된다.
 */
function avatarOf(
  agentId: string | null,
  agents: ComputeMascotLightsInput["agents"],
): MascotLightAvatar | null {
  if (agentId === null) return null;
  const agent = agents[agentId];
  if (!agent) return null;
  return {
    agentId,
    seed: agent.seed || agentId,
    archetype: agent.archetype ?? null,
    colors: agent.colors ?? null,
    spriteUpdatedAt: agent.spriteUpdatedAt ?? null,
  };
}

/**
 * 신호등 칸 목록 집계(§3). `mode==="off"`이거나 결과가 0칸이면 빈 배열 —
 * mascotBridge는 이 배열이 비어 있으면 strip 자체가 없는 것으로 취급한다.
 *
 * - agents 모드: 원자 상태가 `off`가 아닌 근무 중 에이전트만, `agentOrder`
 *   순서(결정 2 — 일이 없으면 칸이 사라지고 뒤 칸이 앞으로 밀려온다).
 * - projects 모드: `projects` 순서 유지, 폴더당 정확히 1칸. 소속 에이전트가
 *   없어도(세션 없는 repo) 칸은 유지되고 상태는 `off`다 — off 필터를 적용하지
 *   않는다.
 */
export function computeMascotLights(input: ComputeMascotLightsInput): MascotLight[] {
  if (input.mode === "off") return [];
  const pending = new Set(input.notifications.map((n) => n.agentId));

  if (input.mode === "agents") {
    const out: MascotLight[] = [];
    for (const id of input.agentOrder) {
      const agent = input.agents[id];
      if (!agent || agent.clockedOut) continue;
      const state = atomicState(id, input.timeTracking, pending);
      if (state === "off") continue;
      out.push({
        id,
        label: agent.name ?? id,
        state,
        clickAgentId: id,
        avatar: avatarOf(id, input.agents),
      });
    }
    return out;
  }

  // E1: settings.json을 직접 편집해 빈 문자열/중복 경로가 들어올 수 있다.
  // 빈 문자열은 isInsideCwd(cwd, "")가 모든 절대경로에 true라 전체 세션을
  // 그 칸 하나로 삼켜 버리고, 중복 경로는 React key(light.id) 충돌을 낳아
  // 경고는 물론 클릭 대상까지 헷갈리게 한다 — 둘 다 여기서 걸러낸다.
  const seen = new Set<string>();
  const projects: string[] = [];
  for (const project of input.projects) {
    if (project.trim() === "" || seen.has(project)) continue;
    seen.add(project);
    projects.push(project);
  }

  return projects.map((project) => {
    const members = membersOf(project, input);
    const representative = representativeOf(members, input);
    return {
      id: project,
      label: projectNameFromCwd(project) ?? project,
      state: aggregateState(members, input.timeTracking, pending),
      clickAgentId: representative,
      // 얼굴은 클릭 대상과 같은 에이전트다 — "이 칸을 누르면 누가 나오나"를
      // 그림으로 미리 보여 준다(설계 §6 개정).
      avatar: avatarOf(representative, input.agents),
    };
  });
}
