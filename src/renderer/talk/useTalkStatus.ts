// src/renderer/talk/useTalkStatus.ts
//
// 동료 대화(docs/agent-talk-design.md §7) 상태 폴링 훅. 원래
// `TalkWidget`(현재는 BottomBar의 "📊 기록" 버튼으로 흡수됨) 안에 있던
// useEffect 폴링을 그대로 옮긴 것이다 — enabled가 꺼져 있으면 폴링하지
// 않고 0을 반환한다(백엔드 라우트도 킬스위치로 막혀 있으니 조회할 이유가
// 없다).
//
// 갱신은 UsageWidget과 같은 단순 폴링이다 — talkStatus는 인메모리 스냅샷이라
// 저비용이고, 백엔드 타이머/추가 이벤트 구독 없이 8초 간격이면 충분하다.
import { useEffect, useState } from "react";
import { tauriApi } from "../ipc/tauriApi";

/** 폴링 주기(ms). 설계 §7 "5~10초" 범위 안. */
const POLL_INTERVAL_MS = 8_000;

/** 아직 안 끝난 대화 수 — `ended`가 붙은 대화는 이미 종료된 것이라 뺀다. */
export function openConversationCount(status: {
  conversations: readonly { ended?: string }[];
}): number {
  return status.conversations.filter((c) => !c.ended).length;
}

export interface TalkStatusSnapshot {
  /** 열린(끝나지 않은) 대화 수. */
  open: number;
  /** 아직 상대에게 닿지 않은 메시지 수(상대가 바쁘면 여기 쌓인다). */
  queued: number;
}

/**
 * `enabled`가 켜져 있는 동안 8초마다 `talkStatus`를 폴링해 열린 대화 수와
 * 전달 대기 수를 반환한다. 꺼지면 폴링을 멈추고 {open: 0, queued: 0}을 낸다.
 * 조회 실패는 콘솔 경고로만 남기고 이전 값을 유지한다(다음 폴링이 재시도).
 */
export function useTalkStatus(enabled: boolean): TalkStatusSnapshot {
  const [open, setOpen] = useState(0);
  const [queued, setQueued] = useState(0);

  useEffect(() => {
    if (!enabled) {
      setOpen(0);
      setQueued(0);
      return;
    }
    let cancelled = false;
    const poll = async () => {
      try {
        const status = await tauriApi.talkStatus();
        if (cancelled) return;
        setOpen(openConversationCount(status));
        setQueued(status.queued);
      } catch (err) {
        // 실패는 콘솔 경고로만 — 다음 폴링이 재시도한다(이전 값 유지).
        console.warn("talk: 상태 조회 실패", err);
      }
    };
    void poll();
    const id = window.setInterval(() => void poll(), POLL_INTERVAL_MS);
    return () => {
      cancelled = true;
      window.clearInterval(id);
    };
  }, [enabled]);

  return { open, queued };
}
