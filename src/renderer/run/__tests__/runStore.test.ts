import { beforeEach, describe, expect, it, vi } from "vitest";

const mocks = vi.hoisted(() => ({
  read: vi.fn(),
  save: vi.fn(),
  clear: vi.fn(),
}));

vi.mock("../../ipc/tauriApi", () => ({
  tauriApi: {
    runRecipesRead: mocks.read,
    runRecipesUserSave: mocks.save,
    runRecipesAgentClear: mocks.clear,
  },
}));

const { useRunStore } = await import("../runStore");

const emptyResult = {
  root: "/work/a",
  agentFilePath: "/data/a.agent.json",
  agentState: "missing" as const,
  agentRecipes: [],
  userRecipes: [],
};

beforeEach(() => {
  useRunStore.setState({ palette: null, result: null, loading: false, saving: false, error: null });
  mocks.read.mockReset().mockResolvedValue(emptyResult);
  mocks.save.mockReset().mockResolvedValue(undefined);
  mocks.clear.mockReset().mockResolvedValue(undefined);
});

describe("runStore", () => {
  it("팔레트를 열면 cwd를 읽고 닫은 뒤 도착한 응답은 버린다", async () => {
    let resolveFirst!: (value: typeof emptyResult) => void;
    mocks.read.mockReturnValueOnce(new Promise((resolve) => { resolveFirst = resolve; }));
    useRunStore.getState().openPalette("/work/a", "a1");
    useRunStore.getState().closePalette();
    resolveFirst(emptyResult);
    await Promise.resolve();
    expect(useRunStore.getState().palette).toBeNull();
    expect(useRunStore.getState().result).toBeNull();
  });

  it("직접 등록은 기존 createdAt을 보존해 전체 교체 저장한다", async () => {
    const current = {
      ...emptyResult,
      userRecipes: [
        { id: "old", label: "Old", command: "old", createdAt: "2026-01-01", source: "user" as const },
      ],
    };
    mocks.read.mockResolvedValue(current);
    useRunStore.setState({ palette: { root: "/work/a", agentId: "a1" }, result: current });
    await useRunStore.getState().addUserRecipe(" Test ", " npm test ");

    expect(mocks.save).toHaveBeenCalledOnce();
    const [root, recipes] = mocks.save.mock.calls[0];
    expect(root).toBe("/work/a");
    expect(recipes[0]).toEqual({ id: "old", label: "Old", command: "old", createdAt: "2026-01-01" });
    expect(recipes[1]).toMatchObject({ label: "Test", command: "npm test" });
  });

  it("직접 등록 삭제와 조사 결과 비우기는 서로의 파일만 건드린다", async () => {
    const current = {
      ...emptyResult,
      agentState: "ready" as const,
      userRecipes: [
        { id: "keep", label: "Keep", command: "keep", createdAt: "now", source: "user" as const },
        { id: "drop", label: "Drop", command: "drop", createdAt: "then", source: "user" as const },
      ],
    };
    mocks.read.mockResolvedValue(current);
    useRunStore.setState({ palette: { root: "/work/a", agentId: "a1" }, result: current });

    await useRunStore.getState().deleteUserRecipe("drop");
    expect(mocks.save).toHaveBeenCalledWith("/work/a", [
      { id: "keep", label: "Keep", command: "keep", createdAt: "now" },
    ]);
    await useRunStore.getState().clearAgentRecipes();
    expect(mocks.clear).toHaveBeenCalledWith("/work/a");
  });

  it("읽기 결과가 없으면 손 등록 파일을 빈 목록 기준으로 덮어쓰지 않는다", async () => {
    useRunStore.setState({
      palette: { root: "/work/a", agentId: "a1" },
      result: null,
    });

    await useRunStore.getState().addUserRecipe("Test", "npm test");

    expect(mocks.save).not.toHaveBeenCalled();
  });
});
