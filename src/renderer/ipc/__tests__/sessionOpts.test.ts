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

  it("preserves a multiline personalityPrompt", () => {
    expect(sessionOptsFor({ personalityPrompt: "차분하게 답한다.\n항상 근거를 든다." })).toEqual({
      personalityPrompt: "차분하게 답한다.\n항상 근거를 든다.",
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

  describe("startupCommand override (Claude 이어하기)", () => {
    it("override가 프로필 startupCommand를 대체하고 나머지 필드는 유지한다", () => {
      expect(
        sessionOptsFor(
          {
            name: "Compiler",
            role: "Platform",
            cwd: "/work",
            shell: "zsh",
            startupCommand: "source ./init.sh",
            personalityPrompt: "차분히",
          },
          { startupCommand: "claude --resume abc-123" },
        ),
      ).toEqual({
        agentName: "Compiler",
        agentRole: "Platform",
        cwd: "/work",
        shell: "zsh",
        startupCommand: "claude --resume abc-123",
        personalityPrompt: "차분히",
      });
    });

    it("프로필에 startupCommand가 없어도 override를 주입한다", () => {
      expect(sessionOptsFor({ cwd: "/work" }, { startupCommand: "claude --resume x" })).toEqual({
        cwd: "/work",
        startupCommand: "claude --resume x",
      });
    });

    it("빈 override는 프로필 startupCommand로 폴백한다", () => {
      expect(
        sessionOptsFor({ startupCommand: "source ./init.sh" }, { startupCommand: "" }),
      ).toEqual({ startupCommand: "source ./init.sh" });
    });

    it("override가 undefined면 인자 하나짜리 호출과 동일하게 동작한다", () => {
      expect(sessionOptsFor({ cwd: "/work" }, undefined)).toEqual({ cwd: "/work" });
    });
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
