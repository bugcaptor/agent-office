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
import { runGuardedCreateSession } from "../ipc/sessionBridge";
import { sharedDiaryFlusher } from "../diary/diaryFlusher";

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
  //    ensureSession이 중복 createSession을 만들지 않는다. 생성 자체는
  //    공통 가드(runGuardedCreateSession)가 담당 — 결과 상태 반영 +
  //    실패/타임아웃 시 exited 복구(2026-07-11 터미널 영구 고착 방지).
  useAppStore.getState().setSessionState({ agentId, status: "starting" });

  // ④' 옛 세션의 일기 catch-up을 명시 트리거(#75). 재시작에서는 create가 먼저
  //     새 세션을 맵에 넣어 옛 세션이 superseded 되므로, 백엔드의 disposed 상태
  //     이벤트가 is_current 가드로 **억제**된다(session/manager). 그러면
  //     diaryAutoWriter의 종료 구독이 이 세션에 대해 발화하지 않아, 앱이 계속
  //     활성인 재시작 연발 구간에서 옛 세션이 flush 트리거를 영영 못 받고
  //     작업 로그에만 쌓이다 유실될 수 있다. 여기서 직접 flush를 태워 막는다.
  //     지금 status가 "starting"(방금 설정)이라 flusher의 live-세션 제외가
  //     옛 세션을 걸러내지 않는다(handle이 다음 마이크로태스크에 status를
  //     읽는데, createSession의 running 반영은 async invoke 뒤라 아직 안 옴).
  //     실패/타임아웃해도 세션 인지 작업 로그 보존 + 유휴 스윕이 백스톱.
  //     fire-and-forget — 재시작을 블록하지 않는다.
  void sharedDiaryFlusher().flushAgent(agentId, { includeLive: false, source: "session-end" });

  await runGuardedCreateSession(agentId);
}
