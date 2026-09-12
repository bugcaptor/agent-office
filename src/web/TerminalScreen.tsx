// src/web/TerminalScreen.tsx
//
// 원격 터미널 한 대. 서버가 주는 `restore`(화면 스냅샷 + 기준 오프셋) 뒤에
// `output`을 이어 붙인다. 오프셋이 어긋나면(구멍) 그 자리에서 재-attach 한다 —
// 스냅샷은 **계수하지 않는 화면 이미지**라는 규칙(#49)이 서버에도 그대로 있다.
//
// 크기 소유권은 호스트 단독이다(Phase 1). 뷰포트에 맞춰 PTY를 리사이즈하는
// 대신 폰트를 줄여 호스트가 정한 열 수를 그대로 담는다.

import { useEffect, useRef, useState } from "react";
import { Terminal } from "@xterm/xterm";
import "@xterm/xterm/css/xterm.css";
import type { HostMsg, RemoteAgent, ClientPermission } from "./protocol";
import type { WebRemoteSocket } from "./ws";
import { KeyBar } from "./KeyBar";
import { createScrollbackGuard } from "./scrollbackGuard";

interface Props {
  socket: WebRemoteSocket;
  agent: RemoteAgent;
  permission: ClientPermission;
  onBack: () => void;
  /** 주 화면(채팅 뷰)으로 돌아간다 — 미러는 폴백이다(M2). */
  onOpenChat: () => void;
}

/** 컨테이너 폭에 호스트가 정한 열 수를 담기 위한 폰트 크기(px). */
function fitFontSize(containerWidth: number, cols: number): number {
  if (!cols || cols <= 0) return 13;
  // xterm 기본 폰트의 문자 폭 ≈ 0.6em. 여유 2px.
  const per = containerWidth / cols;
  return Math.max(7, Math.min(16, Math.floor(per / 0.62)));
}

export function TerminalScreen({
  socket,
  agent,
  permission,
  onBack,
  onOpenChat,
}: Props) {
  const mountRef = useRef<HTMLDivElement | null>(null);
  const termRef = useRef<Terminal | null>(null);
  const offsetRef = useRef<number | null>(null);
  const [status, setStatus] = useState<string>("연결 중…");

  useEffect(() => {
    const mount = mountRef.current;
    if (!mount) return;
    const cols = agent.cols && agent.cols > 0 ? agent.cols : 80;
    const rows = agent.rows && agent.rows > 0 ? agent.rows : 24;
    const term = new Terminal({
      cols,
      rows,
      fontSize: fitFontSize(mount.clientWidth, cols),
      fontFamily: "ui-monospace, SFMono-Regular, Menlo, monospace",
      cursorBlink: true,
      convertEol: false,
      // 호스트가 크기의 주인이라 자동 맞춤을 켜지 않는다.
      scrollback: 2000,
      theme: { background: "#16181a", foreground: "#e8e6e1" },
    });
    term.open(mount);
    termRef.current = term;
    const scrollbackGuard = createScrollbackGuard();
    let resyncPending = false;

    const onData = term.onData((data) => {
      if (permission !== "input") return;
      socket.send({ type: "input", agentId: agent.agentId, data });
    });

    // 탭 진입은 전체 복원부터 시작한다. 이후 socket이 다시 열리면 이 지점부터
    // attach해, 서버 연결별 attach 상태가 사라져도 터미널 구독이 복원된다.
    offsetRef.current = null;
    const attach = () => {
      socket.send({
        type: "attach",
        agentId: agent.agentId,
        lastOffset: offsetRef.current,
      });
    };
    const offState = socket.onState((state) => {
      if (state === "open") attach();
    });

    const off = socket.onMessage((msg: HostMsg) => {
      if (msg.type === "restore" && msg.agentId === agent.agentId) {
        // snapshot:null은 링버퍼 델타만 보내는 재동기화다. 여기서 화면을
        // 지우면 이미 보던 과거 출력이 사라진다. 빈 문자열은 유효한 스냅샷일
        // 수 있으므로 null과 구별해 화면을 비운다.
        if (msg.snapshot !== null && msg.snapshot !== undefined) {
          term.reset();
          scrollbackGuard.reset();
          if (msg.snapshot) term.write(msg.snapshot);
        }
        offsetRef.current = msg.baseOffset;
        resyncPending = false;
        setStatus("");
        return;
      }
      if (msg.type === "output" && msg.agentId === agent.agentId) {
        const expected = offsetRef.current;
        if (expected !== null && msg.offset !== expected) {
          // 구멍 — 이 캐릭터만 마지막 지점부터 다시 받는다.
          if (!resyncPending) {
            resyncPending = true;
            attach();
          }
          return;
        }
        term.write(scrollbackGuard.filter(msg.data));
        offsetRef.current = msg.offset + msg.bytes;
        return;
      }
      if (msg.type === "resized" && msg.agentId === agent.agentId) {
        if (msg.cols > 0 && msg.rows > 0) {
          term.resize(msg.cols, msg.rows);
          term.options.fontSize = fitFontSize(mount.clientWidth, msg.cols);
        }
      }
    });

    const onResize = () => {
      term.options.fontSize = fitFontSize(mount.clientWidth, term.cols);
    };
    window.addEventListener("resize", onResize);
    window.visualViewport?.addEventListener("resize", onResize);

    return () => {
      window.removeEventListener("resize", onResize);
      window.visualViewport?.removeEventListener("resize", onResize);
      off();
      offState();
      onData.dispose();
      socket.send({ type: "detach", agentId: agent.agentId });
      term.dispose();
      termRef.current = null;
    };
  }, [agent.agentId, agent.cols, agent.rows, permission, socket]);

  const sendKey = (data: string) => {
    if (permission !== "input") return;
    socket.send({ type: "input", agentId: agent.agentId, data });
    termRef.current?.focus();
  };

  return (
    <div className="term-screen">
      <header className="bar">
        <button className="btn small" onClick={onBack}>
          ← 목록
        </button>
        <span className="title">{agent.name}</span>
        {permission !== "input" && <span className="badge">읽기 전용</span>}
        <button className="btn small" onClick={onOpenChat}>
          채팅
        </button>
      </header>
      {status && <div className="muted pad">{status}</div>}
      <div className="term-mount" ref={mountRef} />
      {permission === "input" && <KeyBar onKey={sendKey} />}
    </div>
  );
}
