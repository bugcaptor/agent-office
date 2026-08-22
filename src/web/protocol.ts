// src/web/protocol.ts
//
// 웹 원격 와이어 타입 — Rust `src-tauri/src/webremote/protocol.rs`의 미러.
// `src/shared`로 올리지 않는 이유: 지금 소비자가 웹 클라이언트 하나뿐이고,
// 데스크톱 렌더러는 이 프로토콜을 모른다(Tauri 커맨드로 직접 간다).

import type { ColorOverrides } from "@shared/types";

export type ClientPermission = "readOnly" | "input";

export interface RemoteAgent {
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
  /** 아키타입(종족) id. 절차 생성 아바타에 seed와 함께 쓴다. 없으면 "human". */
  archetype?: string | null;
  /** 커스텀 초상이 있으면 epoch ms(캐시 키). 없으면 절차 생성 아바타. */
  portraitUpdatedAt?: number | null;
  /** 사용자가 고른 팔레트 색 오버라이드. 절차 생성 아바타를 호스트와 같은 색으로 그린다. */
  colors?: ColorOverrides | null;
}

export interface RemoteOutput {
  agentId: string;
  sessionId: string;
  seq: number;
  /** 이 청크가 시작하는 절대 스트림 오프셋. */
  offset: number;
  data: string;
  bytes: number;
}

/** 전사 항목 — Rust `session_log::agent_transcript::TranscriptItem`의 미러. */
export type ItemRole = "user" | "assistant";
export type ItemKind = "text" | "tool_use" | "tool_result";

export interface TranscriptItem {
  role: ItemRole;
  kind: ItemKind;
  text: string;
  toolName?: string | null;
  isError?: boolean;
  sidechain?: boolean;
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
      permission: ClientPermission;
      clientId: string;
    }
  | { type: "agents"; agents: RemoteAgent[] }
  | {
      type: "restore";
      agentId: string;
      snapshot?: string | null;
      baseOffset: number;
      cols?: number;
      rows?: number;
      sessionId?: string | null;
    }
  | ({ type: "output" } & RemoteOutput)
  | {
      type: "chat";
      agentId: string;
      /** `backfill`이면 목록을 **교체**하고, 아니면 이어 붙인다. */
      items?: TranscriptItem[];
      backfill?: boolean;
      /** 전사가 없어 채팅화가 불가능한 세션(일반 셸·미지원 CLI). */
      unavailable?: boolean;
    }
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

export type ClientMsg =
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
  mediaPortrait: "media.portrait",
  sessionStart: "session.start",
  sessionDispose: "session.dispose",
  notificationsClear: "notifications.clear",
  chatFollow: "chat.follow",
  chatSend: "chat.send",
  chatKeys: "chat.keys",
} as const;

/** 서버 `webremote::chat::key_bytes`가 아는 이름들. 그 밖은 badArgs다. */
export type ChatKey =
  | "enter"
  | "esc"
  | "up"
  | "down"
  | "left"
  | "right"
  | "tab"
  | "backspace"
  | "space"
  | "ctrl-c"
  | "1"
  | "2"
  | "3"
  | "4"
  | "5"
  | "6"
  | "7"
  | "8"
  | "9"
  | "y"
  | "n";

export interface NotificationItem {
  id: string;
  agentId: string;
  sessionId: string;
  message: string;
  at: number;
  source: string;
}
