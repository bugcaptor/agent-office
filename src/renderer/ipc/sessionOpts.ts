// src/renderer/ipc/sessionOpts.ts
//
// cwd/shell → createSession opts 변환. 세 호출부(ProfileDialog 저장, ensureSession,
// restartAgentSession)가 각자 ad-hoc하게 opts를 조립하던 것을 하나로 통일한다.
import type { CreateSessionOptions } from "@shared/types";

/** 프로필의 cwd/shell을 createSession opts로 변환. 둘 다 없으면 undefined. */
export function sessionOptsFor(a?: { cwd?: string; shell?: string }): CreateSessionOptions | undefined {
  if (!a) return undefined;
  const o: CreateSessionOptions = {};
  if (a.cwd) o.cwd = a.cwd;
  if (a.shell) o.shell = a.shell;
  return Object.keys(o).length ? o : undefined;
}
