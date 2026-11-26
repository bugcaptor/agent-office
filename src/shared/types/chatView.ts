// src/shared/types/chatView.ts
//
// 터미널 탭 "대화 보기"의 계약. Rust `webdash::log::{LogPage, LogItem, ImageMeta}`와
// `observer::permission::PendingAsk` 미러(웹 상태판 `/api/log`·`/api/state`의 `asks`와 같은 모양).

export type ChatItemRole = "user" | "assistant";
export type ChatItemKind = "text" | "tool_use" | "tool_result" | "image";

/** 로그 항목에 붙는 이미지 참조. 바이트는 `chatViewImage`로 따로 받는다. */
export interface ChatImageMeta {
  id: string;
  mediaType: string;
  bytes: number;
  /** 그림이 온 파일 경로나 URL(전사가 알려 줄 때만). 그림 아래에 늘 보인다. */
  source?: string;
}

export interface ChatLogItem {
  seq: number;
  role: ChatItemRole;
  kind: ChatItemKind;
  text: string;
  toolName?: string;
  isError: boolean;
  sidechain: boolean;
  image?: ChatImageMeta;
  files?: { name: string }[];
}

export type ChatDownload =
  | { kind: "image"; imageId: string }
  | { kind: "file"; epoch: number; seq: number; index: number };

export interface ChatLogPage {
  epoch: number;
  /** 버퍼의 마지막 seq. 다음 요청의 `after`. */
  last: number;
  /** 전사 파일에 붙어 있는가(false면 Claude·Codex 세션이 아니다). */
  attached: boolean;
  items: ChatLogItem[];
}

/** 답을 기다리는 권한 요청·질문(Claude `PermissionRequest` 훅). */
export interface PendingAsk {
  id: string;
  toolName: string;
  toolInput: Record<string, unknown>;
  /** "다시 묻지 않기"(Claude가 제안한 권한 규칙)를 고를 수 있는가. */
  canAlways: boolean;
  sidechain: boolean;
  /** 받은 시각(유닉스 ms). */
  at: number;
}

export type AskAction = "allow" | "always" | "deny" | "dismiss";

/** 보내기 결과: 바로 넣었거나(sent) 작업 중이라 대기열에 쌓았다(queued). */
export type ChatSendOutcome = "sent" | "queued";

/** 턴이 끝나면 넣으려고 쌓아 둔 메시지(Rust `chat_queue::QueuedMessage`). */
export interface QueuedMessage {
  id: string;
  text: string;
  at: number;
}

export type TurnPhase = "working" | "needs-human" | "idle" | "ended";

/** 지금 상태와 대기열(Rust `chat_queue::LiveStatus`). 웹 상태판 `/api/state`도 같은 재료를 싣는다. */
export interface ChatLiveStatus {
  running: boolean;
  phase?: TurnPhase;
  /** 이 상태가 시작된 시각(작업 중이면 턴 시작, 유닉스 ms). */
  since?: number;
  /** 마지막 PTY 출력 시각. */
  outputAt?: number;
  /** 서버 시각. */
  now: number;
  /** `steer`: 작업 중이지만 지금 보내면 쌓지 않고 바로 넣는다(Claude, 기다리는 권한 요청·질문 없음). */
  queue: { items: QueuedMessage[]; error?: string; steer?: boolean };
}
