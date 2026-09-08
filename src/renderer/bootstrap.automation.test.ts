// @vitest-environment jsdom
import { describe, expect, it, vi } from "vitest";

const api = vi.hoisted(() => ({ automationStatus: vi.fn() }));
vi.mock("./ipc/tauriApi", () => ({ tauriApi: api }));

describe("automation bootstrap recovery", () => {
  it("retries a failed first status request and seeds the successful snapshot", async () => {
    vi.useFakeTimers();
    api.automationStatus
      .mockRejectedValueOnce(new Error("not ready"))
      .mockResolvedValueOnce({ agents: {} });
    const { seedAutomationMode } = await import("./bootstrap");
    const pending = seedAutomationMode();
    await vi.advanceTimersByTimeAsync(1_000);
    await pending;
    expect(api.automationStatus).toHaveBeenCalledTimes(2);
    vi.useRealTimers();
  });
  it("restores a pending v2 return decision without sending input", async () => {
    const status = {
      running: true,
      phase: "timeoutDecision" as const,
      cli: "agy" as const,
      filePath: "",
      startedAtMs: 1,
      runId: "run",
      stepExecutionId: "exit",
      decisionId: "decision",
      decisionReason: "cliExitTimeout" as const,
      cliContext: {
        state: "cli" as const,
        cli: "agy" as const,
        launchStepId: "launch",
      },
      cliReturnObservedAtMs: 2,
    };
    api.automationStatus.mockResolvedValueOnce({ agents: { a: status } });
    const { seedAutomationMode } = await import("./bootstrap");
    const { useAppStore } = await import("./store/appStore");
    await seedAutomationMode();
    expect(useAppStore.getState().automation.a).toEqual(status);
  });
});
