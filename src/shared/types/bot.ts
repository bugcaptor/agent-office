// src/shared/types/bot.ts
//
// Domain slice: bot mode (이슈 #57) config and runtime status.
// See src/shared/types.ts for the frozen-contract overview.

/** 캐릭터 봇 모드 설정(이슈 #57, docs/bot-mode-design.md). Rust `BotConfig` 미러. */
export interface BotConfig {
  /** 슬래시 명령 slug 별칭(예: "Nova Kim" → "nova"). 비면 이름에서 자동 파생. */
  slug?: string;
  /** 명령을 발동할 수 있는 추가 Gitea 계정. tea 로그인 계정 본인은 항상 포함. */
  whitelist: string[];
  /** 이슈/댓글 폴링 주기(초). 부재 시 기본 60, 하한 30. */
  pollIntervalSec?: number;
  /** turn-taking 유휴 판정 임계(ms). 부재 시 기본 3000. */
  idleQuietMs?: number;
}

/** 봇 폴링 태스크의 현재 단계(이슈 #57 후속). Rust `BotPhase` 미러. */
export type BotPhase = "starting" | "watching" | "working" | "error";

/** 봇 모드가 켜진 캐릭터 한 명의 런타임 상태(이슈 #57). Rust `BotAgentStatus` 미러. */
export interface BotAgentStatus {
  /** 폴링 태스크가 살아 있는지. */
  running: boolean;
  /** 현재 단계 — GUI 배너 문구의 근거. */
  phase: BotPhase;
  /** 현재 이 탭에 바인딩된 이슈 번호(작업 중일 때). */
  issue?: number;
  /** 이 봇이 반응하는 슬래시 slug. */
  slug?: string;
  /** 폴링 주기(초). "다음 확인까지 N초" 카운트다운 계산에 쓴다. */
  pollIntervalSec: number;
  /** 마지막 폴링이 끝난 시각(epoch ms). 없으면 아직 첫 폴링 전. */
  lastPollAtMs?: number;
  /** 마지막 폴링/기동 오류(tea 미로그인 등). 없으면 정상. */
  error?: string;
}

/** `bot_status` 응답(이슈 #57) — 봇 모드가 켜진 탭들의 스냅샷. */
export interface BotStatus {
  /** agentId → 상태. 봇 모드가 켜진 탭만 포함한다. */
  agents: Record<string, BotAgentStatus>;
}
