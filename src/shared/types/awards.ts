// src/shared/types/awards.ts
//
// Domain slice: 이 달의 우수사원(Employee of the Month) 시상 기록.
// See src/shared/types.ts for the frozen-contract overview.
//
// 설계: docs/employee-of-the-month-design.md
//
// 저장은 `<app_data>/awards/awards.json` 단일 문서(temp+rename 원자적 쓰기)이고,
// 수상자 초상은 확정 시점 스냅샷으로 `<app_data>/awards/portraits/<YYYY-MM>.png`에
// 복사된다. 캐릭터가 삭제되거나 이름·초상이 바뀌어도 과거 시상 화면이 온전해야
// 하므로 레코드는 표시에 필요한 값을 전부 자기 안에 스냅샷한다.

/** 현재 시상 파일 스키마 버전. 미래 값이면 로드를 거부하고 파일을 보존한다. */
export const AWARDS_SCHEMA_VERSION = 1 as const;

/**
 * 현재 선정 규칙 버전. 규칙이 바뀌어도 과거 레코드를 그 시절 규칙으로 해석할 수
 * 있도록 레코드마다 박아 둔다. v1 = workedMs → turns → activeDays → agentId
 * 사전식 비교, 최소 활동 임계(3일 / 30분), 봇 제외.
 */
export const AWARD_RULES_VERSION = 1 as const;

/** 시상 파일 전체. Rust `AwardsFile` 미러. */
export interface AwardsFile {
  version: typeof AWARDS_SCHEMA_VERSION;
  /** month 오름차순. */
  awards: AwardRecord[];
}

/** 한 달치 시상 레코드. `month`가 파일 내 유일 키다. Rust `AwardRecord` 미러. */
export interface AwardRecord {
  /** 로컬 기준 "YYYY-MM". 초상 스냅샷 파일명에도 쓰이므로 백엔드가 형식을 검증한다. */
  month: string;
  /** 확정 시각(epoch ms). */
  decidedAt: number;
  /** 이 레코드를 만든 선정 규칙 버전. */
  rulesVersion: number;
  /** 수상자. 자격 후보가 없던 달은 null(그래도 레코드는 남겨 재계산을 막는다). */
  winner: AwardWinner | null;
  /** 확정 시점 상위 5인 스냅샷(수상자 포함, 순위순). */
  leaderboard: AwardStanding[];
  /** 수상 소감. 재생성하면 append하고 이전 소감은 보존한다. 마지막 원소가 대표 소감. */
  speeches: AwardSpeech[];
}

/** 수상자 스냅샷. Rust `AwardWinner` 미러. */
export interface AwardWinner {
  agentId: string;
  /** 확정 시점 이름(개명·삭제에 영향받지 않게 스냅샷). */
  name: string;
  /** 확정 시점 역할. */
  role: string;
  /** 확정 시점 아키타입(있으면). */
  archetype?: string;
  /** `awards/portraits/<month>.png` 스냅샷이 존재하는지. */
  hasPortrait: boolean;
  stats: AwardStats;
}

/** 수상 근거가 된 그 달 통계 스냅샷. Rust `AwardStats` 미러. */
export interface AwardStats {
  workedMs: number;
  turns: number;
  toolEvents: number;
  activeDays: number;
  tokensIn: number;
  tokensOut: number;
  costUsd: number;
}

/** 순위표 한 행 스냅샷. Rust `AwardStanding` 미러. */
export interface AwardStanding {
  agentId: string;
  /** 확정 시점 이름. */
  name: string;
  workedMs: number;
  turns: number;
  activeDays: number;
}

/** 생성된 수상 소감 한 편. Rust `AwardSpeech` 미러. */
export interface AwardSpeech {
  /** 생성 시각(epoch ms). */
  at: number;
  /** 생성에 쓴 요약 provider(claude/codex/gemini/openrouter/...). */
  provider: string;
  /** 소감 본문(1인칭, 캐릭터 말투). */
  text: string;
}
