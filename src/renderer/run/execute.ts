import type { RunRecipe } from "@shared/types";

import { runRecipePromptProfile } from "../i18n/promptProfiles";
import { tauriApi } from "../ipc/tauriApi";
import { useAppStore } from "../store/appStore";

export const RUN_INJECT_SUBMIT_DELAY_MS = 150;

export function wslPath(value: string): string {
  const normalized = value.replace(/\\/g, "/");
  const drive = /^([A-Za-z]):\/(.*)$/.exec(normalized);
  return drive ? `/mnt/${drive[1].toLowerCase()}/${drive[2]}` : normalized;
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
  "started" | "alreadyRunning" | "failed";
export type ProbeExecutionResult =
  "injected" | "starting" | "external" | "failed";

/** 레시피를 캐릭터 PTY와 무관한 백엔드 소유 프로세스로 시작한다. */
export async function executeRunRecipe(
  agentId: string,
  root: string,
  recipe: RunRecipe,
): Promise<RunExecutionResult> {
  const state = useAppStore.getState();
  const profile = state.agents[agentId];
  try {
    await tauriApi.runRecipeStart({
      agentId,
      recipeId: recipe.id,
      label: recipe.label,
      command: recipe.command,
      root,
      ...(recipe.cwd ? { cwd: recipe.cwd } : {}),
      ...(profile?.shell ? { shell: profile.shell } : {}),
    });
    return "started";
  } catch (error) {
    if (String(error).includes("run-recipe-already-running")) {
      return "alreadyRunning";
    }
    console.warn("run recipe start failed", error);
    return "failed";
  }
}

/** 조사 대상 파일을 백엔드가 계산한 뒤, 현재 캐릭터 CLI 입력창에 프롬프트를 넣는다. */
export async function probeRunRecipes(
  agentId: string,
  root: string,
): Promise<ProbeExecutionResult> {
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
