// src/renderer/awards/awardsView.ts
//
// AwardsDialog가 쓰는 순수 표시 로직. 컴포넌트에서 분리해 두면 vitest로
// jsdom 없이 바로 검증할 수 있다(usageView.ts/talkLogView.ts와 같은 관례).
//
// i18n: 완성된 문구를 만들어 돌려주는 곳(`formatWorkedHm`)은 **호출 시점에**
// 번역하고, 상태를 서술하는 곳(`speechButtonState`)은 문구가 아니라 **키**를
// 담는다 — 후자는 값이 렌더 사이에 살아남을 수 있어, 언어를 바꿨을 때 이미
// 계산된 상태가 옛 언어로 남지 않게 하려는 것이다(workdir/status.ts와 같은 관례).
import { t } from "@renderer/i18n";
import type { AwardWinner } from "@shared/types";

/** 작업시간을 "43시간 0분" 식으로. AnalyticsDialog의 `formatDuration`(반올림 소수)과
 * 달리 시상 화면은 사람이 읽는 시:분 단위를 요구한다(설계 §6). */
export function formatWorkedHm(ms: number): string {
  const totalMinutes = Math.max(0, Math.round(ms / 60_000));
  const hours = Math.floor(totalMinutes / 60);
  const minutes = totalMinutes % 60;
  return t("journal:awards.workedHm", { hours, minutes });
}

/** 수상 소감 버튼 비활성 사유. 우선순위: 수상자 없음 > 프로필 삭제됨 > 요약기
 * OFF > 생성 중. 앞의 것이 뒤의 것을 가린다(둘 다 참이면 더 근본적인 사유를 보인다). */
export type SpeechButtonState =
  | { disabled: false }
  /** `reasonKey`는 `journal` 네임스페이스의 키다(표시는 호출자의 `t`가 한다). */
  | { disabled: true; reasonKey: string };

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
    return { disabled: true, reasonKey: "awards.speechReason.noWinner" };
  }
  if (!input.profileExists) {
    return { disabled: true, reasonKey: "awards.speechReason.profileMissing" };
  }
  if (!input.summarizerEnabled) {
    return { disabled: true, reasonKey: "awards.speechReason.disabled" };
  }
  if (input.generating) {
    return { disabled: true, reasonKey: "awards.speechReason.generating" };
  }
  return { disabled: false };
}
