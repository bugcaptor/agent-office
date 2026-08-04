// src/web/protocol.ts
//
// 웹 호스팅(kbm #7m) 와이어 타입 — Rust `src-tauri/src/peer/protocol.rs`의 미러.
// `src/shared`로 올리지 않는 이유: 지금 소비자가 웹 클라이언트 하나뿐이고,
// 데스크톱 렌더러는 이 프로토콜을 모른다(Tauri 커맨드로 직접 간다).

export type PeerPermission = "readOnly" | "input";

export interface PeerAgent {
  agentId: string;
  name: string;
  role?: string | null;
  seed?: string;
  cwd?: string | null;
  /** "running" | "starting" | "exited" | "disposed" | null(세션 없음) */
  state?: string | null;
  sessionId?: string | null;
  cols?: number;
  rows?: number;
}

export interface PeerOutput {
  agentId: string;
  sessionId: string;
  seq: number;
  /** 이 청크가 시작하는 절대 스트림 오프셋. */
  offset: number;
  data: string;
  bytes: number;
}

export interface RpcErrorPayload {
  /** unknownCmd | forbidden | badArgs | notFound | internal */
  code: string;
  message: string;
}

export type HostMsg =
  | {
      type: "hello";
      hostName: string;
      appVersion: string;
      protoVersion: number;
      permission: PeerPermission;
      peerId: string;
    }
  | { type: "agents"; agents: PeerAgent[] }
  | {
      type: "restore";
      agentId: string;
      snapshot?: string | null;
      baseOffset: number;
      cols?: number;
      rows?: number;
      sessionId?: string | null;
    }
  | ({ type: "output" } & PeerOutput)
  | { type: "activity"; agentId: string; payload: Record<string, unknown> }
  | { type: "sessionState"; agentId: string; payload: Record<string, unknown> }
  | { type: "notification"; agentId: string; payload: Record<string, unknown> }
  | { type: "notificationCleared"; agentId: string; ids: string[] }
  | { type: "resized"; agentId: string; cols: number; rows: number }
  | { type: "pong" }
  | {
      type: "rpcResult";
      id: number;
      ok: boolean;
      data?: unknown;
      error?: RpcErrorPayload;
    }
  | { type: "error"; message: string };

export type ViewerMsg =
  | { type: "attach"; agentId: string; lastOffset?: number | null }
  | { type: "detach"; agentId: string }
  | { type: "input"; agentId: string; data: string }
  | { type: "rpc"; id: number; cmd: string; args?: unknown }
  | { type: "ping" };

/** 서버 allowlist와 같은 이름들. 여기 없는 것은 서버에도 없다. */
export const RpcCmd = {
  agentsList: "agents.list",
  notificationsList: "notifications.list",
  usageSnapshot: "usage.snapshot",
  sessionStart: "session.start",
  sessionDispose: "session.dispose",
  notificationsClear: "notifications.clear",
} as const;

export interface NotificationItem {
  id: string;
  agentId: string;
  sessionId: string;
  message: string;
  at: number;
  source: string;
}
