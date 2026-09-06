// src/renderer/ipc/sessionOpts.ts
//
// profile/cwd/shell → createSession opts 변환. 세 호출부(ProfileDialog 저장,
// ensureSession, restartAgentSession)가 각자 ad-hoc하게 opts를 조립하던 것을
// 하나로 통일한다.
import type { CreateSessionOptions } from "@shared/types";

/** 프로필 스냅샷과 런타임 옵션을 createSession opts로 변환. 전부 없으면 undefined.
 *
 * override는 이번 1회 생성에만 프로필 값을 대체한다. `startupCommand`는
 * Claude 세션 이어하기에, `cwd`는 현재 작업 폴더에서의 재시작에 쓰인다. */
export function sessionOptsFor(
  a?: {
    name?: string;
    role?: string;
    cwd?: string;
    shell?: string;
    startupCommand?: string;
    personalityPrompt?: string;
    tmuxHost?: boolean;
  },
  overrides?: { startupCommand?: string; cwd?: string },
): CreateSessionOptions | undefined {
  const startupCommand = overrides?.startupCommand || a?.startupCommand;
  if (!a && !startupCommand && !overrides?.cwd) return undefined;
  const o: CreateSessionOptions = {};
  if (a?.name) o.agentName = a.name;
  if (a?.role) o.agentRole = a.role;
  if (overrides?.cwd || a?.cwd) o.cwd = overrides?.cwd || a?.cwd;
  if (a?.shell) o.shell = a.shell;
  if (startupCommand) o.startupCommand = startupCommand;
  if (a?.personalityPrompt) o.personalityPrompt = a.personalityPrompt;
  if (a?.tmuxHost) o.tmuxHost = true;
  return Object.keys(o).length ? o : undefined;
}
