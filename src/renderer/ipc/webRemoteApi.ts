// src/renderer/ipc/webRemoteApi.ts
//
// 웹 원격(docs/web-remote-design.md)의 렌더러 어댑터. `tauriApi`(동결된
// AgentOfficeApi)를 건드리지 않고 별도 표면으로 둔다 — 앱은 호스트 역할만
// 하므로 여기 있는 것은 **관리 표면뿐**이다(페어링 승인·클라이언트 관리·
// 화면 스냅샷 응답).

import { invoke } from "@tauri-apps/api/core";
import { Commands } from "@shared/ipc";

export type ClientPermission = "readOnly" | "input";

export interface ClientSummary {
  clientId: string;
  name: string;
  permission: ClientPermission;
  createdAt: number;
}

export interface PendingPairing {
  pairingId: string;
  code: string;
  clientName: string;
  /** 코드 만료까지 남은 시간(ms). 승인 다이얼로그가 스스로 지우는 근거. */
  expiresInMs?: number;
}

export interface WebRemoteStatus {
  enabled: boolean;
  running: boolean;
  port?: number | null;
  hostName: string;
  addressHint?: string | null;
  bind: string;
  /** 로컬 인터페이스에서 tailscale 주소를 찾았는지. `bind === "tailnet"`인데
   * false면 리스너가 루프백에만 열려 있다. */
  tailnetFound: boolean;
  clients: ClientSummary[];
  pending: PendingPairing[];
}

export const webRemoteApi = {
  hostStatus(): Promise<WebRemoteStatus> {
    return invoke(Commands.webRemoteStatus);
  },
  approvePairing(pairingId: string, permission: ClientPermission): Promise<boolean> {
    return invoke(Commands.webRemotePairApprove, { pairingId, permission });
  },
  rejectPairing(pairingId: string): Promise<boolean> {
    return invoke(Commands.webRemotePairReject, { pairingId });
  },
  revokeClient(clientId: string): Promise<void> {
    return invoke(Commands.webRemoteRevoke, { clientId });
  },
  setClientPermission(clientId: string, permission: ClientPermission): Promise<void> {
    return invoke(Commands.webRemoteSetPermission, { clientId, permission });
  },
  /** 호스트 렌더러가 `web-remote-snapshot-request`에 답하는 자리. */
  submitSnapshot(requestId: string, snapshot: string): Promise<void> {
    return invoke(Commands.webRemoteSubmitSnapshot, { requestId, snapshot });
  },
};
