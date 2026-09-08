import { describe, expect, it } from "vitest";
import { useAppStore } from "./appStore";

describe("automation status reconciliation", () => {
  it("removes a finished snapshot once the backend has swept it", () => {
    useAppStore.setState({ automation: { a: { running: false, phase: "completed", cli: "claude", filePath: "", startedAtMs: 1 } } } as never);
    useAppStore.getState().applyAutomationStatus({ agents: {} });
    expect(useAppStore.getState().automation.a).toBeUndefined();
  });
});
