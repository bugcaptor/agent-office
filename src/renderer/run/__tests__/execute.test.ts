// @vitest-environment jsdom
import { beforeEach, describe, expect, it, vi } from "vitest";

const mocks = vi.hoisted(() => ({
  writeInput: vi.fn(),
  probeTarget: vi.fn(),
  guardedCreate: vi.fn(),
  getState: vi.fn(),
}));

vi.mock("../../ipc/tauriApi", () => ({
  tauriApi: {
    writeInput: mocks.writeInput,
    runRecipesProbeTarget: mocks.probeTarget,
  },
}));
vi.mock("../../ipc/sessionBridge", () => ({
  runGuardedCreateSession: mocks.guardedCreate,
}));
vi.mock("../../store/appStore", () => ({ useAppStore: { getState: mocks.getState } }));

const {
  RUN_INJECT_SUBMIT_DELAY_MS,
  executeRunRecipe,
  injectAndSubmit,
  probeRunRecipes,
  recipeCommand,
  wslPath,
} = await import("../execute");

const recipe = { id: "test", label: "Test", command: "npm test", source: "agent" as const };

beforeEach(() => {
  vi.useRealTimers();
  mocks.writeInput.mockReset();
  mocks.probeTarget.mockReset();
  mocks.guardedCreate.mockReset();
  mocks.getState.mockReset();
});

describe("recipeCommand", () => {
  it("cwd가 없으면 원문 명령을 그대로 둔다", () => {
    expect(recipeCommand("/work/p", recipe)).toBe("npm test");
  });

  it("POSIX 셸은 서브셸 cd로 감싸고 작은따옴표를 이스케이프한다", () => {
    expect(recipeCommand("/work/a'b", { ...recipe, cwd: "web" }, "zsh")).toBe(
      `( cd '/work/a'\"'\"'b/web' && npm test )`,
    );
  });

  it("PowerShell은 Push/Pop-Location으로 원래 위치를 복구한다", () => {
    expect(recipeCommand("C:\\Work", { ...recipe, cwd: "web" }, "pwsh")).toBe(
      "& { Push-Location -LiteralPath 'C:\\Work\\web'; try { npm test } finally { Pop-Location } }",
    );
  });

  it("Windows의 자동 셸도 PowerShell 래퍼를 쓴다", () => {
    expect(recipeCommand("C:\\Work", { ...recipe, cwd: "web" }, undefined, true)).toBe(
      "& { Push-Location -LiteralPath 'C:\\Work\\web'; try { npm test } finally { Pop-Location } }",
    );
  });

  it("WSL은 Windows 호스트 경로를 /mnt 경로로 바꾼다", () => {
    expect(recipeCommand("C:/Work", { ...recipe, cwd: "web app" }, "wsl")).toBe(
      "( cd '/mnt/c/Work/web app' && npm test )",
    );
    expect(wslPath("C:\\Users\\me\\App Data\\recipes.json")).toBe(
      "/mnt/c/Users/me/App Data/recipes.json",
    );
  });
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
  it("running PTY에는 명령을 두 번 나눠 넣는다", async () => {
    vi.useFakeTimers();
    const state = {
      agents: { a1: { shell: "zsh" } },
      sessions: { a1: { status: "running", kind: "pty" } },
      setSessionState: vi.fn(),
    };
    mocks.getState.mockReturnValue(state);
    const pending = executeRunRecipe("a1", "/work/p", recipe);
    await vi.advanceTimersByTimeAsync(RUN_INJECT_SUBMIT_DELAY_MS);
    await expect(pending).resolves.toBe("injected");
    expect(mocks.writeInput.mock.calls).toEqual([["a1", "npm test"], ["a1", "\r"]]);
  });

  it("idle PTY는 starting으로 바꾸고 startupCommand로 다시 띄운다", async () => {
    const state = {
      agents: { a1: { shell: "zsh" } },
      sessions: { a1: { status: "idle", kind: "pty" } },
      setSessionState: vi.fn(({ status }: { status: string }) => {
        state.sessions.a1.status = status;
      }),
    };
    mocks.getState.mockImplementation(() => state);
    mocks.guardedCreate.mockImplementation(async () => {
      state.sessions.a1.status = "running";
    });
    await expect(executeRunRecipe("a1", "/work/p", recipe)).resolves.toBe("started");
    expect(state.setSessionState).toHaveBeenCalledWith({ agentId: "a1", status: "starting" });
    expect(mocks.guardedCreate).toHaveBeenCalledWith("a1", { startupCommand: "npm test" });
  });

  it("starting과 external 세션에는 입력을 섞지 않는다", async () => {
    const state = {
      agents: { a1: {} },
      sessions: { a1: { status: "starting", kind: "pty" } },
      setSessionState: vi.fn(),
    };
    mocks.getState.mockReturnValue(state);
    await expect(executeRunRecipe("a1", "/work/p", recipe)).resolves.toBe("starting");
    state.sessions.a1 = { status: "running", kind: "external" };
    await expect(executeRunRecipe("a1", "/work/p", recipe)).resolves.toBe("external");
    expect(mocks.writeInput).not.toHaveBeenCalled();
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
