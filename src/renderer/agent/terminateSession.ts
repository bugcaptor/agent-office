// src/renderer/agent/terminateSession.ts
//
// 터미널 종료(탕비실 보내기) 오케스트레이터. restartAgentSession의 ①만 있는
// 사촌 — PTY를 죽이되 재생성하지 않는다:
//   ① tauriApi.disposeSession — PTY 종료. 세션이 없거나 이미 죽었어도 무시.
//   ② tauriApi.clearNotifications — 대기 알림 제거. 알림(hasPending)이 남으면
//      캐릭터가 자리를 지켜 탕비실로 가지 않으므로(behaviorFsm) 함께 지운다.
//
// 스토어/xterm은 건드리지 않는다. 상태 전이는 백엔드 disposed 이벤트 →
// sessionBridge의 exited 정규화가 담당하고, 그 결과 자연 종료(exit)와 동일한
// 최종 상태가 된다: exited 배너 + 스크롤백 유지, 캐릭터는 FSM 규칙대로
// 탕비실행, 캐릭터 클릭(ensureSession)이나 배너 "다시 띄우기"로 재소환.
import { tauriApi } from "../ipc/tauriApi";

export async function terminateAgentSession(agentId: string): Promise<void> {
  // ① PTY 종료 — 세션이 없거나 이미 죽었어도 계속(clockOut과 동일한 관용).
  try {
    await tauriApi.disposeSession(agentId);
  } catch (err) {
    console.warn(`terminateAgentSession: disposeSession failed for ${agentId}`, err);
  }
  // ② 대기 알림 제거 — 실패해도 종료 자체는 완료된 것으로 본다.
  try {
    await tauriApi.clearNotifications(agentId);
  } catch (err) {
    console.warn(`terminateAgentSession: clearNotifications failed for ${agentId}`, err);
  }
}
