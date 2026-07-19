// src/renderer/terminal/BotOverlay.tsx
//
// 봇 운전 중 오버레이(이슈 #57 후속). 봇 모드가 켜진 탭의 터미널 영역을 덮어
// (1) 사람의 실수 클릭·드래그·선택을 막고(pointer-events로 캡처), (2) 지금 봇이
// 무엇을 하는지 상단 배너로 보여준다. 키 입력은 이미 TerminalRegistry.writeInput
// 게이트가 막지만, 마우스 조작과 시각적 상태 표시는 이 오버레이의 몫이다.
//
// 스크림은 반투명이라 아래 에이전트 출력이 비쳐 보인다 — "뭐 하는 중인지" 배너와
// 실제 출력을 함께 볼 수 있다. 유일한 탈출구는 배너의 "봇 끄기" 버튼(확인 후
// stopBot). 오버레이는 terminal-mount 안에 있어 탭 스트립은 덮지 않는다 —
// 탭 우클릭 메뉴로도 끌 수 있다.
import { useEffect, useState } from "react";
import { useAppStore } from "../store/appStore";
import { botStatusText, nextPollSeconds } from "./botStatusText";

export function BotOverlay({ agentId }: { agentId: string }) {
  const status = useAppStore((s) => s.botMode[agentId]);
  const stopBot = useAppStore((s) => s.stopBot);
  // 카운트다운 갱신용 1초 틱. 봇이 켜져 있을 때만 돈다.
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    if (!status) return;
    const iv = window.setInterval(() => setNow(Date.now()), 1000);
    return () => window.clearInterval(iv);
  }, [status]);

  if (!status) return null;

  const { icon, title, detail } = botStatusText(status);
  const secs = status.phase === "error" ? undefined : nextPollSeconds(status, now);
  const countdown =
    secs == null ? undefined : secs <= 0 ? "확인 중…" : `다음 확인까지 ${secs}초`;

  const onStop = () => {
    if (
      window.confirm(
        "봇 모드를 끄고 이 탭을 직접 조작할까요? 진행 중인 봇 작업 흐름이 중단됩니다."
      )
    ) {
      void stopBot(agentId);
    }
  };

  return (
    // 전면 클릭 캡처: 아래로 클릭·mousedown이 새지 않게 막는다(키는 별도 게이트).
    <div
      className={
        status.phase === "error" ? "bot-overlay bot-overlay-error" : "bot-overlay"
      }
      role="status"
      aria-live="polite"
      onMouseDown={(e) => e.stopPropagation()}
      onClick={(e) => e.stopPropagation()}
    >
      <div className="bot-overlay-banner">
        <span className="bot-overlay-icon" aria-hidden="true">
          {icon}
        </span>
        <div className="bot-overlay-text">
          <div className="bot-overlay-title">{title}</div>
          {detail && <div className="bot-overlay-detail">{detail}</div>}
          {countdown && <div className="bot-overlay-countdown">{countdown}</div>}
        </div>
        <button type="button" className="pixel-btn bot-overlay-stop" onClick={onStop}>
          봇 끄기
        </button>
      </div>
    </div>
  );
}
