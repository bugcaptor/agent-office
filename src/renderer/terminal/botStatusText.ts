// src/renderer/terminal/botStatusText.ts
//
// 봇 런타임 상태(BotAgentStatus) → 사람이 읽는 문구(이슈 #57 후속 — 상태 가시화).
// 탭 배지 툴팁과 터미널 오버레이 배너가 같은 규칙을 공유한다.
import type { BotAgentStatus } from "@shared/types";

export interface BotStatusText {
  /** 아이콘(로봇/경고). */
  icon: string;
  /** 한 줄 제목(예: "이슈 #12 처리 중"). */
  title: string;
  /** 보조 설명(예: "/nova 명령 대기 중"). 없을 수 있다. */
  detail?: string;
}

/** phase·issue·slug·error로 배너/툴팁 문구를 만든다. */
export function botStatusText(st: BotAgentStatus): BotStatusText {
  if (st.phase === "error" || st.error) {
    return { icon: "⚠️", title: "봇 오류", detail: st.error ?? "알 수 없는 오류" };
  }
  const slug = st.slug ? `/${st.slug}` : "슬래시";
  switch (st.phase) {
    case "starting":
      return { icon: "🤖", title: "봇 시작 중…", detail: "저장소·계정 확인 중" };
    case "working":
      return {
        icon: "🤖",
        title: st.issue ? `이슈 #${st.issue} 처리 중` : "작업 처리 중",
        detail: "에이전트가 작업하는 동안 터미널이 잠깁니다",
      };
    case "watching":
    default:
      return {
        icon: "🤖",
        title: "이슈 감시 중",
        detail: `${slug} 명령을 기다리는 중`,
      };
  }
}

/**
 * 다음 폴링까지 남은 초. lastPollAtMs가 없으면(첫 폴링 전) undefined.
 * 0 이하이면 확인이 임박/진행 중(0 반환).
 */
export function nextPollSeconds(st: BotAgentStatus, nowMs: number): number | undefined {
  if (st.lastPollAtMs == null || !st.pollIntervalSec) return undefined;
  const nextAt = st.lastPollAtMs + st.pollIntervalSec * 1000;
  return Math.max(0, Math.round((nextAt - nowMs) / 1000));
}
