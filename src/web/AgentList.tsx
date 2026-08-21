// src/web/AgentList.tsx
//
// 폰 첫 화면. "5분 안에: 상태 확인 → 알림 처리 → 터미널 개입 → 죽은 세션
// 재기동 → 폭주 종료"가 이 화면의 설계 기준이다(#7m Phase 1 커트라인).

import { useEffect, useState } from "react";
import type { NotificationItem, RemoteAgent, ClientPermission } from "./protocol";
import { RpcCmd } from "./protocol";
import { AgentAvatar } from "./AgentAvatar";
import * as notify from "./notify";
import type { ConnState, WebRemoteSocket } from "./ws";

interface Props {
  socket: WebRemoteSocket;
  agents: RemoteAgent[];
  permission: ClientPermission;
  hostName: string;
  connState: ConnState;
  notifications: Record<string, NotificationItem[]>;
  onOpen: (agent: RemoteAgent) => void;
  onClearNotifications: (agentId: string) => void;
}

const STATE_LABEL: Record<string, string> = {
  running: "작업 중",
  starting: "시작 중",
  exited: "종료됨",
  disposed: "종료됨",
};

interface UsageWindow {
  kind: string;
  label?: string | null;
  usedPercent: number;
}
interface UsageSnapshot {
  claude?: { windows: UsageWindow[] } | null;
  codex?: { windows: UsageWindow[] } | null;
}

export function AgentList({
  socket,
  agents,
  permission,
  hostName,
  connState,
  notifications,
  onOpen,
  onClearNotifications,
}: Props) {
  const [usage, setUsage] = useState<UsageSnapshot | null>(null);
  const [busy, setBusy] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  // 브라우저 알림 토글. 저장소는 localStorage고 App.tsx가 알림을 띄울 때
  // 직접 다시 읽는다 — 상태를 위로 끌어올리지 않아도 한 벌로 유지된다.
  const [notifyOn, setNotifyOn] = useState(() => notify.enabled());
  const [notifyHint, setNotifyHint] = useState<string | null>(null);

  useEffect(() => {
    let alive = true;
    const poll = () => {
      socket
        .rpc<UsageSnapshot>(RpcCmd.usageSnapshot)
        .then((snap) => {
          if (alive) setUsage(snap);
        })
        .catch(() => {
          /* 사용량은 부가 정보 — 실패해도 화면은 그대로 */
        });
    };
    poll();
    const timer = setInterval(poll, 60_000);
    return () => {
      alive = false;
      clearInterval(timer);
    };
  }, [socket]);

  const act = async (agentId: string, cmd: string) => {
    setBusy(agentId);
    setError(null);
    try {
      await socket.rpc(cmd, { agentId });
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setBusy(null);
    }
  };

  // 켤 때만 권한을 묻는다(끄기는 즉시). 거부는 사이트 설정에서만 되돌릴 수
  // 있으므로 안내 문구를 잠깐 띄우고 토글은 꺼진 채로 둔다.
  const toggleNotify = async () => {
    setNotifyHint(null);
    if (notifyOn) {
      notify.setEnabled(false);
      setNotifyOn(false);
      return;
    }
    const perm = await notify.requestPermission();
    if (perm !== "granted") {
      setNotifyHint("브라우저가 알림을 거부했습니다. 사이트 설정에서 허용해 주세요.");
      setTimeout(() => setNotifyHint(null), 4000);
      return;
    }
    notify.setEnabled(true);
    setNotifyOn(true);
  };

  const session = usage?.claude?.windows.find((w) => w.kind === "session");

  return (
    <div className="list-screen">
      <header className="bar">
        <span className="title">{hostName}</span>
        <span className={`dot ${connState}`} title={connState} />
        {session && <span className="usage">세션 {Math.round(session.usedPercent)}%</span>}
        {/* Notification API 자체가 없는 환경(iOS Safari 비-PWA 등)은 버튼을
            아예 그리지 않는다 — 눌러도 아무 일이 없는 버튼은 두지 않는다. */}
        {notify.supported() && (
          <button
            className="btn small"
            title={notifyOn ? "브라우저 알림 끄기" : "브라우저 알림 켜기"}
            onClick={() => void toggleNotify()}
          >
            {notifyOn ? "🔔" : "🔕"}
          </button>
        )}
      </header>

      {notifyHint && <p className="muted pad">{notifyHint}</p>}
      {error && <p className="error pad">{error}</p>}
      {agents.length === 0 && <p className="muted pad">보여줄 캐릭터가 없습니다.</p>}

      <ul className="cards">
        {agents.map((a) => {
          const pending = notifications[a.agentId] ?? [];
          const running = a.state === "running" || a.state === "starting";
          return (
            <li key={a.agentId} className="card">
              <button className="card-main" onClick={() => onOpen(a)}>
                <span className="name">
                  <AgentAvatar socket={socket} agent={a} size={22} />
                  {a.name}
                </span>
                <span className="meta">
                  {a.role || "역할 없음"} · {STATE_LABEL[a.state ?? ""] ?? "세션 없음"}
                </span>
                {pending.length > 0 && (
                  <span className="notif">알림 {pending.length}</span>
                )}
              </button>
              <div className="card-actions">
                {pending.length > 0 && (
                  <button
                    className="btn small"
                    onClick={() => onClearNotifications(a.agentId)}
                  >
                    알림 지우기
                  </button>
                )}
                {permission === "input" &&
                  (running ? (
                    <button
                      className="btn small danger"
                      disabled={busy === a.agentId}
                      onClick={() => void act(a.agentId, RpcCmd.sessionDispose)}
                    >
                      종료
                    </button>
                  ) : (
                    <button
                      className="btn small"
                      disabled={busy === a.agentId}
                      onClick={() => void act(a.agentId, RpcCmd.sessionStart)}
                    >
                      시작
                    </button>
                  ))}
              </div>
            </li>
          );
        })}
      </ul>
    </div>
  );
}
