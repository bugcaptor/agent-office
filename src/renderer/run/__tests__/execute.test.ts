// @vitest-environment jsdom
import { beforeEach, describe, expect, it, vi } from "vitest";

const mocks = vi.hoisted(() => ({
  writeInput: vi.fn(),
  probeTarget: vi.fn(),
  start: vi.fn(),
  getState: vi.fn(),
}));

vi.mock("../../ipc/tauriApi", () => ({
  tauriApi: {
    writeInput: mocks.writeInput,
    runRecipesProbeTarget: mocks.probeTarget,
    runRecipeStart: mocks.start,
  },
}));
vi.mock("../../store/appStore", () => ({ useAppStore: { getState: mocks.getState } }));

const {
  RUN_INJECT_SUBMIT_DELAY_MS,
  executeRunRecipe,
  injectAndSubmit,
  probeRunRecipes,
  wslPath,
} = await import("../execute");

const recipe = { id: "test", label: "Test", command: "npm test", source: "agent" as const };

beforeEach(() => {
  vi.useRealTimers();
  mocks.writeInput.mockReset();
  mocks.probeTarget.mockReset();
  mocks.start.mockReset().mockResolvedValue({
    agentId: "a1",
    recipeId: "test",
    label: "Test",
    command: "npm test",
    startedAt: 1,
  });
  mocks.getState.mockReset();
});

it("WSL 경로는 조사 프롬프트에서 Linux 접근 경로로 바꾼다", () => {
  expect(wslPath("C:\\Users\\me\\App Data\\recipes.json")).toBe(
    "/mnt/c/Users/me/App Data/recipes.json",
  );
});

it("명령 원문과 CR을 150ms 간격으로 나눠 넣는다", async () => {
  vi.useFakeTimers();
  const pending = injectAndSubmit("a1", "printf 'a  b'");
  expect(mocks.writeInput).toHaveBeenCalledWith("a1", "printf 'a  b'");
  expect(mocks.writeInput).toHaveBeenCalledTimes(1);
  await vi.advanceTimersByTimeAsync(RUN_INJECT_SUBMIT_DELAY_MS);
  await pending;
  expect(mocks.writeInput).toHaveBeenLastCalledWith("a1", "\r");
});

describe("executeRunRecipe", () => {
  it("에이전트 PTY 상태와 무관하게 전용 프로세스 시작 IPC만 호출한다", async () => {
    const state = {
      agents: { a1: { shell: "pwsh" } },
      sessions: { a1: { status: "running", kind: "pty" } },
    };
    mocks.getState.mockReturnValue(state);
    await expect(
      executeRunRecipe("a1", "/work/p", { ...recipe, cwd: "web" }),
    ).resolves.toBe("started");
    expect(mocks.start).toHaveBeenCalledWith({
      agentId: "a1",
      recipeId: "test",
      label: "Test",
      command: "npm test",
      root: "/work/p",
      cwd: "web",
      shell: "pwsh",
    });
    expect(mocks.writeInput).not.toHaveBeenCalled();
  });

  it("세션과 프로필이 없어도 기본 셸로 실행한다", async () => {
    mocks.getState.mockReturnValue({ agents: {}, sessions: {} });
    await expect(executeRunRecipe("a1", "/work/p", recipe)).resolves.toBe("started");
    expect(mocks.start).toHaveBeenCalledWith({
      agentId: "a1",
      recipeId: "test",
      label: "Test",
      command: "npm test",
      root: "/work/p",
    });
  });

  it("이미 실행 중인 캐릭터에는 중복 실행 결과를 돌려준다", async () => {
    mocks.getState.mockReturnValue({ agents: { a1: {} }, sessions: {} });
    mocks.start.mockRejectedValue("run-recipe-already-running");
    await expect(executeRunRecipe("a1", "/work/p", recipe)).resolves.toBe(
      "alreadyRunning",
    );
  });
});

it("조사 대상의 절대 경로가 든 프롬프트를 running PTY에 넣는다", async () => {
  vi.useFakeTimers();
  mocks.getState.mockReturnValue({
    agents: { a1: {} },
    sessions: { a1: { status: "running", kind: "pty" } },
  });
  mocks.probeTarget.mockResolvedValue({
    root: "/work/p",
    agentFilePath: "/app/run-recipes/p.agent.json",
  });
  const pending = probeRunRecipes("a1", "/work/p");
  await vi.advanceTimersByTimeAsync(RUN_INJECT_SUBMIT_DELAY_MS);
  await expect(pending).resolves.toBe("injected");
  expect(mocks.writeInput.mock.calls[0][1]).toContain("/app/run-recipes/p.agent.json");
  expect(mocks.writeInput.mock.calls[1]).toEqual(["a1", "\r"]);
});

it("WSL 조사에는 Linux 접근 경로와 JSON 검증용 host root를 함께 넣는다", async () => {
  vi.useFakeTimers();
  mocks.getState.mockReturnValue({
    agents: { a1: { shell: "wsl" } },
    sessions: { a1: { status: "running", kind: "pty" } },
  });
  mocks.probeTarget.mockResolvedValue({
    root: "c:/work/project",
    agentFilePath: "C:\\Users\\me\\App Data\\p.agent.json",
  });

  const pending = probeRunRecipes("a1", "C:/work/project");
  await vi.advanceTimersByTimeAsync(RUN_INJECT_SUBMIT_DELAY_MS);
  await pending;

  const prompt = mocks.writeInput.mock.calls[0][1];
  expect(prompt).toContain("/mnt/c/work/project");
  expect(prompt).toContain("/mnt/c/Users/me/App Data/p.agent.json");
  expect(prompt).toContain('"root":"c:/work/project"');
});
