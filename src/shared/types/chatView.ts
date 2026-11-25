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
}

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
