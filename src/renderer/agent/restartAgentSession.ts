// src/renderer/agent/restartAgentSession.ts
//
// 터미널 재시작 오케스트레이터 (deleteAgent.ts의 오케스트레이션 패턴 참고).
// 순서를 보장한다:
//   ① tauriApi.disposeSession — 기존 PTY 종료. 세션이 없거나 이미 죽었어도
//      재시작 자체는 계속 진행한다.
//   ② terminalRegistry.destroy — xterm 인스턴스/스크롤백 폐기(onData 구독
//      해제 포함). 재시작이므로 keep-alive 원칙(평소엔 destroy 없이
//      display:none 토글로만 유지)의 예외.
//   ③ bumpTerminalEpoch — TerminalHost의 TerminalMount key를 바꿔 강제
//      리마운트시킨다. 리마운트된 TerminalMount의 attach 이펙트가 새 xterm을
//      생성/연결한다(TerminalHost.tsx 참고).
//   ④ 새 세션 시작. sessionBridge.ensureSession과 동일하게, createSession
//      호출 전에 상태를 먼저 "starting"으로 만들어 두어야 (예: 오피스에서의
//      클릭 등으로 트리거되는) ensureSession이 이 재시작과 경합해 중복
//      createSession을 만들지 않는다.
import { useAppStore } from "../store/appStore";
import { tauriApi } from "../ipc/tauriApi";
import { terminalRegistry } from "../terminal/TerminalRegistry";
import { sessionOptsFor } from "../ipc/sessionOpts";

export async function restartAgentSession(agentId: string): Promise<void> {
  // ① 기존 PTY 종료 — 세션이 없거나 이미 죽었어도 재시작은 계속.
  try {
    await tauriApi.disposeSession(agentId);
  } catch (err) {
    console.warn(`restartAgentSession: disposeSession failed for ${agentId}`, err);
  }

  // ② xterm 인스턴스/스크롤백 폐기 (onData 구독 해제 포함).
  terminalRegistry.destroy(agentId);

  // ③ 에폭 증가 → TerminalMount 리마운트 → attach()가 새 xterm 생성/연결.
  //    새 onData 구독이 createSession 완료보다 늦어도 백엔드 백로그가
  //    초기 출력을 버퍼링하므로 유실 없음 (tauriApi.onData 참고).
  useAppStore.getState().bumpTerminalEpoch(agentId);

  // ④ 새 세션 시작. 상태를 먼저 starting으로 만들어 두면 sessionBridge의
  //    ensureSession이 중복 createSession을 만들지 않는다.
  const { agents, setSessionState } = useAppStore.getState();
  setSessionState({ agentId, status: "starting" });
  try {
    await tauriApi.createSession(agentId, sessionOptsFor(agents[agentId]));
  } catch (err) {
    useAppStore.getState().setSessionState({ agentId, status: "exited" });
    console.warn(`restartAgentSession: createSession failed for ${agentId}`, err);
  }
}
