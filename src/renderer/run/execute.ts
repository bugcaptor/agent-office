import type { AgentProfile, RunRecipe } from "@shared/types";

import { runRecipePromptProfile } from "../i18n/promptProfiles";
import { tauriApi } from "../ipc/tauriApi";
import { runGuardedCreateSession } from "../ipc/sessionBridge";
import { IS_WINDOWS } from "../shared/platform";
import { useAppStore } from "../store/appStore";

export const RUN_INJECT_SUBMIT_DELAY_MS = 150;

function posixQuote(value: string): string {
  return `'${value.replace(/'/g, `'\"'\"'`)}'`;
}

function powershellQuote(value: string): string {
  return `'${value.replace(/'/g, "''")}'`;
}

function childPath(root: string, cwd: string): string {
  const separator = root.includes("\\") && !root.includes("/") ? "\\" : "/";
  return `${root.replace(/[\\/]+$/, "")}${separator}${cwd.replace(/^[\\/]+/, "")}`;
}

export function wslPath(value: string): string {
  const normalized = value.replace(/\\/g, "/");
  const drive = /^([A-Za-z]):\/(.*)$/.exec(normalized);
  return drive ? `/mnt/${drive[1].toLowerCase()}/${drive[2]}` : normalized;
}

/** 모노레포 하위 cwd 명령을 실행 뒤 원래 위치가 남도록 감싼다. */
export function recipeCommand(
  root: string,
  recipe: Pick<RunRecipe, "command" | "cwd">,
  shell?: string,
  windows = IS_WINDOWS,
): string {
  const cwd = recipe.cwd?.trim();
  if (!cwd) return recipe.command;
  const hostPath = childPath(root, cwd);
  const path = shell === "wsl" ? wslPath(hostPath) : hostPath;
  if (shell === "pwsh" || shell === "powershell" || (!shell && windows)) {
    return `& { Push-Location -LiteralPath ${powershellQuote(path)}; try { ${recipe.command} } finally { Pop-Location } }`;
  }
  return `( cd ${posixQuote(path)} && ${recipe.command} )`;
}

/** writeInput 두 번으로 텍스트와 CR을 분리한다(PowerShell 제출 포함). */
export async function injectAndSubmit(
  agentId: string,
  text: string,
): Promise<void> {
  tauriApi.writeInput(agentId, text);
  await new Promise<void>((resolve) =>
    window.setTimeout(resolve, RUN_INJECT_SUBMIT_DELAY_MS),
  );
  tauriApi.writeInput(agentId, "\r");
}

export type RunExecutionResult =
  "started" | "injected" | "starting" | "external" | "failed";

/** 레시피를 그 캐릭터의 기존 PTY에 넣거나, 종료된 PTY를 명령과 함께 다시 띄운다. */
export async function executeRunRecipe(
  agentId: string,
  root: string,
  recipe: RunRecipe,
): Promise<RunExecutionResult> {
  const state = useAppStore.getState();
  const profile = state.agents[agentId];
  const session = state.sessions[agentId];
  if (session?.kind === "external") return "external";
  const command = recipeCommand(root, recipe, profile?.shell);
  const status = session?.status ?? "idle";
  if (status === "starting") return "starting";
  if (status === "running") {
    await injectAndSubmit(agentId, command);
    return "injected";
  }
  state.setSessionState({ agentId, status: "starting" });
  await runGuardedCreateSession(agentId, { startupCommand: command });
  return useAppStore.getState().sessions[agentId]?.status === "running"
    ? "started"
    : "failed";
}

/** 조사 대상 파일을 백엔드가 계산한 뒤, 현재 캐릭터 CLI 입력창에 프롬프트를 넣는다. */
export async function probeRunRecipes(
  agentId: string,
  root: string,
): Promise<RunExecutionResult> {
  const session = useAppStore.getState().sessions[agentId];
  const profile = useAppStore.getState().agents[agentId];
  if (session?.kind === "external") return "external";
  if (session?.status === "starting") return "starting";
  if (session?.status !== "running") return "failed";
  const target = await tauriApi.runRecipesProbeTarget(root);
  const wsl = profile?.shell === "wsl";
  const prompt = runRecipePromptProfile().formatProbePrompt(
    target.root,
    wsl ? wslPath(target.agentFilePath) : target.agentFilePath,
    wsl ? wslPath(target.root) : target.root,
  );
  await injectAndSubmit(agentId, prompt);
  return "injected";
}

export function activeRecipeText(
  profile: AgentProfile | undefined,
  text: string | undefined,
): string {
  if (!profile || !text) return "";
  return text.split(/\r?\n/, 1)[0]?.trim().slice(0, 120) ?? "";
}
