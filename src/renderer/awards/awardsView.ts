// src/renderer/awards/awardsView.ts
//
// AwardsDialog가 쓰는 순수 표시 로직. 컴포넌트에서 분리해 두면 vitest로
// jsdom 없이 바로 검증할 수 있다(usageView.ts/talkLogView.ts와 같은 관례).
import type { AwardWinner } from "@shared/types";

/** 작업시간을 "43시간 0분" 식으로. AnalyticsDialog의 `formatDuration`(반올림 소수)과
 * 달리 시상 화면은 사람이 읽는 시:분 단위를 요구한다(설계 §6). */
export function formatWorkedHm(ms: number): string {
  const totalMinutes = Math.max(0, Math.round(ms / 60_000));
  const hours = Math.floor(totalMinutes / 60);
  const minutes = totalMinutes % 60;
  return `${hours}시간 ${minutes}분`;
}

/** 수상 소감 버튼 비활성 사유. 우선순위: 수상자 없음 > 프로필 삭제됨 > 요약기
 * OFF > 생성 중. 앞의 것이 뒤의 것을 가린다(둘 다 참이면 더 근본적인 사유를 보인다). */
export type SpeechButtonState =
  | { disabled: false }
  | { disabled: true; reason: string };

export interface SpeechButtonInput {
  winner: AwardWinner | null;
  /** 수상자 캐릭터 프로필이 지금도 존재하는지(삭제되지 않았는지). */
  profileExists: boolean;
  /** 설정 > 요약기 활성 여부. */
  summarizerEnabled: boolean;
  /** 이 달 소감 생성이 인플라이트인지. */
  generating: boolean;
}

export function speechButtonState(input: SpeechButtonInput): SpeechButtonState {
  if (input.winner === null) {
    return { disabled: true, reason: "이 달은 수상자가 없어 소감을 들을 수 없습니다." };
  }
  if (!input.profileExists) {
    return {
      disabled: true,
      reason: "수상자 캐릭터가 남아 있지 않아 소감을 들을 수 없습니다.",
    };
  }
  if (!input.summarizerEnabled) {
    return {
      disabled: true,
      reason: "설정에서 요약 기능을 켜면 소감을 들을 수 있습니다.",
    };
  }
  if (input.generating) {
    return { disabled: true, reason: "소감을 생성하는 중입니다." };
  }
  return { disabled: false };
}
