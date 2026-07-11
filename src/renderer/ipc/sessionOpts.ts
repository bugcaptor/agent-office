// src/renderer/ipc/sessionOpts.ts
//
// profile/cwd/shell → createSession opts 변환. 세 호출부(ProfileDialog 저장,
// ensureSession, restartAgentSession)가 각자 ad-hoc하게 opts를 조립하던 것을
// 하나로 통일한다.
import type { CreateSessionOptions } from "@shared/types";

/** 프로필 스냅샷과 런타임 옵션을 createSession opts로 변환. 전부 없으면 undefined. */
export function sessionOptsFor(
  a?: { name?: string; role?: string; cwd?: string; shell?: string; startupCommand?: string },
): CreateSessionOptions | undefined {
  if (!a) return undefined;
  const o: CreateSessionOptions = {};
  if (a.name) o.agentName = a.name;
  if (a.role) o.agentRole = a.role;
  if (a.cwd) o.cwd = a.cwd;
  if (a.shell) o.shell = a.shell;
  if (a.startupCommand) o.startupCommand = a.startupCommand;
  return Object.keys(o).length ? o : undefined;
}
