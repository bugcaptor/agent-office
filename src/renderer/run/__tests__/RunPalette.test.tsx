// @vitest-environment jsdom
import { cleanup, fireEvent, render, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

import { useAppStore } from "../../store/appStore";

const mocks = vi.hoisted(() => ({
  read: vi.fn(),
  save: vi.fn(),
  clear: vi.fn(),
  writeInput: vi.fn(),
  status: vi.fn(),
  stop: vi.fn(),
  execute: vi.fn(),
  probe: vi.fn(),
}));

vi.mock("../../ipc/tauriApi", () => ({
  tauriApi: {
    runRecipesRead: mocks.read,
    runRecipesUserSave: mocks.save,
    runRecipesAgentClear: mocks.clear,
    writeInput: mocks.writeInput,
    runRecipeStatus: mocks.status,
    runRecipeStop: mocks.stop,
  },
}));
vi.mock("../execute", () => ({
  executeRunRecipe: mocks.execute,
  probeRunRecipes: mocks.probe,
}));

const { RunPalette } = await import("../RunPalette");
const { useRunStore } = await import("../runStore");

const initialApp = useAppStore.getState();
const initialRun = useRunStore.getState();
const result = {
  root: "/work/p",
  agentFilePath: "/data/p.agent.json",
  agentState: "ready" as const,
  agentRecipes: [
    { id: "test", label: "Frontend tests", command: "npm test", source: "agent" as const },
  ],
  userRecipes: [
    { id: "dev", label: "Dev server", command: "npm run dev", createdAt: "now", source: "user" as const },
  ],
};

beforeEach(() => {
  useAppStore.setState(initialApp, true);
  useRunStore.setState(initialRun, true);
  useAppStore.setState({
    appSettings: { ...useAppStore.getState().appSettings, runRecipesEnabled: true },
    agents: {
      a1: { id: "a1", name: "A", role: "dev", seed: "a", createdAt: 1, deskIndex: 0, cwd: "/work/p" },
    },
  });
  mocks.read.mockReset().mockResolvedValue(result);
  mocks.save.mockReset().mockResolvedValue(undefined);
  mocks.clear.mockReset().mockResolvedValue(undefined);
  mocks.writeInput.mockReset();
  mocks.status.mockReset().mockResolvedValue(null);
  mocks.stop.mockReset().mockResolvedValue(undefined);
  mocks.execute.mockReset().mockResolvedValue("started");
  mocks.probe.mockReset().mockResolvedValue("injected");
});

afterEach(() => cleanup());

describe("RunPalette", () => {
  it("설정이 꺼져 있으면 열린 store가 있어도 보이지 않고 닫힌다", async () => {
    useRunStore.setState({ palette: { root: "/work/p", agentId: "a1" }, result });
    useAppStore.setState({
      appSettings: { ...useAppStore.getState().appSettings, runRecipesEnabled: false },
    });
    const { container } = render(<RunPalette />);
    expect(container.firstChild).toBeNull();
    await waitFor(() => expect(useRunStore.getState().palette).toBeNull());
  });

  it("조사/직접 레시피를 그리고 행 클릭으로 별도 실행 프로세스를 시작한다", async () => {
    useRunStore.setState({ palette: { root: "/work/p", agentId: "a1" }, result });
    const { getByRole, getByText } = render(<RunPalette />);

    expect(getByText("Frontend tests")).toBeTruthy();
    expect(getByText("Dev server")).toBeTruthy();
    fireEvent.click(getByRole("button", { name: /Frontend tests/ }));

    await waitFor(() => expect(mocks.execute).toHaveBeenCalledWith("a1", "/work/p", result.agentRecipes[0]));
    expect(mocks.writeInput).not.toHaveBeenCalled();
  });

  it("실행 중인 프로세스를 표시하고 전용 중단 IPC로 끝낸다", async () => {
    mocks.status.mockResolvedValue({
      agentId: "a1",
      recipeId: "dev",
      label: "Dev server",
      command: "npm run dev",
      startedAt: 1,
    });
    useRunStore.setState({ palette: { root: "/work/p", agentId: "a1" }, result });
    const { getByRole, getByText } = render(<RunPalette />);

    await waitFor(() => expect(getByText(/Dev server · npm run dev/)).toBeTruthy());
    fireEvent.click(getByRole("button", { name: "중단" }));

    await waitFor(() => expect(mocks.stop).toHaveBeenCalledWith("a1"));
    expect(mocks.writeInput).not.toHaveBeenCalled();
  });

  it("실행 제출 중 연속 클릭을 무시하고 백엔드가 정규화한 root를 쓴다", async () => {
    let finish!: (outcome: string) => void;
    mocks.execute.mockReturnValueOnce(
      new Promise((resolve) => {
        finish = resolve;
      }),
    );
    useRunStore.setState({
      palette: { root: "~/work/p", agentId: "a1" },
      result: { ...result, root: "/Users/me/work/p" },
    });
    const { getByRole } = render(<RunPalette />);
    const runButton = getByRole("button", { name: /Frontend tests/ });

    fireEvent.click(runButton);
    fireEvent.click(runButton);

    expect(mocks.execute).toHaveBeenCalledTimes(1);
    expect(mocks.execute).toHaveBeenCalledWith(
      "a1",
      "/Users/me/work/p",
      result.agentRecipes[0],
    );
    finish("started");
    await waitFor(() =>
      expect((runButton as HTMLButtonElement).disabled).toBe(false),
    );
  });

  it("직접 등록과 조사시키기 버튼을 store/실행기로 연결한다", async () => {
    useRunStore.setState({ palette: { root: "/work/p", agentId: "a1" }, result });
    const { getByPlaceholderText, getByRole } = render(<RunPalette />);
    fireEvent.change(getByPlaceholderText("이름"), { target: { value: "Lint" } });
    fireEvent.change(getByPlaceholderText("셸 명령"), { target: { value: "npm run lint" } });
    fireEvent.click(getByRole("button", { name: "추가" }));

    await waitFor(() => expect(mocks.save).toHaveBeenCalled());
    fireEvent.click(getByRole("button", { name: "실행 방법 조사시키기" }));
    await waitFor(() => expect(mocks.probe).toHaveBeenCalledWith("a1", "/work/p"));
  });
});
