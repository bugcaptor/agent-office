// src/renderer/terminal/botStatusText.ts
//
// 봇 런타임 상태(BotAgentStatus) → 사람이 읽는 문구(이슈 #57 후속 — 상태 가시화).
// 탭 배지 툴팁과 터미널 오버레이 배너가 같은 규칙을 공유한다.
//
// React 밖에서도 불릴 수 있는 순수 모듈이라 훅이 아니라 모듈 `t`를 쓴다 —
// 다만 **호출 시점**에만 부른다(모듈 최상위에서 부르면 언어를 바꿔도
// 문구가 그대로 굳는다).
import { t } from "@renderer/i18n";
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
    return {
      icon: "⚠️",
      title: t("terminal:bot.errorTitle"),
      detail: st.error ?? t("terminal:bot.errorUnknown"),
    };
  }
  const slug = st.slug ? `/${st.slug}` : t("terminal:bot.slashFallback");
  switch (st.phase) {
    case "starting":
      return {
        icon: "🤖",
        title: t("terminal:bot.startingTitle"),
        detail: t("terminal:bot.startingDetail"),
      };
    case "working":
      return {
        icon: "🤖",
        title: st.issue
          ? t("terminal:bot.workingIssueTitle", { issue: st.issue })
          : t("terminal:bot.workingTitle"),
        detail: t("terminal:bot.workingDetail"),
      };
    case "watching":
    default:
      return {
        icon: "🤖",
        title: t("terminal:bot.watchingTitle"),
        detail: t("terminal:bot.watchingDetail", { slug }),
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
