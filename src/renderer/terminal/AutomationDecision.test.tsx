// @vitest-environment jsdom
import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, beforeEach, describe, expect, it } from "vitest";
import { initI18nForTest } from "../i18n";
import { useAppStore } from "../store/appStore";
import { AutomationDecision } from "./AutomationDecision";

describe("AutomationDecision", () => {
  beforeEach(async () => {
    await initI18nForTest("en");
    useAppStore.setState({
      automation: {
        a: {
          running: true,
          phase: "confirming",
          cli: "claude",
          filePath: "",
          startedAtMs: 1,
          decisionReason: "protocolError",
        },
      },
    } as never);
  });
  afterEach(cleanup);
  it("offers only extend and stop for a protocol error", () => {
    render(<AutomationDecision agentId="a" />);
    expect(screen.getByRole("button", { name: "Keep waiting" })).toBeTruthy();
    expect(
      screen.getByRole("button", { name: "Stop automation" }),
    ).toBeTruthy();
    expect(screen.queryByRole("button", { name: "Continue" })).toBeNull();
  });
});
