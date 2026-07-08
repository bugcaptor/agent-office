// src/renderer/agent/deleteAgent.ts
//
// 캐릭터 삭제 오케스트레이터. 순서를 보장한다:
//   ① tauriApi.disposeSession — PTY 종료. 세션이 없거나 이미 죽었어도 무시.
//      이 await 이후에 스토어 상태를 읽는다 — await 이전에 읽으면 IPC 대기
//      중 스토어가 변할 수 있어(예: 겹치는 삭제 호출) 낡은 상태로 판단하게
//      된다.
//   ② 삭제 대상이 활성 탭이면 인접(다음, 없으면 이전) 에이전트로 활성 탭
//      전환. removeAgent가 활성 탭을 null로 만들기 전에, disposeSession 이후
//      시점의 탭 순서(recentAgentIds) 기준으로 이웃을 계산해 옮긴다.
//   ③ removeAgent — 스토어 캐스케이드 (agents/sessions/portraits/
//      spritePreviews/agentOrder/recentAgentIds/notifications).
//   ④ terminalRegistry.destroy — xterm 인스턴스/DOM 정리 (기존 미연결 지점).
//
// 파일 삭제(profiles.json / <agentId>.png)와 오피스 엔티티 파괴는 스토어
// 변경을 구독하는 기존 persist/캐시/OfficeWorld.syncAgents가 자동 처리하므로
// 여기서 다루지 않는다.
import { useAppStore } from "../store/appStore";
import { tauriApi } from "../ipc/tauriApi";
import { terminalRegistry } from "../terminal/TerminalRegistry";

export async function deleteAgent(agentId: string): Promise<void> {
  // ① PTY 종료. 실패(세션 없음/이미 종료)해도 삭제는 계속 진행.
  try {
    await tauriApi.disposeSession(agentId);
  } catch (err) {
    console.warn(`deleteAgent: disposeSession failed for ${agentId}`, err);
  }

  // 상태 스냅샷은 disposeSession await 이후에 읽는다. await 이전에 읽으면
  // IPC 대기 중 스토어가 변한 경우(예: 겹치는 삭제 호출) 낡은 상태로 탭
  // 전환을 결정하게 되어, 유효한 이웃 탭 대신 활성 탭이 null이 될 수 있다.
  const { recentAgentIds, activeTerminalAgentId, removeAgent, openTerminal } =
    useAppStore.getState();

  // ② 활성 탭이면 인접 에이전트로 전환. removeAgent(활성 탭 null화) 전에,
  //    최신 recentAgentIds에서 이웃을 계산한다.
  if (activeTerminalAgentId === agentId) {
    const idx = recentAgentIds.indexOf(agentId);
    const next = recentAgentIds[idx + 1] ?? recentAgentIds[idx - 1] ?? null;
    // next를 먼저 활성화하면 이어지는 removeAgent는 활성 탭이 이미
    // agentId가 아니므로 null로 덮어쓰지 않는다. 이웃이 없으면(마지막 탭)
    // 아무것도 하지 않고, removeAgent가 활성 탭을 null로 만든다.
    if (next) openTerminal(next);
  }

  // ③ 스토어 캐스케이드 제거.
  removeAgent(agentId);

  // ④ xterm 정리.
  terminalRegistry.destroy(agentId);
}
