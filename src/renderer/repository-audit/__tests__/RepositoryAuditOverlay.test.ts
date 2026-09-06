import { describe, expect, it } from "vitest";
import { auditStates } from "../RepositoryAuditOverlay";
import type { RepositoryAuditItem } from "@shared/types";

const base: RepositoryAuditItem = { path: "/repo", name: "repo", lastWorkedAt: 0, branch: "main", upstream: "origin/main", hasRemote: true, ahead: 0, behind: 0, changedCount: 0, conflictCount: 0, timedOut: false, unavailable: false };

describe("repository audit status", () => {
  it("prioritizes unknown and conflict signals over ordinary git state", () => {
    expect(auditStates({ ...base, changedCount: 2, conflictCount: 1 })).toEqual(["conflict", "commit"]);
    expect(auditStates({ ...base, ahead: 2, timedOut: true })).toEqual(["unknown"]);
  });
  it("describes commit, sync and clean states", () => {
    expect(auditStates({ ...base, changedCount: 1, ahead: 2 })).toEqual(["commit", "push"]);
    expect(auditStates({ ...base, ahead: 1, behind: 1 })).toEqual(["diverged"]);
    expect(auditStates({ ...base, ahead: 1 })).toEqual(["push"]);
    expect(auditStates({ ...base, behind: 1 })).toEqual(["pull"]);
    expect(auditStates(base)).toEqual(["clean"]);
  });
  it("does not describe missing remote configuration as clean", () => {
    expect(auditStates({ ...base, hasRemote: false, upstream: null })).toEqual(["noRemote"]);
    expect(auditStates({ ...base, upstream: null })).toEqual(["noUpstream"]);
  });
});
