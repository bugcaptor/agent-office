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
import type { ColorOverrides, MascotLightsLabel } from "@shared/types";
import {
  effectiveCwd,
  isInsideCwd,
  projectAnchorCwd,
  projectNameFromCwd,
  requestSentence,
  truncateChars,
} from "../labels/labelText";
import type { AgentTaskLabel } from "./types";
import type { TurnPhase } from "../timeline/turnReducer";

/** `mascotLightsLabel==="task"`일 때 작업명 절단 길이(chars). */
const TASK_LABEL_MAX_CHARS = 60;

export interface ComputeMascotLightsInput {
  mode: "off" | "agents" | "projects";
  /** 칸에 표시할 텍스트 선택(§7 개정). 고른 값이 그 칸에서 비어 있으면 auto로
   *  폴백한다(빈 칸 방지). */
  labelMode: MascotLightsLabel;
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
        /** 초상 존재 표시 + 캐시 무효화 키(§6 개정 — 칸 얼굴 스프라이트/초상 선택). */
        portraitUpdatedAt?: number;
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
    portraitUpdatedAt: agent.portraitUpdatedAt ?? null,
  };
}

/** 에이전트 이름 — 없으면 id로 폴백(항상 비어 있지 않다, 값이 없는 label의
 *  최종 폴백 대상이기도 하다). */
function agentNameOf(agentId: string, agents: ComputeMascotLightsInput["agents"]): string {
  return agents[agentId]?.name ?? agentId;
}

/**
 * 에이전트의 프로젝트명(정체성 앵커 기준, labelText.ts와 같은 규칙) —
 * `projectAnchorCwd(세션 cwd, 프로필 cwd)`의 basename. 둘 다 없으면 undefined.
 */
function agentProjectNameOf(
  agentId: string,
  agents: ComputeMascotLightsInput["agents"],
  taskLabels: ComputeMascotLightsInput["taskLabels"],
): string | undefined {
  const agent = agents[agentId];
  return projectNameFromCwd(projectAnchorCwd(taskLabels[agentId]?.cwd, agent?.cwd));
}

/**
 * 에이전트의 작업명(목표 > 저장된 요청 폴백 > 첫 프롬프트 요청 문장) —
 * `TASK_LABEL_MAX_CHARS`로 절단. 셋 다 없으면 undefined.
 */
function agentTaskNameOf(
  agentId: string,
  taskLabels: ComputeMascotLightsInput["taskLabels"],
): string | undefined {
  const label = taskLabels[agentId];
  const text = label?.goal ?? label?.goalFallback ?? requestSentence(label?.firstPromptText);
  return text ? truncateChars(text, TASK_LABEL_MAX_CHARS) : undefined;
}

/**
 * agents 모드 칸 하나의 표시 텍스트(§7 개정) — `label`은 `labelMode`로 고른
 * 값, 비어 있으면(cwd/작업 없음) `auto`(에이전트 이름)로 폴백한다. `tooltip`은
 * labelMode와 무관하게 [이름, 프로젝트명, 작업명] 중 있는 것만 이어 붙인다.
 * `projecttask`는 둘째 줄용 `sublabel`(작업명)을 함께 채운다 — 없으면 null(칸
 * 높이는 tall 모드라 그대로 유지되고 둘째 줄만 비게 그린다).
 */
function agentLightText(
  agentId: string,
  labelMode: MascotLightsLabel,
  input: ComputeMascotLightsInput,
): { label: string; sublabel: string | null; tooltip: string } {
  const name = agentNameOf(agentId, input.agents);
  const project = agentProjectNameOf(agentId, input.agents, input.taskLabels);
  const task = agentTaskNameOf(agentId, input.taskLabels);
  if (labelMode === "projecttask") {
    return {
      label: project ?? name,
      sublabel: task ?? null,
      tooltip: [name, project, task].filter(Boolean).join(" · "),
    };
  }
  const chosen =
    labelMode === "project" ? project : labelMode === "task" ? task : name; // auto/agent 둘 다 이름
  return {
    label: chosen ?? name,
    sublabel: null,
    tooltip: [name, project, task].filter(Boolean).join(" · "),
  };
}

/**
 * projects 모드 칸 하나의 표시 텍스트(§7 개정) — `label`은 `labelMode`로 고른
 * 값, 비어 있으면(대표 없음 등) `auto`(폴더 basename)로 폴백한다. `tooltip`
 * 순서는 agentLightText와 동일하게 [대표 이름, 프로젝트명, 작업명]. `projecttask`는
 * 둘째 줄용 `sublabel`(대표 에이전트 작업명)을 함께 채운다 — 없으면 null.
 */
function projectLightText(
  project: string,
  representative: string | null,
  labelMode: MascotLightsLabel,
  input: ComputeMascotLightsInput,
): { label: string; sublabel: string | null; tooltip: string } {
  const folderName = projectNameFromCwd(project) ?? project;
  const repName = representative !== null ? agentNameOf(representative, input.agents) : undefined;
  const repTask =
    representative !== null ? agentTaskNameOf(representative, input.taskLabels) : undefined;
  if (labelMode === "projecttask") {
    return {
      label: folderName,
      sublabel: repTask ?? null,
      tooltip: [repName, folderName, repTask].filter(Boolean).join(" · "),
    };
  }
  const chosen =
    labelMode === "agent" ? repName : labelMode === "task" ? repTask : folderName; // auto/project 둘 다 폴더명
  return {
    label: chosen ?? folderName,
    sublabel: null,
    tooltip: [repName, folderName, repTask].filter(Boolean).join(" · "),
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
      const { label, sublabel, tooltip } = agentLightText(id, input.labelMode, input);
      out.push({
        id,
        label,
        sublabel,
        tooltip,
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
    const { label, sublabel, tooltip } = projectLightText(
      project,
      representative,
      input.labelMode,
      input,
    );
    return {
      id: project,
      label,
      sublabel,
      tooltip,
      state: aggregateState(members, input.timeTracking, pending),
      clickAgentId: representative,
      // 얼굴은 클릭 대상과 같은 에이전트다 — "이 칸을 누르면 누가 나오나"를
      // 그림으로 미리 보여 준다(설계 §6 개정).
      avatar: avatarOf(representative, input.agents),
    };
  });
}
