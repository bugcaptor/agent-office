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
import { useTranslation } from "react-i18next";
import { useShallow } from "zustand/react/shallow";
import { useAppStore } from "../store/appStore";
import { tauriApi } from "../ipc/tauriApi";
import { ensureSession } from "../ipc/sessionBridge";
import { terminalRegistry } from "./TerminalRegistry";
import { useTaskChatStore } from "../workpanels/taskChatStore";
import { useDiscussionSessionStore } from "../discussion/discussionSessionStore";
import { useOfficeAgentStore } from "../officeAgent/officeAgentStore";
import { useChatViewStore } from "../chatView/chatViewStore";
import { ChatView } from "../chatView/ChatView";
import { ShellAssistBar } from "./ShellAssistBar";

const RESIZE_DEBOUNCE_MS = 120;

export function TerminalHost() {
  // Every agent that needs a live (or previously-live) session mount.
  const agentIds = useAppStore(
    useShallow((s) => s.agentOrder.filter((id) => s.sessions[id]?.status !== "idle"))
  );
  const epochs = useAppStore(useShallow((s) => s.terminalEpochs));
  // 태스크 대화 탭. 캐릭터 뒤에 같은 방식으로 붙는다.
  const chatIds = useTaskChatStore(useShallow((s) => s.order));
  // 토의 참가자 탭. 대화 탭과 같은 방식으로 붙는다.
  const discussionIds = useDiscussionSessionStore(useShallow((s) => s.order));
  // 일반 에이전트 탭.
  const officeIds = useOfficeAgentStore(useShallow((s) => s.order));

  return (
    <div className="terminal-host">
      {agentIds.map((id) => (
        // key에 에폭을 포함: 터미널 재시작(restartAgentSession의
        // bumpTerminalEpoch)이 에폭을 올리면 강제 리마운트되어, attach()가
        // (registry.destroy로 폐기된) 새 xterm을 다시 만든다.
        <TerminalMount key={`${id}#${epochs[id] ?? 0}`} agentId={id} />
      ))}
      {chatIds.map((id) => (
        <TerminalMount key={id} agentId={id} chat />
      ))}
      {discussionIds.map((id) => (
        <TerminalMount key={id} agentId={id} chat discussion />
      ))}
      {officeIds.map((id) => (
        <TerminalMount key={id} agentId={id} chat office />
      ))}
    </div>
  );
}

function TerminalMount({ agentId, chat = false, discussion = false, office = false }: { agentId: string; chat?: boolean; discussion?: boolean; office?: boolean }) {
  const { t } = useTranslation("terminal");
  const hostRef = useRef<HTMLDivElement>(null);
  const isActive = useAppStore((s) => s.activeTerminalAgentId === agentId);
  const appExited = useAppStore((s) => s.sessions[agentId]?.status === "exited");
  const chatExited = useTaskChatStore((s) => s.chats[agentId]?.status === "exited");
  const discussionExited = useDiscussionSessionStore((s) => s.sessions[agentId]?.status === "exited");
  const officeExited = useOfficeAgentStore((s) => s.sessions[agentId]?.status === "exited");
  const isExited = office ? officeExited : discussion ? discussionExited : chat ? chatExited : appExited;
  const setSessionSize = useAppStore((s) => s.setSessionSize);
  // 대화 보기: xterm은 그대로 두고 그 위를 덮는다(keep-alive 불변식 유지).
  const chatView = useChatViewStore((s) => !!s.on[agentId]);
  const setChatView = useChatViewStore((s) => s.set);
  // 셸 캐릭터: 터미널 위에 말 → 명령 입력줄을 둔다.
  const shellAssist = useAppStore((s) => !chat && s.agents[agentId]?.shellAssist === true);

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
      className={shellAssist ? "terminal-mount has-shell-assist" : "terminal-mount"}
      style={{ display: isActive ? (shellAssist ? "flex" : "block") : "none" }}
      data-agent-id={agentId}
    >
      {shellAssist && <ShellAssistBar agentId={agentId} />}
      <div ref={hostRef} className="terminal-mount-host" />
      {chatView && (
        <ChatView
          agentId={agentId}
          active={isActive}
          onShowTerminal={() => {
            setChatView(agentId, false);
            terminalRegistry.get(agentId)?.term.focus();
          }}
        />
      )}
      {isExited && (
        <div className="terminal-exited-banner" role="alert">
          {/* 대화는 다시 띄우지 않는다 — 탭을 닫고 태스크에서 새로 연다(맥락 묶음이 새로 만들어진다). */}
          <span>{office ? t("host.officeAgentEnded") : discussion ? t("host.discussionEnded") : chat ? t("host.chatEnded") : t("host.exited")}</span>
          {!chat && (
            <button type="button" className="pixel-btn primary" onClick={relaunch}>
              {t("host.relaunch")}
            </button>
          )}
        </div>
      )}
    </div>
  );
}
