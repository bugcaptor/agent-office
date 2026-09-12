// VS Code 표시 확장의 focus 요청을 본체에서 처리한다. officeBus를 거치면
// 세션을 자동 생성할 수 있으므로, 프로필 모달만 여는 별도 경로를 둔다.
import { listen } from "@tauri-apps/api/event";
import { Events } from "@shared/ipc";
import { useAppStore } from "../store/appStore";

export function installDisplayFocusBridge(): () => void {
  let unlisten: (() => void) | null = null;
  let disposed = false;
  void listen<{ agentId?: unknown; intent?: unknown }>(Events.displayFocusAgent, (event) => {
    const agentId = event.payload?.agentId;
    if (typeof agentId !== "string") return;
    const agent = useAppStore.getState().agents[agentId];
    if (!agent) return;
    if (event.payload?.intent === "connectCodex") {
      useAppStore.getState().openModal({
        kind: "ide-session",
        initialAgentId: agentId,
        initialCwd: agent.cwd,
        initialProvider: "codex",
      });
      return;
    }
    useAppStore.getState().openModal({ kind: "profile-edit", agentId });
  }).then((off) => {
    if (disposed) off();
    else unlisten = off;
  });
  return () => {
    if (disposed) return;
    disposed = true;
    unlisten?.();
    unlisten = null;
  };
}
