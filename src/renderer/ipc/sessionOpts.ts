// src/renderer/ipc/sessionOpts.ts
//
// profile/cwd/shell → createSession opts 변환. 세 호출부(ProfileDialog 저장,
// ensureSession, restartAgentSession)가 각자 ad-hoc하게 opts를 조립하던 것을
// 하나로 통일한다.
import type { CreateSessionOptions } from "@shared/types";

/** 프로필 스냅샷과 런타임 옵션을 createSession opts로 변환. 전부 없으면 undefined.
 *
 * `overrides.startupCommand`는 이번 1회 생성에만 프로필의 startupCommand를
 * 대체한다(Claude 세션 이어하기가 `claude --resume <id>`를 주입하는 경로).
 * 나머지 필드(cwd/shell/페르소나 등)는 프로필 그대로 유지된다. */
export function sessionOptsFor(
  a?: {
    name?: string;
    role?: string;
    cwd?: string;
    shell?: string;
    startupCommand?: string;
    personalityPrompt?: string;
  },
  overrides?: { startupCommand?: string },
): CreateSessionOptions | undefined {
  const startupCommand = overrides?.startupCommand || a?.startupCommand;
  if (!a && !startupCommand) return undefined;
  const o: CreateSessionOptions = {};
  if (a?.name) o.agentName = a.name;
  if (a?.role) o.agentRole = a.role;
  if (a?.cwd) o.cwd = a.cwd;
  if (a?.shell) o.shell = a.shell;
  if (startupCommand) o.startupCommand = startupCommand;
  if (a?.personalityPrompt) o.personalityPrompt = a.personalityPrompt;
  return Object.keys(o).length ? o : undefined;
}
