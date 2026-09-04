// Project run recipes (docs/run-recipes-design.md). Rust serde mirrors live in
// src-tauri/src/types.rs.

export type RunRecipeSource = "agent" | "user";
export type RunRecipeAgentState = "missing" | "ready" | "invalid";

/** 팔레트에 그리는 실행 레시피. source는 두 저장 파일을 읽은 뒤 백엔드가 붙인다. */
export interface RunRecipe {
  id: string;
  label: string;
  command: string;
  cwd?: string;
  longRunning?: boolean;
  note?: string;
  createdAt?: string;
  source: RunRecipeSource;
}

/** 앱이 소유하는 *.user.json에 전체 교체로 저장할 항목. */
export interface RunRecipeUserInput {
  id: string;
  label: string;
  command: string;
  createdAt: string;
}

export interface RunRecipesReadResult {
  root: string;
  agentFilePath: string;
  agentState: RunRecipeAgentState;
  agentError?: string;
  agentRecipes: RunRecipe[];
  userRecipes: RunRecipe[];
}

/** 조사 프롬프트에 앱이 계산해 박는 대상. 호출하면서 run-recipes/도 만든다. */
export interface RunRecipeProbeTarget {
  root: string;
  agentFilePath: string;
}
