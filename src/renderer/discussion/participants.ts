// src/renderer/discussion/participants.ts
//
// 토의 참가자 이름. 이 머신 캐릭터는 프로필 이름, 머신 연동 호스트에서 다른 머신 캐릭터(`기기id:캐릭터id`)는
// `이름 · 머신`(머신 간 이름이 겹쳐도 머신으로 구분한다). 둘 다 아니면 없음(지워진 팀원).
import { useCallback } from "react";
import type { AgentProfile, RemoteCharacter, RemoteMachine, RemoteOffice } from "@shared/types";
import { t } from "../i18n";
import { useAppStore } from "../store/appStore";
import { findRemote, remoteProfile, useRemoteFloors } from "../link/remoteFloors";
import type { AgentProfile as OfficeProfile } from "../office/types";

/** 원격 참가자 id 모양(`기기id:캐릭터id`). 이 머신 캐릭터 id에는 `:`가 없다. */
export function isRemoteParticipantId(agentId: string | undefined): agentId is string {
  return !!agentId && agentId.includes(":");
}
/** 원격 참가자 id → 기기 id. */
export function remoteDeviceId(agentId: string): string { return agentId.slice(0, agentId.indexOf(":")); }

export function remoteParticipant(agentId: string | undefined, office: RemoteOffice = useRemoteFloors.getState().office): { machine: RemoteMachine; character: RemoteCharacter } | null {
  return isRemoteParticipantId(agentId) ? findRemote(office, agentId) : null;
}

/** 참가자 이름. 모르면 undefined. */
export function participantName(
  agentId: string | undefined,
  agents: Readonly<Record<string, AgentProfile>> = useAppStore.getState().agents,
  office: RemoteOffice = useRemoteFloors.getState().office,
): string | undefined {
  if (!agentId) return undefined;
  const local = agents[agentId]?.name.trim();
  if (local) return local;
  const remote = remoteParticipant(agentId, office);
  return remote ? `${remote.character.name} · ${remote.machine.name}` : undefined;
}

/** 이름, 모르면 "지워진 팀원". */
export function participantLabel(agentId: string | undefined): string {
  return participantName(agentId) ?? t("discussion:board.missingAgent");
}

/** 화면용: 이 머신·연동 머신이 바뀌면 다시 그린다. */
export function useParticipantLabel(): (agentId: string | undefined) => string {
  const agents = useAppStore((s) => s.agents);
  const office = useRemoteFloors((s) => s.office);
  return useCallback((agentId) => participantName(agentId, agents, office) ?? t("discussion:board.missingAgent"), [agents, office]);
}

/** 원격 참가자의 오피스 외형(회의실 분신). 이 머신 캐릭터이거나 모르면 undefined. */
export function remoteParticipantProfile(agentId: string, office: RemoteOffice = useRemoteFloors.getState().office): OfficeProfile | undefined {
  const remote = remoteParticipant(agentId, office);
  return remote ? remoteProfile(remote.character) : undefined;
}

/** 토의에 고를 수 있는 다른 머신 캐릭터(퇴근하지 않은). 끊긴 머신 캐릭터도 고를 수는 있다(시작할 때 막는다). */
export function remoteParticipantChoices(office: RemoteOffice): RemoteCharacter[] {
  return office.machines.flatMap((m) => m.characters.filter((c) => !c.clockedOut));
}
