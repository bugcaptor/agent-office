// src/renderer/talk/TalkWidget.tsx
//
// 동료 대화(docs/agent-talk-design.md §7) 하단바 표시 + 킬스위치.
// `talkEnabled`가 켜져 있을 때만 보이고(꺼져 있으면 null 렌더 — 폴링도 안 한다),
// 열린 대화 수를 "대화 N"으로 보여준다. 클릭하면 공유 ContextMenu로 두 항목:
// "대화 로그 보기"(talk-log 모달)와 "대화 전체 중지"(danger — talkEnabled를
// false로 저장해 백엔드 라우트까지 통째로 막는다, 설계 §5 킬스위치).
//
// 갱신은 UsageWidget과 같은 단순 폴링이다 — talkStatus는 인메모리 스냅샷이라
// 저비용이고, 백엔드 타이머/추가 이벤트 구독 없이 8초 간격이면 충분하다.
import { useEffect, useState } from "react";
import { useAppStore } from "../store/appStore";
import { tauriApi } from "../ipc/tauriApi";
import { ContextMenu } from "../ui/ContextMenu";
import "./talk.css";

/** 폴링 주기(ms). 설계 §7 "5~10초" 범위 안. */
const POLL_INTERVAL_MS = 8_000;

/** 아직 안 끝난 대화 수 — `ended`가 붙은 대화는 이미 종료된 것이라 뺀다. */
export function openConversationCount(status: {
  conversations: readonly { ended?: string }[];
}): number {
  return status.conversations.filter((c) => !c.ended).length;
}

export function TalkWidget() {
  const enabled = useAppStore((s) => s.appSettings.talkEnabled);
  const openModal = useAppStore((s) => s.openModal);
  const updateAppSettings = useAppStore((s) => s.updateAppSettings);
  const [open, setOpen] = useState(0);
  const [queued, setQueued] = useState(0);
  const [menu, setMenu] = useState<{ x: number; y: number } | null>(null);

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

  if (!enabled) return null;

  return (
    <>
      <button
        type="button"
        className="pixel-btn talk-widget"
        aria-haspopup="menu"
        aria-label="동료 대화"
        title={
          queued > 0
            ? `열린 대화 ${open}건 · 전달 대기 ${queued}건 (상대가 한가해지면 전달)`
            : `열린 대화 ${open}건`
        }
        onClick={(e) => {
          const rect = e.currentTarget.getBoundingClientRect();
          setMenu({ x: rect.left, y: rect.top });
        }}
      >
        💬 대화 {open}
        {queued > 0 && <span className="talk-widget-queued">+{queued}</span>}
      </button>
      {menu && (
        <ContextMenu
          x={menu.x}
          y={menu.y}
          onClose={() => setMenu(null)}
          items={[
            {
              icon: "🗒",
              label: "대화 로그 보기",
              onSelect: () => openModal({ kind: "talk-log" }),
            },
            { separator: true },
            {
              icon: "⛔",
              label: "대화 전체 중지",
              danger: true,
              onSelect: () => updateAppSettings({ talkEnabled: false }),
            },
          ]}
        />
      )}
    </>
  );
}
