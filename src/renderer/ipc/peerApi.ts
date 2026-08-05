// src/renderer/ipc/peerApi.ts
//
// 피어 세션 공유(#7k, docs/peer-session-share-design.md)의 렌더러 어댑터.
// `tauriApi`(동결된 AgentOfficeApi)를 건드리지 않고 별도 표면으로 둔다 —
// 원격 세션의 출력/입력은 기존 `subscribe_output`/`write_input`이 백엔드에서
// `peer:` 접두사로 라우팅하므로, 여기 있는 것은 **관리 표면뿐**이다.

import { invoke } from "@tauri-apps/api/core";
import { Commands } from "@shared/ipc";

export type PeerPermission = "readOnly" | "input";
export type PeerConnState = "connecting" | "connected" | "disconnected";

/** 원격 캐릭터 메타(뷰어 쪽 캐시 — 호스트가 소유권을 갖는다). */
export interface PeerAgentInfo {
  /** `peer:<peerId>:<agentId>` — 렌더러 전역에서 이 값이 agentId다. */
  agentId: string;
  /** 호스트 쪽 원래 agentId. */
  localAgentId: string;
  peerId: string;
  name: string;
  role?: string | null;
  seed?: string;
  cwd?: string | null;
  state?: string | null;
  sessionId?: string | null;
  cols?: number;
  rows?: number;
}

export interface PeerViewerStatus {
  peerId: string;
  label: string;
  address: string;
  state: PeerConnState;
  permission: PeerPermission;
  error?: string;
  agents: PeerAgentInfo[];
}

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
  /** "web"이면 브라우저에서 온 요청(웹 호스팅 #7m). 기본 "peer". */
  clientKind?: "peer" | "web";
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
  sharedAgents: string[];
  peers: PeerSummary[];
  pending: PendingPairing[];
}

export interface PeerHostRecord {
  peerId: string;
  label: string;
  address: string;
  token: string;
  permission: PeerPermission;
  autoConnect: boolean;
}

export interface PairStartOutcome {
  pairingId: string;
  hostName: string;
  expiresIn: number;
}

export const peerApi = {
  // ── 호스트 역할 ──────────────────────────────────────────────────
  hostStatus(): Promise<PeerHostStatus> {
    return invoke(Commands.peerHostStatus);
  },
  setShared(agentId: string, shared: boolean): Promise<void> {
    return invoke(Commands.peerSetShared, { agentId, shared });
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

  // ── 뷰어 역할 ────────────────────────────────────────────────────
  viewerStatus(): Promise<PeerViewerStatus[]> {
    return invoke(Commands.peerViewerStatus);
  },
  /** 1단계: 호스트 화면에 6자리 코드와 승인 다이얼로그를 띄운다. */
  pairStart(address: string): Promise<PairStartOutcome> {
    return invoke(Commands.peerPairStart, { address });
  },
  /**
   * 2단계: 코드를 제시한다. 호스트가 아직 승인 버튼을 안 눌렀으면 `false`라
   * 호출자는 잠시 후 다시 부르면 된다(폴링).
   */
  pairFinish(address: string, pairingId: string, code: string): Promise<boolean> {
    return invoke(Commands.peerPairFinish, { address, pairingId, code });
  },
  hosts(): Promise<PeerHostRecord[]> {
    return invoke(Commands.peerHosts);
  },
  connect(peerId: string): Promise<void> {
    return invoke(Commands.peerConnect, { peerId });
  },
  disconnect(peerId: string): Promise<void> {
    return invoke(Commands.peerDisconnect, { peerId });
  },
  forgetHost(peerId: string): Promise<void> {
    return invoke(Commands.peerForgetHost, { peerId });
  },
};

/** `peer:<peerId>:<agentId>` 인가 — 렌더러 전역의 원격 판별. */
export function isRemoteAgentId(agentId: string): boolean {
  if (!agentId.startsWith("peer:")) return false;
  const rest = agentId.slice("peer:".length);
  const idx = rest.indexOf(":");
  return idx > 0 && idx < rest.length - 1;
}

/** 원격 키에서 peerId만 뽑는다(모르면 undefined). */
export function peerIdOf(agentId: string): string | undefined {
  if (!isRemoteAgentId(agentId)) return undefined;
  const rest = agentId.slice("peer:".length);
  return rest.slice(0, rest.indexOf(":"));
}
