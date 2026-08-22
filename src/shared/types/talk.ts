// src/shared/types/talk.ts
//
// 동료 대화(docs/agent-talk-design.md)의 렌더러 쪽 타입. Rust `talk::*` /
// `ipc::commands::talk::TalkStatus` / `types::TalkEvent`의 거울이다.

/** 살아 있는(또는 끝난) 대화 한 건. */
export interface TalkConversation {
  id: string;
  /** 말을 건 쪽. */
  a: string;
  /** 상대. */
  b: string;
  /** 지금까지 오간 메시지 수(왕복 상한의 기준). */
  turns: number;
  startedAt: number;
  /** 끝났으면 사유("manual" | "max-turns" | "expired" | "disabled"). */
  ended?: string;
}

/** `talk_status` 응답 — 하단바 표시용 스냅샷. */
export interface TalkStatus {
  enabled: boolean;
  /** 아직 상대에게 닿지 않은 메시지 수(상대가 바쁘면 여기 쌓인다). */
  queued: number;
  conversations: TalkConversation[];
}

/** "talk-message" 이벤트 — 누가 누구에게 뭐라고 말했는지. 배달(주입)은 상대가
 * 한가해질 때까지 늦춰질 수 있으므로 이건 "말했다"이지 "전달됐다"가 아니다. */
export interface TalkEvent {
  convId: string;
  from: string;
  fromName: string;
  to: string;
  toName: string;
  text: string;
  at: number;
}

/** 감사 로그 한 줄(`<app_data>/talks/YYYY-MM-DD.jsonl`). */
export interface TalkLogEntry {
  /** "send" | "deliver" | "expire". */
  kind: string;
  id: string;
  convId: string;
  from: string;
  fromName: string;
  to: string;
  text: string;
  at: number;
  note?: string | null;
}
