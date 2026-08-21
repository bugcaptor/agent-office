// src/renderer/ipc/peerApi.ts
//
// 웹 원격(docs/web-remote-design.md)의 렌더러 어댑터. `tauriApi`(동결된
// AgentOfficeApi)를 건드리지 않고 별도 표면으로 둔다 — 앱은 호스트 역할만
// 하므로 여기 있는 것은 **관리 표면뿐**이다(페어링 승인·클라이언트 관리·
// 화면 스냅샷 응답).

import { invoke } from "@tauri-apps/api/core";
import { Commands } from "@shared/ipc";

export type PeerPermission = "readOnly" | "input";

export interface PeerSummary {
  peerId: string;
  name: string;
  permission: PeerPermission;
  createdAt: number;
}

export interface PendingPairing {
  pairingId: string;
  code: string;
  viewerName: string;
  /** 코드 만료까지 남은 시간(ms). 승인 다이얼로그가 스스로 지우는 근거. */
  expiresInMs?: number;
}

export interface PeerHostStatus {
  enabled: boolean;
  running: boolean;
  port?: number | null;
  hostName: string;
  addressHint?: string | null;
  bind: string;
  peers: PeerSummary[];
  pending: PendingPairing[];
}

export const peerApi = {
  hostStatus(): Promise<PeerHostStatus> {
    return invoke(Commands.peerHostStatus);
  },
  approvePairing(pairingId: string, permission: PeerPermission): Promise<boolean> {
    return invoke(Commands.peerPairApprove, { pairingId, permission });
  },
  rejectPairing(pairingId: string): Promise<boolean> {
    return invoke(Commands.peerPairReject, { pairingId });
  },
  revokePeer(peerId: string): Promise<void> {
    return invoke(Commands.peerRevoke, { peerId });
  },
  setPeerPermission(peerId: string, permission: PeerPermission): Promise<void> {
    return invoke(Commands.peerSetPermission, { peerId, permission });
  },
  /** 호스트 렌더러가 `peer-snapshot-request`에 답하는 자리. */
  submitSnapshot(requestId: string, snapshot: string): Promise<void> {
    return invoke(Commands.submitPeerSnapshot, { requestId, snapshot });
  },
};
