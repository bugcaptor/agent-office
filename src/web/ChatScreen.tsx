// src/web/ChatScreen.tsx
//
// 채팅 뷰 — 웹 원격의 **주 화면**(docs/web-remote-design.md §2).
//
// 터미널 미러가 아니라 대화를 그린다. 재료는 셋이다:
//
//   · `chat` 프레임    — CLI가 남기는 JSONL 전사를 서버가 tail 해 구조화한 것
//   · `notification`   — 확인 요청(hook)은 상단 카드 + 퀵 키, stop/bell은 라인
//   · `activity`       — 턴 진행 표시(토큰 스트리밍이 없는 자리를 메운다)
//
// 입력은 **표준 `<input>`**이다. xterm의 숨은 textarea를 거치지 않으므로
// 모바일 IME(한글 조합)·소프트 키보드 문제군이 구조적으로 없다. 조합이 끝난
// 문장을 통째로 주입하는 것은 봇 모드의 선례와 같다.

import { useEffect, useMemo, useRef, useState } from "react";
import type {
  ClientPermission,
  HostMsg,
  NotificationItem,
  RemoteAgent,
  TranscriptItem,
} from "./protocol";
import { RpcCmd } from "./protocol";
import {
  activityLine,
  applyChatFrame,
  isAtBottom,
  isQuestion,
  itemGlyph,
  toolSummary,
  QUICK_KEYS,
} from "./chatView";
import type { WebRemoteSocket } from "./ws";

interface Props {
  socket: WebRemoteSocket;
  agent: RemoteAgent;
  permission: ClientPermission;
  notifications: NotificationItem[];
  onBack: () => void;
  onOpenTerminal: () => void;
  onClearNotifications: () => void;
}

