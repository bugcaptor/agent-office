// src/shared/types/memo.ts
//
// Domain slice: 에이전트별 포스트잇 메모(#79).
// See src/shared/types.ts for the frozen-contract overview.

/**
 * 포스트잇 메모의 장(sheet) 한 장. 디스크 원본은
 * `<app_data>/memos/<agentId>/<sheetId>.txt`(Obsidian식 frontmatter + plain
 * text)이고 이 타입은 그 파싱 결과다. Rust `MemoSheet` 미러. agentId는
 * 폴더명이 담으므로 레코드엔 없다.
 *
 * 시각이 epoch ms가 아니라 RFC3339 문자열(로컬 오프셋 포함)인 이유: 이 파일은
 * 사용자가 에디터로 직접 열어 읽는 메모라, 헤더도 사람이 읽을 수 있어야 한다.
 * 렌더러는 표시 외에 이 값으로 계산하지 않는다.
 */
export interface MemoSheet {
  /** 생성 시각 기반 식별자(사전순 = 시간순, 예: `20260730T123456`). */
  sheetId: string;
  /** 장이 만들어진 시각(RFC3339). */
  created: string;
  /** 마지막 본문 저장 시각(RFC3339). */
  updated: string;
  /** 넘겨진(아카이브된) 시각. **없으면 이 장이 현재 장**이다. */
  archived?: string;
  /** 본문(plain text — 마크다운으로 렌더하지 않는다). */
  content: string;
}

/** 아카이브 목록 한 항목 — 본문을 뺀 메타만. Rust `MemoSheetMeta` 미러.
 *  `archived`가 필수인 점이 `MemoSheet`와 다르다(아카이브된 장만 담기므로). */
export interface MemoSheetMeta {
  sheetId: string;
  created: string;
  updated: string;
  archived: string;
}
