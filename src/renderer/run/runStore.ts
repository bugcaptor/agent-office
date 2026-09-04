import { create } from "zustand";

import type { RunRecipesReadResult, RunRecipeUserInput } from "@shared/types";

import { tauriApi } from "../ipc/tauriApi";

export interface RunPaletteTarget {
  root: string;
  agentId: string;
}

interface RunState {
  palette: RunPaletteTarget | null;
  result: RunRecipesReadResult | null;
  loading: boolean;
  saving: boolean;
  error: string | null;
  openPalette(root: string, agentId: string): void;
  closePalette(): void;
  refresh(): Promise<void>;
  addUserRecipe(label: string, command: string): Promise<boolean>;
  deleteUserRecipe(id: string): Promise<void>;
  clearAgentRecipes(): Promise<void>;
}

let readGeneration = 0;

function userInputs(result: RunRecipesReadResult | null): RunRecipeUserInput[] {
  return (result?.userRecipes ?? []).map((recipe) => ({
    id: recipe.id,
    label: recipe.label,
    command: recipe.command,
    createdAt: recipe.createdAt ?? new Date().toISOString(),
  }));
}

function manualId(): string {
  return `manual-${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 7)}`;
}

export const useRunStore = create<RunState>()((set, get) => ({
  palette: null,
  result: null,
  loading: false,
  saving: false,
  error: null,

  openPalette(root, agentId) {
    readGeneration += 1;
    set({ palette: { root, agentId }, result: null, error: null });
    void get().refresh();
  },

  closePalette() {
    readGeneration += 1;
    set({
      palette: null,
      result: null,
      loading: false,
      saving: false,
      error: null,
    });
  },

  async refresh() {
    const target = get().palette;
    if (!target) return;
    const generation = ++readGeneration;
    set({ loading: true, error: null });
    try {
      const result = await tauriApi.runRecipesRead(target.root);
      const current = get().palette;
      if (
        generation !== readGeneration ||
        !current ||
        current.root !== target.root ||
        current.agentId !== target.agentId
      ) {
        return;
      }
      set({ result, loading: false });
    } catch (error) {
      if (generation !== readGeneration) return;
      console.warn("runStore: recipe read failed", error);
      set({ loading: false, error: String(error) });
    }
  },

  async addUserRecipe(label, command) {
    const target = get().palette;
    const result = get().result;
    if (!target || !result || !label.trim() || !command.trim()) return false;
    const recipes = userInputs(result);
    recipes.push({
      id: manualId(),
      label: label.trim(),
      command: command.trim(),
      createdAt: new Date().toISOString(),
    });
    set({ saving: true, error: null });
    try {
      await tauriApi.runRecipesUserSave(target.root, recipes);
      await get().refresh();
      return true;
    } catch (error) {
      console.warn("runStore: manual recipe save failed", error);
      set({ error: String(error) });
      return false;
    } finally {
      set({ saving: false });
    }
  },

  async deleteUserRecipe(id) {
    const target = get().palette;
    const result = get().result;
    if (!target || !result) return;
    const recipes = userInputs(result).filter(
      (recipe) => recipe.id !== id,
    );
    set({ saving: true, error: null });
    try {
      await tauriApi.runRecipesUserSave(target.root, recipes);
      await get().refresh();
    } catch (error) {
      console.warn("runStore: manual recipe delete failed", error);
      set({ error: String(error) });
    } finally {
      set({ saving: false });
    }
  },

  async clearAgentRecipes() {
    const target = get().palette;
    if (!target) return;
    set({ saving: true, error: null });
    try {
      await tauriApi.runRecipesAgentClear(target.root);
      await get().refresh();
    } catch (error) {
      console.warn("runStore: agent recipe clear failed", error);
      set({ error: String(error) });
    } finally {
      set({ saving: false });
    }
  },
}));
