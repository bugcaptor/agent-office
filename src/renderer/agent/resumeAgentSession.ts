// src/renderer/agent/resumeAgentSession.ts
//
// Claude 세션 이어하기 오케스트레이터. restartAgentSession의 변형으로,
// 기존 세션을 dispose/폐기한 뒤 이번 1회 생성에만 startupCommand를
// `claude --resume <sessionId>`로 override해 이전 대화를 이어서 시작한다.
// 설계: docs/claude-session-resume-design.md §4.
//
// 재시작과 순서는 동일하다: disposeSession → registry.destroy →
// bumpTerminalEpoch → setSessionState(starting) → createSession(override).
// 셸 래퍼가 `claude`에 --settings/페르소나를 투명 주입하므로 프로필의 다른
// 옵션(cwd/shell/personalityPrompt)은 그대로 유지된다.
import { useAppStore } from "../store/appStore";
import { tauriApi } from "../ipc/tauriApi";
import { terminalRegistry } from "../terminal/TerminalRegistry";
import { runGuardedCreateSession } from "../ipc/sessionBridge";

/** Claude native 세션 ID 형식(UUID류: 16진수+하이픈). 리줌 ID는 셸 stdin
 * 라인에 그대로 들어가므로 명령 구성 전 형식 검증이 안전장치다. */
const RESUME_SESSION_ID_RE = /^[0-9a-fA-F-]+$/;

/** 검증된 sessionId로 이어하기 시작 명령을 만든다. 형식이 어긋나면 null. */
export function buildResumeStartupCommand(sessionId: string): string | null {
  if (!sessionId || !RESUME_SESSION_ID_RE.test(sessionId)) return null;
  return `claude --resume ${sessionId}`;
}

/**
 * `agentId`의 터미널을 종료하고 Claude native 세션 `sessionId`를 이어서 시작.
 * sessionId 형식이 유효하지 않으면 콘솔 경고만 남기고 아무것도 하지 않는다
 * (기존 세션을 건드리지 않는다).
 */
export async function resumeAgentSession(agentId: string, sessionId: string): Promise<void> {
  const startupCommand = buildResumeStartupCommand(sessionId);
  if (!startupCommand) {
    console.warn(
      `resumeAgentSession: refusing resume for ${agentId} — invalid session id`,
      sessionId,
    );
    return;
  }

  // ① 기존 PTY 종료 — 세션이 없거나 이미 죽었어도 이어하기는 계속.
  try {
    await tauriApi.disposeSession(agentId);
  } catch (err) {
    console.warn(`resumeAgentSession: disposeSession failed for ${agentId}`, err);
  }

  // ② xterm 인스턴스/스크롤백 폐기 (onData 구독 해제 포함).
  terminalRegistry.destroy(agentId);

  // ③ 에폭 증가 → TerminalMount 리마운트 → attach()가 새 xterm 생성/연결.
  useAppStore.getState().bumpTerminalEpoch(agentId);

  // ④ 새 세션 시작. 상태를 먼저 starting으로 만들어 ensureSession과의 경합을
  //    막고, 이번 1회에만 startupCommand override로 `claude --resume`을 주입한다.
  useAppStore.getState().setSessionState({ agentId, status: "starting" });
  await runGuardedCreateSession(agentId, { startupCommand });
}