export function ChatScreen({
  socket,
  agent,
  permission,
  notifications,
  onBack,
  onOpenTerminal,
  onClearNotifications,
}: Props) {
  const [items, setItems] = useState<TranscriptItem[]>([]);
  const [unavailable, setUnavailable] = useState(false);
  const [activity, setActivity] = useState<string | null>(null);
  const [draft, setDraft] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [hasNew, setHasNew] = useState(false);
  const [expanded, setExpanded] = useState<Set<number>>(new Set());
  const listRef = useRef<HTMLDivElement | null>(null);
  // 렌더 전에 "바닥을 보고 있었는가"를 기억해 둔다(레이아웃이 늘어난 뒤에
  // 판단하면 항상 false다).
  const stickRef = useRef(true);
  const agentId = agent.agentId;
  const canInput = permission === "input";

  useEffect(() => {
    setItems([]);
    setUnavailable(false);
    setActivity(null);
    setExpanded(new Set());
    stickRef.current = true;

    const follow = () => {
      socket.rpc(RpcCmd.chatFollow, { agentId }).catch((err: unknown) => {
        setError(err instanceof Error ? err.message : String(err));
      });
    };
    follow();
    // 재접속하면 서버의 구독도 새 연결에 다시 걸어야 한다.
    const offState = socket.onState((s) => {
      if (s === "open") follow();
    });

    const off = socket.onMessage((msg: HostMsg) => {
      if (msg.type === "chat" && msg.agentId === agentId) {
        if (msg.unavailable) {
          setUnavailable(true);
          return;
        }
        setUnavailable(false);
        const view = listRef.current;
        stickRef.current = view ? isAtBottom(view) : true;
        setItems((prev) => applyChatFrame(prev, msg));
        // 펼침 상태는 항목 인덱스에 매여 있다 — 교체 프레임이 오면 그 인덱스가
        // 다른 항목을 가리키므로 접어 둔다.
        if (msg.backfill) setExpanded(new Set());
        // 새 대화가 오면 진행 라인은 소임을 다했다.
        setActivity(null);
        if (!stickRef.current) setHasNew(true);
        return;
      }
      if (msg.type === "activity" && msg.agentId === agentId) {
        const line = activityLine(msg.payload as { kind?: string; text?: string });
        if (line) setActivity(line.text);
        return;
      }
    });

    return () => {
      off();
      offState();
    };
  }, [agentId, socket]);

  // 바닥을 보고 있었을 때만 따라간다. 위로 올려 읽는 중이면 배지만 띄운다.
  useEffect(() => {
    const view = listRef.current;
    if (!view) return;
    if (stickRef.current) {
      view.scrollTop = view.scrollHeight;
      setHasNew(false);
    }
  }, [items, activity]);

  const question = useMemo(
    () => notifications.filter(isQuestion).slice(-1)[0] ?? null,
    [notifications]
  );
  const otherNotices = useMemo(
    () => notifications.filter((n) => !isQuestion(n)),
    [notifications]
  );

  const sendKeys = (key: string) => {
    if (!canInput) return;
    setError(null);
    socket.rpc(RpcCmd.chatKeys, { agentId, keys: [key] }).catch((err: unknown) => {
      setError(err instanceof Error ? err.message : String(err));
    });
  };

  const submit = (e: React.FormEvent) => {
    e.preventDefault();
    const text = draft.trim();
    if (!text || !canInput) return;
    setDraft("");
    setError(null);
    stickRef.current = true;
    socket.rpc(RpcCmd.chatSend, { agentId, text }).catch((err: unknown) => {
      setError(err instanceof Error ? err.message : String(err));
      // 주입에 실패했으면 애써 친 문장을 돌려준다.
      setDraft(text);
    });
  };

  const toggle = (index: number) => {
    setExpanded((prev) => {
      const next = new Set(prev);
      if (next.has(index)) next.delete(index);
      else next.add(index);
      return next;
    });
  };

  return (
    <div className="chat-screen">
      <header className="bar">
        <button className="btn small" onClick={onBack}>
          ← 목록
        </button>
        <span className="title">{agent.name}</span>
        {!canInput && <span className="badge">읽기 전용</span>}
        <button className="btn small" onClick={onOpenTerminal}>
          터미널
        </button>
      </header>

      {question && (
        <div className="ask-card">
          <div className="ask-message">❓ {question.message}</div>
          {canInput && (
            <div className="ask-keys">
              {QUICK_KEYS.map((k) => (
                <button
                  key={k.key}
                  className="key"
                  onClick={() => sendKeys(k.key)}
                >
                  {k.label}
                </button>
              ))}
            </div>
          )}
          <button className="btn small" onClick={onClearNotifications}>
            알림 지우기
          </button>
        </div>
      )}

      {error && <p className="error pad">{error}</p>}

      <div className="chat-list" ref={listRef} onScroll={() => {
        const view = listRef.current;
        if (!view) return;
        stickRef.current = isAtBottom(view);
        if (stickRef.current) setHasNew(false);
      }}>
        {unavailable && (
          <div className="chat-empty">
            <p>이 세션은 전사가 없어 채팅을 표시할 수 없습니다.</p>
            <button className="btn primary" onClick={onOpenTerminal}>
              터미널로 보기
            </button>
          </div>
        )}
        {!unavailable && items.length === 0 && (
          <p className="muted pad">아직 대화가 없습니다.</p>
        )}
        {items.map((item, i) => {
          if (item.kind === "text") {
            return (
              <div
                key={i}
                className={`bubble ${item.role}${item.sidechain ? " side" : ""}`}
              >
                {item.text}
              </div>
            );
          }
          const open = expanded.has(i);
          return (
            <div
              key={i}
              className={`tool-line${item.isError ? " tool-error" : ""}`}
            >
              <button className="tool-head" onClick={() => toggle(i)}>
                {itemGlyph(item)} {toolSummary(item)}
              </button>
              {open && <pre className="tool-body">{item.text}</pre>}
            </div>
          );
        })}
        {otherNotices.map((n) => (
          <div key={n.id} className="notice-line">
            {n.source === "bell" ? "🔔" : "✅"} {n.message}
          </div>
        ))}
        {activity && <div className="activity-line">{activity}</div>}
      </div>

      {hasNew && (
        <button
          className="new-badge"
          onClick={() => {
            const view = listRef.current;
            if (view) view.scrollTop = view.scrollHeight;
            stickRef.current = true;
            setHasNew(false);
          }}
        >
          ↓ 새 메시지
        </button>
      )}

      {canInput && (
        <form className="chat-input" onSubmit={submit}>
          <input
            type="text"
            value={draft}
            placeholder="메시지 입력"
            autoCapitalize="off"
            autoCorrect="off"
            spellCheck={false}
            onChange={(e) => setDraft(e.target.value)}
          />
          <button className="btn primary" type="submit" disabled={!draft.trim()}>
            전송
          </button>
        </form>
      )}
    </div>
  );
}
