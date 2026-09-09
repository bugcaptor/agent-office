import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";
import type { AutomationDefinition, AutomationRunRecord, AutomationStatus } from "../types/automation";

const fixture = (name: string): unknown => JSON.parse(readFileSync(fileURLToPath(new URL(`../contract-fixtures/${name}.json`, import.meta.url)), "utf8"));

describe("automation wire contract", () => {
  it("reads every step variant's camelCase configuration", () => {
    const definition = fixture("automation-definition") as AutomationDefinition;
    expect(definition.schemaVersion).toBe(1);
    expect(definition.repeat?.maxCycles).toBe(3);
    expect(definition.inputs[0].default).toBe("자동화");
    expect(definition.inputValues).toEqual({ topic: "저장된 실행값" });
    expect(definition.steps.map(step => step.kind)).toEqual(["launchCli", "llmTask", "wait", "confirm", "exitCli"]);
    for (const step of definition.steps) {
      switch (step.kind) {
        case "launchCli": expect([step.cliProfileId, step.model, step.effort, step.startupWaitMs]).toEqual(["claude", "test-model", "high", 10000]); break;
        case "llmTask": expect([step.label, step.promptTemplate, step.waitTimeoutMs, step.completionGraceMs, step.allowEarlyComplete]).toEqual(["작성", "주제: {{topic}}\n검수용 문서를 작성한다.", 1800000, 10000, true]); break;
        case "wait": expect(step.durationMs).toBe(1000); break;
        case "confirm": expect(step.message).toBe("다음 단계로 진행할까요?"); break;
        case "exitCli": expect(step.command).toBe("/exit"); break;
      }
    }
  });

  it("retains the saved snapshot and restored confirmation identity", () => {
    const run = fixture("automation-run-record") as AutomationRunRecord;
    expect(run.definitionSnapshot).toEqual(fixture("automation-definition"));
    expect(run.status).toBe("interrupted");
    expect(run.events[1].kind).toBe("interrupted-after-restart");
    const status = (fixture("automation-status") as AutomationStatus).agents.a4;
    expect([status.definitionId, status.sessionId, status.cycle, status.stepIndex, status.stepId]).toEqual(["workflow-1", "session-4", 2, 3, "confirm"]);
    expect([status.phase, status.decisionReason, status.decisionId, status.decisionMessage]).toEqual(["confirming", "confirm", "decision-4", "다음 단계로 진행할까요?"]);
  });

  it("accepts agy as an automation CLI status value", () => {
    const status = (fixture("automation-status") as AutomationStatus).agents.a3;
    expect(status.cli).toBe("agy");
  });

  it("reads the v2 paired CLI-switch contract without changing the v1 fixture", () => {
    const definition = fixture("automation-definition.v2") as AutomationDefinition;
    expect(definition.schemaVersion).toBe(2);
    expect(definition.repeat?.maxCycles).toBe(2);
    const exits = definition.steps.filter((step): step is Extract<typeof step, { kind: "exitCli" }> => step.kind === "exitCli");
    expect(exits.map(step => [step.returnMode, step.exitWaitMs])).toEqual([["auto", 30000], ["manual", undefined]]);
  });
  it("preserves v2 return decisions and all CLI context variants", () => {
    const { agents } = fixture("automation-status.v2") as AutomationStatus;
    expect([agents.ready.decisionReason, agents.return.decisionReason, agents.manual.decisionReason]).toEqual(["shellReady", "cliExitTimeout", "cliExitUnconfirmed"]);
    expect(agents.ready.cliContext).toEqual({ state: "unknown" });
    expect(agents.return.cliContext).toEqual({ state: "cli", cli: "agy", launchStepId: "launch-agy" });
    expect(agents.shell.cliContext).toEqual({ state: "shell" });
    expect(agents.return.cliReturnObservedAtMs).toBe(1720000000010);
  });

});
