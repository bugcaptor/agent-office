// src/web/App.tsx
//
// 웹 클라이언트 루트. 상태는 **전부 서버 push에서 온다** — 로컬 영속이 없으므로
// 데스크톱 앱과 스토어가 두 벌이 되는 문제가 애초에 없다(#7m A 결정의 핵심).
//
// 알림·활동·세션 상태는 **attach 없이** 온다(M2에서 서버 필터를 터미널
// 프레임으로 좁혔다). 그래서 앱은 모든 캐릭터에 미리 붙지 않는다 — 출력 tap과
// 링버퍼는 터미널 화면을 실제로 연 캐릭터에만 생긴다.
//
// 캐릭터를 열면 기본은 **채팅 뷰**이고, 터미널 미러는 헤더 토글로 가는
// 폴백이다(docs/web-remote-design.md §2).

import { useEffect, useMemo, useRef, useState } from "react";
import { AgentList } from "./AgentList";
import { ChatScreen } from "./ChatScreen";
import { PairingScreen } from "./PairingScreen";
import { TerminalScreen } from "./TerminalScreen";
import type { HostMsg, NotificationItem, RemoteAgent, ClientPermission } from "./protocol";
import { RpcCmd } from "./protocol";
import { WebRemoteSocket, probeAuth, type ConnState } from "./ws";

type Phase = "checking" | "pairing" | "ready";
type View = "chat" | "terminal";

export function App() {
  const [phase, setPhase] = useState<Phase>("checking");
  const [agents, setAgents] = useState<RemoteAgent[]>([]);
  const [permission, setPermission] = useState<ClientPermission>("readOnly");
  const [hostName, setHostName] = useState("Agent Office");
  const [connState, setConnState] = useState<ConnState>("closed");
  const [openAgentId, setOpenAgentId] = useState<string | null>(null);
  const [view, setView] = useState<View>("chat");
  const [notifications, setNotifications] = useState<Record<string, NotificationItem[]>>({});
  const socketRef = useRef<WebRemoteSocket | null>(null);

  // 쿠키가 살아 있는지 먼저 본다(WS 401이면 페어링 화면).
  useEffect(() => {
    let alive = true;
    void probeAuth().then((ok) => {
      if (alive) setPhase(ok ? "ready" : "pairing");
    });
    return () => {
      alive = false;
    };
  }, []);

  useEffect(() => {
    if (phase !== "ready") return;
    const socket = new WebRemoteSocket();
    socketRef.current = socket;

    const offState = socket.onState(setConnState);
    const off = socket.onMessage((msg: HostMsg) => {
      switch (msg.type) {
        case "hello":
          setPermission(msg.permission);
          setHostName(msg.hostName);
          break;
        case "agents":
          setAgents(msg.agents);
          break;
        case "notification": {
          const item = msg.payload as unknown as NotificationItem;
          setNotifications((prev) => {
            const list = prev[msg.agentId] ?? [];
            if (list.some((n) => n.id === item.id)) return prev;
            return { ...prev, [msg.agentId]: [...list, item] };
          });
          break;
        }
        case "notificationCleared":
          setNotifications((prev) => ({ ...prev, [msg.agentId]: [] }));
          break;
        case "sessionState":
          // 목록 메타는 서버가 곧바로 `agents`로 다시 밀어 준다.
          break;
        case "error":
          console.warn("host error:", msg.message);
          break;
        default:
          break;
      }
    });

    socket.connect();

    return () => {
      off();
      offState();
      socket.dispose();
      socketRef.current = null;
    };
  }, [phase]);

  const openAgent = useMemo(
    () => agents.find((a) => a.agentId === openAgentId) ?? null,
    [agents, openAgentId]
  );

  if (phase === "checking") {
    return <div className="pair muted">확인 중…</div>;
  }
  if (phase === "pairing") {
    return <PairingScreen onPaired={() => setPhase("ready")} />;
  }
  const socket = socketRef.current;
  if (!socket) return <div className="pair muted">연결 준비 중…</div>;

  const clearFor = (agentId: string) => {
    void socket.rpc(RpcCmd.notificationsClear, { agentId }).catch(() => {
      /* 서버가 거부하면 이벤트로 되돌아오지 않으므로 그대로 둔다 */
    });
    setNotifications((prev) => ({ ...prev, [agentId]: [] }));
  };

  if (openAgent) {
    return view === "terminal" ? (
      <TerminalScreen
        socket={socket}
        agent={openAgent}
        permission={permission}
        onBack={() => setOpenAgentId(null)}
        onOpenChat={() => setView("chat")}
      />
    ) : (
      <ChatScreen
        socket={socket}
        agent={openAgent}
        permission={permission}
        notifications={notifications[openAgent.agentId] ?? []}
        onBack={() => setOpenAgentId(null)}
        onOpenTerminal={() => setView("terminal")}
        onClearNotifications={() => clearFor(openAgent.agentId)}
      />
    );
  }

  return (
    <AgentList
      socket={socket}
      agents={agents}
      permission={permission}
      hostName={hostName}
      connState={connState}
      notifications={notifications}
      onOpen={(a) => {
        setOpenAgentId(a.agentId);
        // 캐릭터를 열 때는 항상 채팅이 먼저다(터미널은 폴백).
        setView("chat");
      }}
      onClearNotifications={clearFor}
    />
  );
}
