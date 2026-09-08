// @vitest-environment jsdom
import {
  cleanup,
  fireEvent,
  render,
  screen,
  waitFor,
} from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { initI18nForTest } from "../i18n";
import { useAppStore } from "../store/appStore";

const api = vi.hoisted(() => ({
  automationDefinitionsList: vi.fn().mockResolvedValue([]),
  automationRunsList: vi.fn().mockResolvedValue([]),
  automationDefinitionsSave: vi.fn(),
  automationDefinitionsImport: vi.fn(),
  automationDefinitionsDelete: vi.fn(),
  automationDefinitionsExport: vi.fn(),
  automationRunStart: vi.fn(),
  automationCliTransitionPreview: vi
    .fn()
    .mockResolvedValue({ autoReturnSupported: true, launchCommand: "claude" }),
  automationCliModels: vi.fn().mockResolvedValue([]),
}));
vi.mock("../ipc/tauriApi", () => ({ tauriApi: api }));
import { AutomationPalette } from "./AutomationPalette";

describe("AutomationPalette", () => {
  afterEach(cleanup);
  beforeEach(async () => {
    await initI18nForTest("en");
    vi.clearAllMocks();
    api.automationDefinitionsList.mockResolvedValue([]);
    api.automationRunsList.mockResolvedValue([]);
    api.automationCliTransitionPreview.mockResolvedValue({
      autoReturnSupported: true,
      launchCommand: "claude",
    });
    api.automationCliModels.mockResolvedValue([]);
    useAppStore.setState({
      automationEditor: { agentId: "agent", workspace: "/work" },
      automation: {},
      automationEpochs: {},
      botMode: {},
      sessions: { agent: { status: "running" } },
    } as never);
  });

  it("opens the editor without starting automation and blocks a dirty sample", async () => {
    render(<AutomationPalette />);
    await screen.findByRole("dialog");
    expect(api.automationRunStart).not.toHaveBeenCalled();
    fireEvent.click(screen.getByRole("button", { name: "Single task" }));
    const run = screen.getByRole("button", { name: "Run" });
    expect((run as HTMLButtonElement).disabled).toBe(true);
    fireEvent.click(run);
    expect(api.automationRunStart).not.toHaveBeenCalled();
  });

  it("seeds the returned status after an explicit saved run", async () => {
    const definition = {
      schemaVersion: 1,
      id: "d",
      revision: 1,
      name: "Saved",
      inputs: [{ key: "task", label: "Task", default: "default task" }],
      steps: [
        {
          id: "s",
          kind: "llmTask",
          label: "Task",
          promptTemplate: "{{task}}",
          waitTimeoutMs: 1,
        },
      ],
    } as const;
    api.automationDefinitionsSave.mockResolvedValue(definition);
    api.automationDefinitionsList.mockResolvedValue([definition]);
    api.automationRunStart.mockResolvedValue({
      running: true,
      phase: "watching",
      cli: "claude",
      filePath: "",
      startedAtMs: 1,
    });
    render(<AutomationPalette />);
    fireEvent.click(await screen.findByRole("button", { name: "Saved" }));
    fireEvent.click(screen.getAllByRole("button", { name: "Run" })[0]);
    await waitFor(() =>
      expect(api.automationRunStart).toHaveBeenCalledWith(
        "agent",
        "d",
        { task: "default task" },
        "/work",
      ),
    );
    expect(useAppStore.getState().automation.agent.running).toBe(true);
  });

  it("saves agy as the selected CLI profile", async () => {
    const definition = {
      schemaVersion: 1,
      id: "d",
      revision: 1,
      name: "Saved",
      inputs: [],
      steps: [
        { id: "launch", kind: "launchCli", cliProfileId: "claude" },
        { id: "wait", kind: "wait", durationMs: 1 },
      ],
    } as const;
    api.automationDefinitionsList.mockResolvedValue([definition]);
    api.automationDefinitionsSave.mockResolvedValue({
      ...definition,
      revision: 2,
      steps: [
        { id: "launch", kind: "launchCli", cliProfileId: "agy" },
        { id: "wait", kind: "wait", durationMs: 1 },
      ],
    });
    render(<AutomationPalette />);
    api.automationCliModels.mockResolvedValue(["gemini-3.8-flash-medium"]);
    fireEvent.click(await screen.findByRole("button", { name: "Saved" }));
    fireEvent.change(screen.getByLabelText("CLI"), {
      target: { value: "agy" },
    });
    await screen.findByText(
      "Choose a model reported by the installed agy CLI, or enter one manually.",
    );
    expect(api.automationCliModels).toHaveBeenCalledWith("agy");
    fireEvent.change(screen.getByLabelText("Select agy model"), {
      target: { value: "gemini-3.8-flash-medium" },
    });
    expect(
      (screen.getByDisplayValue("gemini-3.8-flash-medium") as HTMLInputElement)
        .value,
    ).toBe("gemini-3.8-flash-medium");
    expect(
      screen.getByText(/agy --dangerously-skip-permissions --model 'gemini-3\.8-flash-medium'/),
    ).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "Save" }));
    await waitFor(() =>
      expect(api.automationDefinitionsSave).toHaveBeenCalledWith(
        expect.objectContaining({
          steps: expect.arrayContaining([
            expect.objectContaining({ cliProfileId: "agy" }),
          ]),
        }),
      ),
    );
  });

  it("translates an invalid saved agy effort", async () => {
    const definition = {
      schemaVersion: 1,
      id: "d",
      revision: 1,
      name: "Saved",
      inputs: [],
      steps: [
        {
          id: "launch",
          kind: "launchCli",
          cliProfileId: "agy",
          effort: "high",
        },
      ],
    } as const;
    api.automationDefinitionsList.mockResolvedValue([definition]);
    api.automationDefinitionsSave.mockRejectedValue(
      new Error("automation-agy-effort-invalid"),
    );
    render(<AutomationPalette />);
    fireEvent.click(await screen.findByRole("button", { name: "Saved" }));
    fireEvent.change(screen.getByLabelText("Effort"), {
      target: { value: "xhigh" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Save" }));
    expect(
      await screen.findByText(
        "Save failed: agy effort must be low, medium, or high.",
      ),
    ).toBeTruthy();
  });

  it("selects models reported by Claude and Codex and clears incompatible choices on CLI switch", async () => {
    api.automationCliModels.mockImplementation(async (cli: string) =>
      cli === "claude" ? ["opus", "haiku"] : ["gpt-6-astra", "gpt-5.6-sol"],
    );
    render(<AutomationPalette />);
    await screen.findByText("Recent runs");
    fireEvent.click(screen.getByRole("button", { name: "Single task" }));
    await screen.findByRole("option", { name: "opus" });
    fireEvent.change(screen.getByLabelText("Select claude model"), {
      target: { value: "opus" },
    });
    expect((screen.getByLabelText("Model") as HTMLInputElement).value).toBe(
      "opus",
    );
    fireEvent.change(screen.getByLabelText("Effort"), {
      target: { value: "high" },
    });
    fireEvent.change(screen.getByLabelText("CLI"), {
      target: { value: "codex" },
    });
    expect((screen.getByLabelText("Model") as HTMLInputElement).value).toBe("");
    expect((screen.getByLabelText("Effort") as HTMLInputElement).value).toBe(
      "",
    );
    await screen.findByRole("option", { name: "gpt-6-astra" });
    fireEvent.change(screen.getByLabelText("Select codex model"), {
      target: { value: "gpt-6-astra" },
    });
    expect(screen.getByText(/codex --model 'gpt-6-astra'/)).toBeTruthy();
    expect(api.automationRunStart).not.toHaveBeenCalled();
  });

  it("selects Kilo Code, loads its model catalog, and previews its command", async () => {
    api.automationCliModels.mockImplementation(async (cli: string) =>
      cli === "kilo" ? ["kilo/sonnet-4.5"] : [],
    );
    render(<AutomationPalette />);
    await screen.findByText("Recent runs");
    fireEvent.click(screen.getByRole("button", { name: "Single task" }));
    fireEvent.change(screen.getByLabelText("CLI"), {
      target: { value: "kilo" },
    });
    expect(screen.queryByLabelText("Effort")).toBeNull();
    await screen.findByRole("option", { name: "kilo/sonnet-4.5" });
    expect(api.automationCliModels).toHaveBeenCalledWith("kilo");
    fireEvent.change(screen.getByLabelText("Select kilo model"), {
      target: { value: "kilo/sonnet-4.5" },
    });
    expect(screen.getByText(/kilo --auto --model 'kilo\/sonnet-4\.5'/)).toBeTruthy();
  });

  it("selects Pi, loads its model catalog, and previews model and thinking", async () => {
    api.automationCliModels.mockImplementation(async (cli: string) =>
      cli === "pi" ? ["openai/gpt-5.6-sol"] : [],
    );
    render(<AutomationPalette />);
    await screen.findByText("Recent runs");
    fireEvent.click(screen.getByRole("button", { name: "Single task" }));
    fireEvent.change(screen.getByLabelText("CLI"), {
      target: { value: "pi" },
    });
    await screen.findByRole("option", { name: "openai/gpt-5.6-sol" });
    expect(api.automationCliModels).toHaveBeenCalledWith("pi");
    fireEvent.change(screen.getByLabelText("Select pi model"), {
      target: { value: "openai/gpt-5.6-sol" },
    });
    fireEvent.change(screen.getByLabelText("Effort"), {
      target: { value: "high" },
    });
    expect(
      screen.getByText(/pi --approve --model 'openai\/gpt-5\.6-sol' --thinking 'high'/),
    ).toBeTruthy();
  });

  it("uses Pi's default exit command and preserves a customized exit command", async () => {
    const definition = {
      schemaVersion: 2,
      id: "pi-exit",
      revision: 1,
      name: "Pi exit",
      inputs: [],
      steps: [
        { id: "launch", kind: "launchCli", cliProfileId: "claude" },
        { id: "exit", kind: "exitCli", command: "/exit" },
      ],
    } as const;
    api.automationDefinitionsList.mockResolvedValue([definition]);
    render(<AutomationPalette />);
    fireEvent.click(await screen.findByRole("button", { name: "Pi exit" }));
    fireEvent.change(screen.getByLabelText("CLI"), { target: { value: "pi" } });
    const command = screen.getByLabelText("Command") as HTMLInputElement;
    expect(command.value).toBe("/quit");
    fireEvent.change(command, { target: { value: "exit --clean" } });
    fireEvent.change(screen.getByLabelText("CLI"), {
      target: { value: "claude" },
    });
    expect(command.value).toBe("exit --clean");
  });

  it("defaults a new exit step to Pi's quit command after a Pi launch", async () => {
    render(<AutomationPalette />);
    await screen.findByText("Recent runs");
    fireEvent.click(screen.getByRole("button", { name: "Single task" }));
    fireEvent.change(screen.getByLabelText("CLI"), { target: { value: "pi" } });
    fireEvent.click(screen.getByRole("button", { name: "Exit CLI" }));
    expect((screen.getByLabelText("Command") as HTMLInputElement).value).toBe(
      "/quit",
    );
  });

  it("keeps manual models on catalog failure and allows retry without stale CLI results", async () => {
    let resolveClaude!: (models: string[]) => void;
    api.automationCliModels.mockImplementation((cli: string) =>
      cli === "claude"
        ? new Promise<string[]>((resolve) => {
            resolveClaude = resolve;
          })
        : Promise.reject(new Error("offline")),
    );
    render(<AutomationPalette />);
    await screen.findByText("Recent runs");
    fireEvent.click(screen.getByRole("button", { name: "Single task" }));
    await waitFor(() =>
      expect(api.automationCliModels).toHaveBeenCalledWith("claude"),
    );
    fireEvent.change(screen.getByLabelText("CLI"), {
      target: { value: "codex" },
    });
    fireEvent.change(screen.getByLabelText("Model"), {
      target: { value: "custom-model" },
    });
    await screen.findByText(/Could not load codex models/);
    resolveClaude(["stale-claude-model"]);
    expect((screen.getByLabelText("Model") as HTMLInputElement).value).toBe(
      "custom-model",
    );
    expect(
      screen.queryByRole("option", { name: "stale-claude-model" }),
    ).toBeNull();
    api.automationCliModels.mockResolvedValue(["gpt-5.6-sol"]);
    fireEvent.click(screen.getByRole("button", { name: "Reload model list" }));
    await screen.findByRole("option", { name: "gpt-5.6-sol" });
    expect((screen.getByLabelText("Model") as HTMLInputElement).value).toBe(
      "custom-model",
    );
  });

  it("persists the selected workspace and runtime input values with a definition", async () => {
    const definition = {
      schemaVersion: 1,
      id: "d",
      revision: 1,
      name: "Saved",
      inputs: [{ key: "task", label: "Runtime value", default: "default" }],
      steps: [{ id: "wait", kind: "wait", durationMs: 1 }],
    } as const;
    const saved = {
      ...definition,
      revision: 2,
      workspace: "/work/persisted",
      inputValues: { task: "saved runtime value" },
    };
    api.automationDefinitionsList.mockResolvedValue([definition]);
    api.automationDefinitionsSave.mockResolvedValue(saved);
    const first = render(<AutomationPalette />);
    fireEvent.click(await screen.findByRole("button", { name: "Saved" }));
    fireEvent.change(screen.getByLabelText("Show work folder"), {
      target: { value: "/work/persisted" },
    });
    fireEvent.change(screen.getByLabelText("Runtime value"), {
      target: { value: "saved runtime value" },
    });
    fireEvent.click(screen.getByRole("button", { name: "Save" }));
    await waitFor(() =>
      expect(api.automationDefinitionsSave).toHaveBeenCalledWith(
        expect.objectContaining({
          workspace: "/work/persisted",
          inputValues: { task: "saved runtime value" },
        }),
      ),
    );
    await waitFor(() =>
      expect(
        (screen.getByRole("button", { name: "Run" }) as HTMLButtonElement)
          .disabled,
      ).toBe(false),
    );

    first.unmount();
    api.automationDefinitionsList.mockResolvedValue([saved]);
    render(<AutomationPalette />);
    fireEvent.click(await screen.findByRole("button", { name: "Saved" }));
    expect(
      (screen.getByLabelText("Show work folder") as HTMLInputElement).value,
    ).toBe("/work/persisted");
    const restoredValue = screen.getByLabelText(
      "Runtime value",
    ) as HTMLInputElement;
    expect(restoredValue.value).toBe("saved runtime value");
    fireEvent.change(restoredValue, {
      target: { value: "changed only for this run" },
    });
    expect(
      (screen.getByRole("button", { name: "Run" }) as HTMLButtonElement)
        .disabled,
    ).toBe(true);
  });

  it.each([
    [new Error("disk unavailable"), "disk unavailable"],
    ["automation-llm-task-invalid", "automation-llm-task-invalid"],
    [
      "write-failed: /work/automation.json: Permission denied",
      "/work/automation.json: Permission denied",
    ],
  ])(
    "shows the save failure reason and keeps unsaved values (%s)",
    async (error, reason) => {
      const definition = {
        schemaVersion: 1,
        id: "d",
        revision: 1,
        name: "Saved",
        inputs: [{ key: "task", label: "Runtime value", default: "default" }],
        steps: [{ id: "wait", kind: "wait", durationMs: 1 }],
      } as const;
      api.automationDefinitionsList.mockResolvedValue([definition]);
      api.automationDefinitionsSave.mockRejectedValue(error);
      render(<AutomationPalette />);
      fireEvent.click(await screen.findByRole("button", { name: "Saved" }));
      const workspace = screen.getByLabelText("Show work folder");
      const value = screen.getByLabelText("Runtime value");
      fireEvent.change(workspace, { target: { value: "/work/unsaved" } });
      fireEvent.change(value, { target: { value: "keep this" } });
      fireEvent.click(screen.getByRole("button", { name: "Save" }));
      expect(
        await screen.findByText(
          (text) => text.startsWith("Save failed:") && text.includes(reason),
        ),
      ).toBeTruthy();
      expect((workspace as HTMLInputElement).value).toBe("/work/unsaved");
      expect((value as HTMLInputElement).value).toBe("keep this");
    },
  );

  it("edits steps without running and prevents duplicate explicit starts", async () => {
    const definition = {
      schemaVersion: 1,
      id: "d",
      revision: 1,
      name: "Saved",
      inputs: [],
      steps: [{ id: "s", kind: "wait", durationMs: 1 }],
    } as const;
    let resolveStart!: (value: unknown) => void;
    api.automationDefinitionsSave.mockResolvedValue(definition);
    api.automationDefinitionsList.mockResolvedValue([definition]);
    api.automationRunStart.mockReturnValue(
      new Promise((resolve) => {
        resolveStart = resolve;
      }),
    );
    render(<AutomationPalette />);
    fireEvent.click(await screen.findByRole("button", { name: "Saved" }));
    fireEvent.click(screen.getByRole("button", { name: "LLM task" }));
    expect(api.automationRunStart).not.toHaveBeenCalled();
    fireEvent.click(screen.getAllByRole("button", { name: "Copy" })[1]);
    fireEvent.click(screen.getAllByRole("button", { name: "Move step up" })[1]);
    fireEvent.click(screen.getAllByRole("button", { name: "Delete" })[2]);
    fireEvent.click(screen.getByRole("button", { name: "Save" }));
    await waitFor(() =>
      expect(api.automationDefinitionsSave).toHaveBeenCalled(),
    );
    fireEvent.click(screen.getByRole("button", { name: "Run" }));
    fireEvent.click(screen.getByRole("button", { name: "Run" }));
    expect(api.automationRunStart).toHaveBeenCalledTimes(1);
    resolveStart({
      running: true,
      phase: "watching",
      cli: "claude",
      filePath: "",
      startedAtMs: 1,
    });
  });

  it("imports a definition without starting it", async () => {
    api.automationDefinitionsImport.mockResolvedValue({
      schemaVersion: 1,
      id: "imported",
      revision: 1,
      name: "Imported",
      inputs: [],
      steps: [{ id: "s", kind: "wait", durationMs: 1 }],
    });
    render(<AutomationPalette />);
    const file = { text: () => Promise.resolve("{}") };
    fireEvent.change(document.querySelector('input[type="file"]')!, {
      target: { files: [file] },
    });
    await waitFor(() =>
      expect(api.automationDefinitionsImport).toHaveBeenCalled(),
    );
    expect(api.automationRunStart).not.toHaveBeenCalled();
  });

  it("locks sidebar actions while saving", async () => {
    let resolveSave!: (value: unknown) => void;
    api.automationDefinitionsSave.mockReturnValue(
      new Promise((resolve) => {
        resolveSave = resolve;
      }),
    );
    render(<AutomationPalette />);
    await screen.findByText("Recent runs");
    fireEvent.click(await screen.findByRole("button", { name: "Single task" }));
    fireEvent.click(await screen.findByRole("button", { name: "Save" }));
    expect(
      (screen.getByRole("button", { name: "New" }) as HTMLButtonElement)
        .disabled,
    ).toBe(true);
    expect(
      (screen.getByRole("button", { name: "Single task" }) as HTMLButtonElement)
        .disabled,
    ).toBe(true);
    resolveSave({
      schemaVersion: 1,
      id: "x",
      revision: 1,
      name: "Single task",
      inputs: [],
      steps: [{ id: "s", kind: "wait", durationMs: 1 }],
    });
  });

  it("shows the six-step Claude to agy transition example and return settings", async () => {
    render(<AutomationPalette />);
    await screen.findByText("Recent runs");
    fireEvent.click(
      await screen.findByRole("button", {
        name: "Claude design → agy implementation",
      }),
    );
    await waitFor(() =>
      expect(
        (screen.getAllByRole("combobox")[0] as HTMLSelectElement).value,
      ).toBe("2"),
    );
    expect(
      screen.getAllByText("Exit CLI", { selector: "strong" }),
    ).toHaveLength(2);
    expect(
      screen.getAllByText("Return confirmation", { selector: "label" }),
    ).toHaveLength(2);
    expect(
      screen.getByText(/Every cycle launches and exits each CLI/i),
    ).toBeTruthy();
    await waitFor(() =>
      expect(api.automationCliTransitionPreview).toHaveBeenCalledWith(
        "agent",
        "claude",
        undefined,
        undefined,
      ),
    );
  });

  it("disables a saved v2 run when the actual shell cannot return automatically", async () => {
    const definition = {
      schemaVersion: 2,
      id: "v2",
      revision: 1,
      name: "Transition",
      inputs: [],
      steps: [
        { id: "l", kind: "launchCli", cliProfileId: "claude" },
        { id: "x", kind: "exitCli", command: "/exit" },
      ],
    } as const;
    api.automationDefinitionsList.mockResolvedValue([definition]);
    api.automationCliTransitionPreview.mockResolvedValue({
      autoReturnSupported: false,
      unavailableReason: "automation-cli-return-shell-unsupported",
    });
    render(<AutomationPalette />);
    await screen.findByText("Recent runs");
    fireEvent.click(await screen.findByRole("button", { name: "Transition" }));
    expect(await screen.findByText(/This shell is not supported/)).toBeTruthy();
    expect(
      (screen.getByRole("button", { name: "Run" }) as HTMLButtonElement)
        .disabled,
    ).toBe(true);
  });

  it("does not let a stale save overwrite a newly opened editor", async () => {
    let resolveSave!: (value: unknown) => void;
    api.automationDefinitionsSave.mockReturnValue(
      new Promise((resolve) => {
        resolveSave = resolve;
      }),
    );
    render(<AutomationPalette />);
    await screen.findByText("Recent runs");
    fireEvent.click(screen.getByRole("button", { name: "Single task" }));
    fireEvent.click(screen.getByRole("button", { name: "Save" }));
    useAppStore.setState({
      automationEditor: { agentId: "other", workspace: "/other" },
    } as never);
    await waitFor(() => expect(screen.getByText("/other")).toBeTruthy());
    fireEvent.click(screen.getByRole("button", { name: "New" }));
    resolveSave({
      schemaVersion: 1,
      id: "old",
      revision: 1,
      name: "Old save",
      inputs: [],
      steps: [{ id: "s", kind: "wait", durationMs: 1 }],
    });
    await waitFor(() =>
      expect(screen.queryByDisplayValue("Old save")).toBeNull(),
    );
  });
});
