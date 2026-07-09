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
// 출근(clockInAgent)은 플래그 해제 + 에폭 증가(restartAgentSession.ts와 동일한
// 원리로 TerminalMount를 강제 리마운트 — 안 그러면 퇴근 때 destroy된 xterm
// 자리가 빈 화면으로 남는다) + createSession 직접 호출(PTY 생성) +
// officeBus.emitAgentClicked(터미널 오픈/알림 클리어) 순서로 진행한다.
// clockInAll은 퇴근한 에이전트 전부를 순회 출근시킨다.
import { useAppStore } from "../store/appStore";
import { tauriApi } from "../ipc/tauriApi";
import { terminalRegistry } from "../terminal/TerminalRegistry";
import { officeBus } from "../ipc/sessionBridge";
import { sessionOptsFor } from "../ipc/sessionOpts";

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
  // ① 플래그 해제 → 캔버스 재등장(useAgentList 필터 통과) + 세션 런타임 엔트리
  //    재생성(clockIn이 status="starting"으로 되살려 머리 위 현황 UI 복원).
  clockIn(agentId);
  // ② 퇴근 시 terminalRegistry.destroy로 xterm을 폐기했으므로, 에폭을 올려
  //    TerminalMount를 강제 리마운트해야 attach()가 새 xterm을 만든다.
  //    안 그러면 재소환 시 빈 화면(restartAgentSession과 동일한 원리).
  bumpTerminalEpoch(agentId);
  // ③ 세션(PTY)을 직접 생성한다. clockIn이 세션 상태를 "starting"으로 선점해 두어
  //    아래 emitAgentClicked→ensureSession은 needsStart=false로 스킵되므로,
  //    restartAgentSession과 동일하게 createSession을 직접 불러야 터미널 셸이
  //    실제로 뜬다. (예전엔 clockIn이 세션을 안 만들어 ensureSession이 대신
  //    생성했지만, UI 복원용으로 starting을 선점하면서 그 경로가 막혔다 —
  //    PowerShell에서 첫 출근 시 셸이 안 뜨던 원인.) dispose 직후 죽어가는 세션
  //    재사용은 매니저의 create 재사용 가드(kill_requested)가 이미 차단한다.
  const agent = useAppStore.getState().agents[agentId];
  tauriApi.createSession(agentId, sessionOptsFor(agent)).catch((err) => {
    useAppStore.getState().setSessionState({ agentId, status: "exited" });
    console.warn(`clockInAgent: createSession failed for ${agentId}`, err);
  });
  // ④ 터미널 오픈 + 알림 클리어(ensureSession은 이미 starting이라 no-op).
  officeBus.emitAgentClicked(agentId);
}

export function clockInAll(): void {
  // 퇴근한 에이전트 전부 출근. clockInAgent가 store를 바꾸므로 스냅샷을 먼저 뜬다.
  const { agents, agentOrder } = useAppStore.getState();
  const clockedOut = agentOrder.filter((id) => agents[id]?.clockedOut);
  for (const id of clockedOut) {
    clockInAgent(id);
  }
}
