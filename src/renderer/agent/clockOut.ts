// src/renderer/agent/clockOut.ts
//
// 퇴근/소환 오케스트레이터. `deleteAgent.ts`와 같은 순서 원칙을 따르되,
// 프로필 자체는 지우지 않는(되돌릴 수 있는) 사촌이다:
//   ① tauriApi.disposeSession — PTY 종료. 세션이 없거나 이미 죽었어도 무시.
//   ② store.clockOut — clockedOut=true 플래그 + 세션 런타임/최근탭 정리
//      (활성 탭이었다면 이웃 탭으로 전환, 없으면 오버레이가 닫힘).
//   ③ terminalRegistry.destroy — xterm 인스턴스/DOM 정리(터미널이 사라짐).
//
// 프로필/초상/스프라이트/timeTracking은 건드리지 않는다 — 소환하면 그대로
// 복원된다(터미널 스크롤백만 세션 종료로 사라지는 게 의도된 동작).
// 오피스 캔버스에서 사라지는 것은 `useAgentList` 셀렉터가 clockedOut을
// 필터링하기 때문이며, OfficeWorld.syncAgents가 이를 구독해 처리한다.
//
// 소환(clockInAgent)은 플래그 해제 + 에폭 증가(restartAgentSession.ts와 동일한
// 원리로 TerminalMount를 강제 리마운트 — 안 그러면 퇴근 때 destroy된 xterm
// 자리가 빈 화면으로 남는다) + officeBus.emitAgentClicked(세션 생성/터미널
// 오픈/알림 클리어) 순서로 진행한다.
import { useAppStore } from "../store/appStore";
import { tauriApi } from "../ipc/tauriApi";
import { terminalRegistry } from "../terminal/TerminalRegistry";
import { officeBus } from "../ipc/sessionBridge";

export async function clockOutAgent(agentId: string): Promise<void> {
  // ① PTY 종료(세션 종료 → 시간 집계 정산은 sessionBridge의 onSessionState가 처리).
  try {
    await tauriApi.disposeSession(agentId);
  } catch (err) {
    console.warn(`clockOutAgent: disposeSession failed for ${agentId}`, err);
  }
  // ② 스토어: clockedOut=true + 런타임 정리(활성 탭 이웃 전환 포함).
  useAppStore.getState().clockOut(agentId);
  // ③ xterm 인스턴스/DOM 정리(터미널이 사라짐).
  terminalRegistry.destroy(agentId);
}

export async function clockOutAll(): Promise<void> {
  // 근무 중(=clockedOut 아님)인 에이전트 전부 퇴근. 스냅샷을 먼저 떠서 반복.
  const { agents, agentOrder } = useAppStore.getState();
  const onDuty = agentOrder.filter((id) => agents[id] && !agents[id].clockedOut);
  for (const id of onDuty) {
    // 순차 처리: 각자 dispose await. 병렬 dispose는 백엔드 부담이 커서 지양.
    // eslint-disable-next-line no-await-in-loop
    await clockOutAgent(id);
  }
}

export function clockInAgent(agentId: string): void {
  const { clockIn, bumpTerminalEpoch } = useAppStore.getState();
  // ① 플래그 해제 → 캔버스 재등장(useAgentList 필터 통과).
  clockIn(agentId);
  // ② 퇴근 시 terminalRegistry.destroy로 xterm을 폐기했으므로, 에폭을 올려
  //    TerminalMount를 강제 리마운트해야 attach()가 새 xterm을 만든다.
  //    안 그러면 재소환 시 빈 화면(restartAgentSession과 동일한 원리).
  bumpTerminalEpoch(agentId);
  // ③ 세션 생성 + 터미널 오픈 + 알림 클리어.
  officeBus.emitAgentClicked(agentId);
}
