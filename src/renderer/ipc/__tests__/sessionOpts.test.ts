// src/renderer/ipc/__tests__/sessionOpts.test.ts
//
// TDD for `sessionOptsFor`: cwd/shell → createSession opts 변환 헬퍼.
import { describe, expect, it } from "vitest";
import { sessionOptsFor } from "../sessionOpts";

describe("sessionOptsFor", () => {
  it("returns undefined when given no agent", () => {
    expect(sessionOptsFor(undefined)).toBeUndefined();
  });

  it("returns undefined when the agent has neither cwd nor shell", () => {
    expect(sessionOptsFor({})).toBeUndefined();
  });

  it("includes only cwd when only cwd is set", () => {
    expect(sessionOptsFor({ cwd: "/a/b" })).toEqual({ cwd: "/a/b" });
  });

  it("includes only shell when only shell is set", () => {
    expect(sessionOptsFor({ shell: "pwsh" })).toEqual({ shell: "pwsh" });
  });

  it("includes both cwd and shell when both are set", () => {
    expect(sessionOptsFor({ cwd: "/a/b", shell: "wsl" })).toEqual({ cwd: "/a/b", shell: "wsl" });
  });

  it("includes only startupCommand when only startupCommand is set", () => {
    expect(sessionOptsFor({ startupCommand: "source ./init.sh" })).toEqual({
      startupCommand: "source ./init.sh",
    });
  });

  it("includes cwd, shell and startupCommand when all are set", () => {
    expect(
      sessionOptsFor({ cwd: "/a/b", shell: "wsl", startupCommand: "mysetup.bat" }),
    ).toEqual({ cwd: "/a/b", shell: "wsl", startupCommand: "mysetup.bat" });
  });

  it("omits empty-string cwd/shell/startupCommand (falsy)", () => {
    expect(sessionOptsFor({ cwd: "", shell: "", startupCommand: "" })).toBeUndefined();
  });

  it("includes the profile snapshot used by session event analysis", () => {
    expect(sessionOptsFor({ name: "Compiler", role: "Platform" })).toEqual({
      agentName: "Compiler",
      agentRole: "Platform",
    });
  });

  it("keeps cwd shell and startup command with the profile snapshot", () => {
    expect(
      sessionOptsFor({
        name: "Compiler",
        role: "Platform",
        cwd: "/work",
        shell: "zsh",
        startupCommand: "source ./init.sh",
      }),
    ).toEqual({
      agentName: "Compiler",
      agentRole: "Platform",
      cwd: "/work",
      shell: "zsh",
      startupCommand: "source ./init.sh",
    });
  });
});
