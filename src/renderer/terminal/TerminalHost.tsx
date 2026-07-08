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
// Deviation from the original design skeleton: that skeleton's `window.api`
// is the `tauriApi` module, so `resize` is called on that
// import directly instead of a `window.api` global.
import { useEffect, useRef } from "react";
import { useShallow } from "zustand/react/shallow";
import { useAppStore } from "../store/appStore";
import { tauriApi } from "../ipc/tauriApi";
import { ensureSession } from "../ipc/sessionBridge";
import { terminalRegistry } from "./TerminalRegistry";

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

function TerminalMount({ agentId }: { agentId: string }) {
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
