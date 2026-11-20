// src/shared/types/role.ts
//
// Domain slice: 토의 역할 목록(`roles.json`). Rust `DiscussionRole`/`RolesFile` 미러.
// 프로필의 자유 텍스트 `role`(직함)과는 별개다 — 토의에서 맡는 역할 카드다.

import type { WorkAgent } from './workPanels';

/** 이 빌드가 읽는 최대 스키마 버전. Rust `ROLES_SCHEMA_VERSION` 미러. */
export const ROLES_SCHEMA_VERSION = 1;

/** 토의 태도. `moderator`는 의견 없이 지목·수렴 판단·메모 작성만 한다. */
export type DiscussionStance =
  | "moderator"
  | "free"
  | "supportive"
  | "critical"
  | "devils_advocate"
  | "divergent"
  | "synthesizer";

export const DISCUSSION_STANCES: readonly DiscussionStance[] = [
  "moderator", "free", "supportive", "critical", "devils_advocate", "divergent", "synthesizer",
];

export interface DiscussionRole {
  id: string;
  name: string;
  /** 관점·책임. */
  perspective: string;
  stance: DiscussionStance;
  /** 판단 기준·금지사항. */
  criteria: string;
  /** 토의 참가 시 미리 채울 CLI·모델. 없으면 참가자 프로필 기본값. */
  agent?: WorkAgent;
}

export interface RolesFile {
  version: number;
  roles: DiscussionRole[];
}
