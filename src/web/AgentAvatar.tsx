// src/web/AgentAvatar.tsx
//
// 아바타 이미지 한 칸. 해석은 `avatar.ts`(비동기·캐시)가 하고 여기서는
// 자리를 먼저 잡아 둔다 — 이미지가 나중에 채워져도 옆 텍스트가 밀리지 않게
// 빈 상자를 먼저 그린다.

import { useEffect, useState } from "react";

import { cachedAvatar, resolveAvatar } from "./avatar";
import type { RemoteAgent } from "./protocol";
import type { WebRemoteSocket } from "./ws";

interface Props {
  socket: WebRemoteSocket;
  agent: RemoteAgent;
  /** 표시 크기(px). */
  size?: number;
}

export function AgentAvatar({ socket, agent, size = 24 }: Props) {
  // 캐시가 있으면 첫 렌더부터 그린다(목록 ↔ 채팅 왕복에 깜빡이지 않는다).
  const [src, setSrc] = useState<string | null>(() => cachedAvatar(agent));

  useEffect(() => {
    let alive = true;
    void resolveAvatar(socket, agent).then((url) => {
      if (alive) setSrc(url);
    });
    return () => {
      alive = false;
    };
    // 아바타를 결정하는 것은 이 넷뿐이다(`avatar.ts`의 캐시 키와 같은 조합).
    // `agent` 객체 통째로 걸면 `agents` 프레임이 올 때마다 재해석이 돈다.
  }, [socket, agent.agentId, agent.seed, agent.archetype, agent.portraitUpdatedAt]);

  return (
    <span className="avatar" style={{ width: size, height: size }}>
      {src && <img src={src} alt="" width={size} height={size} />}
    </span>
  );
}
