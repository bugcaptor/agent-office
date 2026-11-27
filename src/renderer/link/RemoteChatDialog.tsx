// src/renderer/link/RemoteChatDialog.tsx
//
// 머신 연동 호스트: 다른 층 캐릭터를 누르면 뜨는 대화 보기. 터미널 탭의 대화 보기(`ChatView`)를 그대로 쓰되
// 원격 모드라 보기만 한다 — 전사·권한 요청·대기열·지금 상태를 그 머신에 물어 그린다(`chat_view_*`가
// `기기id:캐릭터id`를 알아보고 링크로 묻는다). 터미널 화면은 비추지 않는다.
import { useEffect } from "react";
import { ChatView } from "../chatView/ChatView";
import { findRemote, openRemoteChat, useRemoteFloors } from "./remoteFloors";
import "./remoteFloors.css";

export function RemoteChatDialog() {
  const chat = useRemoteFloors((s) => s.chat);
  const office = useRemoteFloors((s) => s.office);
  const found = chat ? findRemote(office, chat) : null;
  useEffect(() => {
    if (!chat) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") openRemoteChat(null);
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [chat]);
  if (!chat || !found) return null;
  const close = () => openRemoteChat(null);
  return (
    <div className="remote-chat-backdrop" onClick={close}>
      <div className="remote-chat pixel-panel" role="dialog" aria-label={found.character.name} onClick={(e) => e.stopPropagation()}>
        <div className="remote-chat-head">
          <span className="remote-chat-name">{found.character.name}</span>
          <span className="remote-chat-machine">{found.machine.name}</span>
        </div>
        <div className="remote-chat-body">
          <ChatView key={chat} agentId={chat} active remote={{ machine: found.machine.name, onClose: close }} />
        </div>
      </div>
    </div>
  );
}
