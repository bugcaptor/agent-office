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

  it("omits empty-string cwd/shell (falsy)", () => {
    expect(sessionOptsFor({ cwd: "", shell: "" })).toBeUndefined();
  });
});
