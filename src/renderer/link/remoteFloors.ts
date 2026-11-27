// src/renderer/link/remoteFloors.ts
//
// 머신 연동 호스트: 붙은 머신들의 캐릭터(`link-remote` 이벤트)와 오피스에서 지금 보는 층. 층 하나가 머신
// 하나다 — 이 머신 층(null)과 다른 머신 층(기기 id). 다른 층의 캐릭터를 누르면 그 머신에 물어 대화 보기를 연다
// (`RemoteChatDialog`, 입력·권한 답·세션 조작은 그 머신에 넘긴다).
import { create } from "zustand";
import { listen } from "@tauri-apps/api/event";
import { Events } from "@shared/ipc";
import type { RemoteCharacter, RemoteMachine, RemoteOffice } from "@shared/types";
import type { AgentProfile } from "../office/types";
import type { RemoteFloorView } from "../office/world/RemoteFloor";
import { tauriApi } from "../ipc/tauriApi";

interface RemoteFloorsState {
  office: RemoteOffice;
  /** 보고 있는 층의 기기 id(null = 이 머신). */
  floor: string | null;
  /** 대화 보기를 연 원격 캐릭터(`기기id:캐릭터id`). */
  chat: string | null;
  /** 다른 층 캐릭터 hover(창 좌표). */
  hover: { id: string; x: number; y: number } | null;
  /** 원격 캐릭터 그림(커스텀 시트·미니미)을 받을 때마다 오른다 — 씬이 그 층을 다시 맞추게(`remoteImages.ts`). */
  imagesRev: number;
}

const EMPTY: RemoteOffice = { machines: [] };

export const useRemoteFloors = create<RemoteFloorsState>(() => ({ office: EMPTY, floor: null, chat: null, hover: null, imagesRev: 0 }));

export function setRemoteOffice(office: RemoteOffice): void {
  useRemoteFloors.setState((s) => {
    // 해제된 머신의 층을 보고 있었으면 이 머신으로 돌아온다.
    const floor = s.floor && office.machines.some((m) => m.deviceId === s.floor) ? s.floor : null;
    const chat = s.chat && findRemote(office, s.chat) ? s.chat : null;
    return { office, floor, chat, hover: floor === s.floor ? s.hover : null };
  });
}

export function selectFloor(floor: string | null): void {
  useRemoteFloors.setState({ floor, hover: null });
}

export function openRemoteChat(id: string | null): void {
  useRemoteFloors.setState({ chat: id, hover: null });
}

export function setRemoteHover(id: string | null, x: number, y: number): void {
  useRemoteFloors.setState({ hover: id ? { id, x, y } : null });
}

/** 원격 캐릭터와 그 머신. */
export function findRemote(office: RemoteOffice, id: string): { machine: RemoteMachine; character: RemoteCharacter } | null {
  for (const machine of office.machines) {
    const character = machine.characters.find((c) => c.id === id);
    if (character) return { machine, character };
  }
  return null;
}

/** 원격 캐릭터 → 오피스 씬 프로필. 커스텀 시트·미니미·초상 판을 실어, 그림이 오면(`remoteImages.ts`가 원격 id로
 *  덮어쓰기·미리보기를 채운다) 이 머신 캐릭터와 같은 길로 그려진다. 그림이 아직 없으면 절차 생성 모습. */
export function remoteProfile(c: RemoteCharacter): AgentProfile {
  const a = c.appearance ?? {};
  return {
    id: c.id,
    name: c.name,
    role: c.role,
    seed: a.seed || c.id,
    ...(a.archetype ? { archetype: a.archetype } : {}),
    ...(a.colors ? { colors: a.colors } : {}),
    ...(a.spriteUpdatedAt != null ? { spriteUpdatedAt: a.spriteUpdatedAt } : {}),
    ...(a.minimiUpdatedAt != null ? { minimiUpdatedAt: a.minimiUpdatedAt } : {}),
    ...(c.portrait != null ? { portraitUpdatedAt: c.portrait } : {}),
  };
}

/** 그 층을 씬에 넘길 모양. 이 머신 층이거나 그 머신이 없으면 null. 퇴근한 캐릭터는 세우지 않는다. */
export function remoteFloorView(office: RemoteOffice, floor: string | null): RemoteFloorView | null {
  if (!floor) return null;
  const machine = office.machines.find((m) => m.deviceId === floor);
  if (!machine) return null;
  return {
    key: machine.deviceId,
    agents: machine.characters
      .filter((c) => !c.clockedOut)
      .map((c) => ({
        profile: remoteProfile(c),
        active: c.sessionState === "starting" || c.sessionState === "running",
        pending: c.needsHuman,
        online: machine.online,
      })),
  };
}

/** 층 단추에 보일 요약: 일하는 수·답을 기다리는 수. */
export function machineSummary(machine: RemoteMachine): { working: number; waiting: number } {
  let working = 0;
  let waiting = 0;
  for (const c of machine.characters) {
    if (c.needsHuman) waiting += 1;
    else if (c.sessionState === "running" || c.sessionState === "starting") working += 1;
  }
  return { working, waiting };
}

export function installRemoteFloors(): () => void {
  let disposed = false;
  let off: (() => void) | undefined;
  void listen<RemoteOffice>(Events.linkRemote, (event) => setRemoteOffice(event.payload)).then((unlisten) => {
    if (disposed) unlisten();
    else off = unlisten;
  });
  void tauriApi.linkRemote().then((office) => {
    if (!disposed) setRemoteOffice(office);
  }).catch(() => {});
  return () => {
    disposed = true;
    off?.();
  };
}
