// src/web/App.tsx
//
// 웹 클라이언트 루트. 상태는 **전부 서버 push에서 온다** — 로컬 영속이 없으므로
// 데스크톱 앱과 스토어가 두 벌이 되는 문제가 애초에 없다(#7m A 결정의 핵심).
//
// 접속 직후 **모든 캐릭터에 attach** 한다: 서버는 attach된 캐릭터의 메시지만
// 그 연결로 흘리므로(agent-bound 필터), 알림·상태를 받으려면 붙어 있어야 한다.
// 활성 탭이 아닌 캐릭터의 출력은 화면에 쓰지 않고 버린다.

import { useEffect, useMemo, useRef, useState } from "react";
import { AgentList } from "./AgentList";
import { PairingScreen } from "./PairingScreen";
import { TerminalScreen } from "./TerminalScreen";
import type { HostMsg, NotificationItem, PeerAgent, PeerPermission } from "./protocol";
import { RpcCmd } from "./protocol";
import { PeerSocket, probeAuth, type ConnState } from "./ws";

type Phase = "checking" | "pairing" | "ready";

export function App() {
  const [phase, setPhase] = useState<Phase>("checking");
  const [agents, setAgents] = useState<PeerAgent[]>([]);
  const [permission, setPermission] = useState<PeerPermission>("readOnly");
  const [hostName, setHostName] = useState("Agent Office");
  const [connState, setConnState] = useState<ConnState>("closed");
  const [openAgentId, setOpenAgentId] = useState<string | null>(null);
  const [notifications, setNotifications] = useState<Record<string, NotificationItem[]>>({});
  const socketRef = useRef<PeerSocket | null>(null);
  const attachedRef = useRef<Set<string>>(new Set());

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
    const socket = new PeerSocket();
    socketRef.current = socket;

    const offState = socket.onState(setConnState);
    const off = socket.onMessage((msg: HostMsg) => {
      switch (msg.type) {
        case "hello":
          setPermission(msg.permission);
          setHostName(msg.hostName);
          break;
        case "agents": {
          setAgents(msg.agents);
          // 새로 등장한 캐릭터에 붙는다(알림·상태 수신 조건).
          for (const a of msg.agents) {
            if (!attachedRef.current.has(a.agentId)) {
              attachedRef.current.add(a.agentId);
              socket.send({ type: "attach", agentId: a.agentId, lastOffset: null });
            }
          }
          break;
        }
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
    // 재접속 때는 attach를 다시 걸어야 한다.
    const offReattach = socket.onState((s) => {
      if (s === "open") {
        for (const id of attachedRef.current) {
          socket.send({ type: "attach", agentId: id, lastOffset: null });
        }
      }
    });

    return () => {
      off();
      offState();
      offReattach();
      socket.dispose();
      socketRef.current = null;
      attachedRef.current.clear();
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

  if (openAgent) {
    return (
      <TerminalScreen
        socket={socket}
        agent={openAgent}
        permission={permission}
        onBack={() => setOpenAgentId(null)}
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
      onOpen={(a) => setOpenAgentId(a.agentId)}
      onClearNotifications={(agentId) => {
        void socket.rpc(RpcCmd.notificationsClear, { agentId }).catch(() => {
          /* 서버가 거부하면 이벤트로 되돌아오지 않으므로 그대로 둔다 */
        });
        setNotifications((prev) => ({ ...prev, [agentId]: [] }));
      }}
    />
  );
}
