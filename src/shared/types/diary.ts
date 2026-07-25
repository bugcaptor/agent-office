// src/shared/types/diary.ts
//
// Domain slice: character diary (#56/#60) and its work-log source data.
// See src/shared/types.ts for the frozen-contract overview.

import type { SessionId } from './common';

/**
 * 캐릭터 일기 한 편(#56). 성격 프롬프트 문체로 쓴 작업 로그 겸 일기.
 * per-agent append-only 로그(`diaries/<agentId>.jsonl`)의 한 줄. Rust `DiaryEntry` 미러.
 * agentId는 파일명이 담으므로 레코드엔 없다.
 */
export interface DiaryEntry {
  /** 작성 시각(백엔드 epoch ms). */
  at: number;
  /** 이 일기가 다룬 세션의 sessionId(재시작 경계 추적용). */
  sessionId: SessionId;
  /** 일기 본문(LLM 생성, 성격 문체 반영). */
  body: string;
}

/** 일기 내보내기(#65) JSON 파일 판별자. characterBundle과 같은 관례로 고정 문자열 —
 *  diaries/*.jsonl(줄 단위 원본)과 헷갈리지 않게 한다. */
export const DIARY_BUNDLE_KIND = "agent-office.diary" as const;
/** 현재 일기 번들 스키마 버전. */
export const DIARY_BUNDLE_SCHEMA_VERSION = 1 as const;

/**
 * 한 캐릭터의 일기 전체를 담는 자기완결형 내보내기 번들(#65). 현재는 내보내기
 * 전용(가져오기는 범위 밖)이라 파서는 없다 — 나중에 가져오기를 붙일 때
 * `parseCharacterBundle`과 같은 자리에서 검증한다.
 */
export interface DiaryBundle {
  kind: typeof DIARY_BUNDLE_KIND;
  schemaVersion: typeof DIARY_BUNDLE_SCHEMA_VERSION;
  /** 내보낸 시점의 캐릭터 이름(표시용 스냅샷). */
  agentName: string;
  /** 내보낸 시각(epoch ms). */
  exportedAt: number;
  /** 일기 전체(작성순, 오래된 → 최신). 원본 레코드 그대로. */
  entries: DiaryEntry[];
}

/** 작업 로그 한 항목의 종류(캐릭터 일기 원천). */
export type WorkLogKind = "prompt" | "tool" | "narration";

/**
 * 캐릭터 일기(#60)의 원천 데이터 한 조각. 렌더러 작업 로그 버퍼(`workLog.ts`)가
 * 세션 활동에서 캡처하며, 일기화 전까지 디스크에 스냅샷 보존된다
 * (`worklogs/<agentId>.json`). Rust `WorkLogItem` 미러(백엔드는 `kind`를 불투명
 * String으로 통과).
 */
export interface WorkLogItem {
  /** 캡처 시각(epoch ms). */
  at: number;
  /** 이 항목이 속한 세션(재시작 경계 추적용). */
  sessionId: SessionId;
  kind: WorkLogKind;
  /** 항목 본문(프롬프트 원문·도구 요약·내레이션 꼬리). */
  text: string;
  /** prompt 항목일 때, 그 시점 LLM 목표(goal). 일기 서사에 방향을 준다. */
  goal?: string;
}
