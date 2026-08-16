// src/renderer/terminal/TerminalHost.tsx
//
// Keep-alive xterm mount tree. Renders one
// `TerminalMount` per agent that has (or has had) a session — i.e. every
// non-`idle` session — and never unmounts them individually; the active
// agent's mount is `display:block`, every other mount is `display:none`.
// Actual xterm lifecycle (create/open/dispose) lives in `TerminalRegistry`
// — this component only decides *which* container is visible and
// when to (re)fit it.
//
// 예외: 앱 밖 터미널에 붙인 외부(논리) 세션(`SessionRuntime.kind === "external"`)은
// PTY가 없어 미러링할 출력이 없다 — xterm을 아예 만들지 않고 안내 패널 +
// "연결 해제" 버튼만 그린다(계획 M4).
//
// Deviation from the original design skeleton: that skeleton's `window.api`
// is the `tauriApi` module, so `resize` is called on that
// import directly instead of a `window.api` global.
import { useEffect, useRef, useState } from "react";
import { useShallow } from "zustand/react/shallow";
import { useAppStore } from "../store/appStore";
import { tauriApi } from "../ipc/tauriApi";
import { ensureSession } from "../ipc/sessionBridge";
import { terminalRegistry } from "./TerminalRegistry";
import { BotOverlay } from "./BotOverlay";

const RESIZE_DEBOUNCE_MS = 120;

export function TerminalHost() {
  // Every agent that needs a live (or previously-live) session mount.
  const agentIds = useAppStore(
    useShallow((s) => s.agentOrder.filter((id) => s.sessions[id]?.status !== "idle"))
  );
  const epochs = useAppStore(useShallow((s) => s.terminalEpochs));

  return (
    <div className="terminal-host">
      {agentIds.map((id) => (
        // key에 에폭을 포함: 터미널 재시작(restartAgentSession의
        // bumpTerminalEpoch)이 에폭을 올리면 강제 리마운트되어, attach()가
        // (registry.destroy로 폐기된) 새 xterm을 다시 만든다.
        <TerminalMount key={`${id}#${epochs[id] ?? 0}`} agentId={id} />
      ))}
    </div>
  );
}

/**
 * 마운트 분기: 앱 밖 터미널에 붙인 외부(논리) 세션은 PTY가 없어 미러링할
 * 출력 스트림이 없다 — xterm을 만들지 않고 placeholder만 그린다. 그 외(PTY
 * 세션·tmux 미러 세션 포함)는 기존 경로 그대로.
 */
function TerminalMount({ agentId }: { agentId: string }) {
  const isExternal = useAppStore(
    (s) => s.sessions[agentId]?.kind === "external" && s.sessions[agentId]?.status === "running"
  );
  return isExternal ? <ExternalMount agentId={agentId} /> : <PtyMount agentId={agentId} />;
}

function ExternalMount({ agentId }: { agentId: string }) {
  const isActive = useAppStore((s) => s.activeTerminalAgentId === agentId);
  const [detaching, setDetaching] = useState(false);

  const detach = () => {
    setDetaching(true);
    // 성공하면 백엔드가 session-state(disposed)를 방출해 이 마운트가 알아서
    // 사라진다. 실패해도 버튼은 다시 눌러볼 수 있게 되돌린다.
    void tauriApi
      .detachExternalSession(agentId)
      .catch((err) => console.warn(`detachExternalSession failed for ${agentId}`, err))
      .finally(() => setDetaching(false));
  };

  return (
    <div
      className="terminal-mount"
      style={{ display: isActive ? "block" : "none" }}
      data-agent-id={agentId}
    >
      <div className="terminal-external-panel" role="status">
        <span className="terminal-external-icon" aria-hidden="true">
          🔗
        </span>
        <div className="terminal-external-title">외부 터미널 세션에 연결됨</div>
        <div className="terminal-external-detail">
          앱 밖 터미널의 세션이라 화면 미러링은 없습니다. 알림과 성격 프롬프트만 이
          캐릭터를 통해 동작합니다.
        </div>
        <button
          type="button"
          className="pixel-btn"
          onClick={detach}
          disabled={detaching}
        >
          연결 해제
        </button>
      </div>
    </div>
  );
}

function PtyMount({ agentId }: { agentId: string }) {
  const hostRef = useRef<HTMLDivElement>(null);
  const isActive = useAppStore((s) => s.activeTerminalAgentId === agentId);
  const isExited = useAppStore((s) => s.sessions[agentId]?.status === "exited");
  const setSessionSize = useAppStore((s) => s.setSessionSize);

  // First mount only: attach the (keep-alive) registry container. Never
  // detaches/destroys on unmount — only `removeAgent` -> registry.destroy()
  // does that, elsewhere.
  useEffect(() => {
    if (hostRef.current) terminalRegistry.attach(agentId, hostRef.current);
  }, [agentId]);

  // Becoming active: fit (after layout settles) + report size + focus.
  useEffect(() => {
    if (!isActive) return;
    terminalRegistry.activate(agentId, (cols, rows) => {
      setSessionSize(agentId, cols, rows);
      tauriApi.resize(agentId, cols, rows);
    });
  }, [isActive, agentId, setSessionSize]);

  // Active-only ResizeObserver, debounced, calling refit (not activate — a
  // plain container resize shouldn't re-focus or re-scroll).
  useEffect(() => {
    if (!isActive || !hostRef.current) return;
    let t: ReturnType<typeof setTimeout> | undefined;
    const ro = new ResizeObserver(() => {
      clearTimeout(t);
      t = setTimeout(() => {
        terminalRegistry.refit(agentId, (cols, rows) => {
          setSessionSize(agentId, cols, rows);
          tauriApi.resize(agentId, cols, rows);
        });
      }, RESIZE_DEBOUNCE_MS);
    });
    ro.observe(hostRef.current);
    return () => {
      clearTimeout(t);
      ro.disconnect();
    };
  }, [isActive, agentId, setSessionSize]);

  const relaunch = () => {
    ensureSession(agentId);
    terminalRegistry.get(agentId)?.term.focus();
  };

  return (
    <div
      className="terminal-mount"
      style={{ display: isActive ? "block" : "none" }}
      data-agent-id={agentId}
    >
      <div ref={hostRef} className="terminal-mount-host" />
      {/* 봇 운전 중이면 터미널을 덮는 클릭 블로커 + 상태 배너(이슈 #57). 봇 모드가
          꺼진 탭에선 self-gate로 null 렌더 — 아무 것도 덮지 않는다. */}
      <BotOverlay agentId={agentId} />
      {isExited && (
        <div className="terminal-exited-banner" role="alert">
          <span>프로세스가 종료되었습니다.</span>
          <button type="button" className="pixel-btn primary" onClick={relaunch}>
            다시 띄우기
          </button>
        </div>
      )}
    </div>
  );
}
