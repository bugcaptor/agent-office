import { describe, expect, it } from "vitest";
import fixture from "@shared/contract-fixtures/discussions-file.json";
import { discussionProblem, type AgentProfile, type Discussion, type DiscussionRole, type DiscussionsFile } from "@shared/types";
import {
  defaultDiscussionLimits, discussionFromDraft, draftFromDiscussion, newDiscussionDraft, participantDraft, validDiscussionCwd, withRole,
} from "../discussionModel";

const file = fixture as DiscussionsFile;
const profile = (id: string, extra: Partial<AgentProfile> = {}): AgentProfile => ({ id, name: id, role: "", seed: id, createdAt: 0, deskIndex: 0, ...extra });
const roles: DiscussionRole[] = [
  { id: "mod", name: "사회자", perspective: "", stance: "moderator", criteria: "" },
  { id: "critic", name: "비판자", perspective: "", stance: "critical", criteria: "", agent: { cli: "codex", model: "gpt" } },
  { id: "free", name: "자유", perspective: "", stance: "free", criteria: "" },
];
const profiles = {
  a: profile("a", { defaultRoleId: "critic", cwd: "/a" }),
  b: profile("b", { defaultRoleId: "mod", cwd: "/b", defaultAgent: { cli: "agy" } }),
  c: profile("c", { defaultRoleId: "free", defaultAgent: { cli: "pi", effort: "high" } }),
};

describe("discussion model", () => {
  it("accepts every contract fixture and rejects the same structures Rust rejects", () => {
    for (const d of file.discussions) expect(discussionProblem(d)).toBeNull();
    const base = file.discussions[1];
    const cases: [Partial<Discussion>, string][] = [
      [{ id: " " }, "id"],
      [{ topic: "  " }, "topic"],
      [{ participants: [base.participants[0], base.participants[0]] }, "duplicate"],
      [{ moderatorAgentId: "ghost" }, "moderator"],
      [{ participants: [base.participants[0]] }, "debaters"],
      [{ participants: ["m", "1", "2", "3", "4", "5"].map((agentId) => ({ agentId, agent: { cli: "claude" as const } })), moderatorAgentId: "m" }, "too-many"],
      [{ limits: { ...base.limits, maxRounds: 13 } }, "limits"],
      [{ limits: { ...base.limits, maxRounds: 2.5 } }, "limits"],
      [{ limits: { ...base.limits, maxMinutes: 4 } }, "limits"],
      [{ limits: { ...base.limits, maxCostUsd: NaN } }, "limits"],
    ];
    for (const [patch, code] of cases) expect(discussionProblem({ ...base, ...patch })).toBe(code);
  });

  it("fills roles from profiles (moderator seat only takes moderator roles) and CLI from role → profile → claude", () => {
    expect(participantDraft("a", profiles, roles, false)).toEqual({ agentId: "a", roleId: "critic", agent: { cli: "codex", model: "gpt" }, agentTouched: false });
    // 사회자 칸에서는 비판자 역할을 쓰지 않고 첫 사회자 역할을 넣는다.
    expect(participantDraft("a", profiles, roles, true)).toMatchObject({ roleId: "mod", agent: { cli: "claude" } });
    // 토론자 칸에서는 사회자 역할을 쓰지 않는다.
    expect(participantDraft("b", profiles, roles, false)).toMatchObject({ roleId: undefined, agent: { cli: "agy" } });
    expect(participantDraft("c", profiles, roles, false)).toMatchObject({ roleId: "free", agent: { cli: "pi", effort: "high" } });
    expect(participantDraft("ghost", profiles, [], false)).toEqual({ agentId: "ghost", roleId: undefined, agent: { cli: "claude" }, agentTouched: false });
  });

  it("changing the role refills the CLI only while the row was not edited by hand", () => {
    const row = participantDraft("c", profiles, roles, false);
    expect(withRole(row, "critic", profiles, roles).agent).toEqual({ cli: "codex", model: "gpt" });
    const touched = { ...row, agent: { cli: "claude" as const, model: "opus" }, agentTouched: true };
    expect(withRole(touched, "critic", profiles, roles)).toMatchObject({ roleId: "critic", agent: { cli: "claude", model: "opus" } });
  });

  it("starts a new draft with the default-moderator member and their folder", () => {
    const draft = newDiscussionDraft(["a", "b", "c"], profiles, roles);
    expect(draft.moderator).toMatchObject({ agentId: "b", roleId: "mod" });
    expect(draft).toMatchObject({ cwd: "/b", debaters: [], limits: defaultDiscussionLimits() });
    expect(defaultDiscussionLimits()).toEqual({ maxRounds: 4, maxMinutes: 30, maxCostUsd: 5, usageGuard: true });
    expect(newDiscussionDraft(["a"], { a: profiles.a }, []).moderator).toMatchObject({ agentId: "a", roleId: undefined });
    expect(newDiscussionDraft([], {}, roles).moderator).toBeNull();
  });

  it("round-trips a saved discussion through the draft without touching run state", () => {
    for (const base of file.discussions) {
      expect(discussionFromDraft(draftFromDiscussion(base), 99, base)).toEqual({ ...base, updatedAt: 99 });
    }
    const draft = draftFromDiscussion(file.discussions[0]);
    const edited = discussionFromDraft({ ...draft, cwd: "  ", topic: " New " }, 5, file.discussions[0]);
    expect(edited.cwd).toBeUndefined();
    expect(edited).toMatchObject({ topic: "New", transcript: file.discussions[0].transcript, costUsd: file.discussions[0].costUsd });
  });

  it("builds a new ready discussion with the moderator listed first", () => {
    const draft = newDiscussionDraft(["a", "b", "c"], profiles, roles);
    draft.topic = " 방향 ";
    draft.debaters = [participantDraft("a", profiles, roles, false), { ...participantDraft("c", profiles, roles, false), agent: { cli: "pi", model: "  ", effort: "high" } }];
    const d = discussionFromDraft(draft, 7);
    expect(d.id).toMatch(/^disc-/);
    expect(d).toMatchObject({ topic: "방향", state: "ready", round: 0, transcript: [], costUsd: 0, createdAt: 7, updatedAt: 7, cwd: "/b", moderatorAgentId: "b" });
    expect(d.participants).toEqual([
      { agentId: "b", roleId: "mod", agent: { cli: "agy" } },
      { agentId: "a", roleId: "critic", agent: { cli: "codex", model: "gpt" } },
      { agentId: "c", roleId: "free", agent: { cli: "pi", effort: "high" } },
    ]);
    expect(discussionProblem(d)).toBeNull();
  });
});

describe("작업 폴더", () => {
  it("현재 플랫폼에서 절대 경로와 빈 값만 받는다", () => {
    for (const windows of [true, false]) {
      expect(validDiscussionCwd(undefined, windows)).toBe(true);
      expect(validDiscussionCwd("./repo", windows)).toBe(false);
      expect(validDiscussionCwd("/work/repo", windows)).toBe(!windows);
      expect(validDiscussionCwd("C:\\work", windows)).toBe(windows);
      expect(validDiscussionCwd("C:work", windows)).toBe(false);
    }
    expect(validDiscussionCwd("\\\\server\\share", true)).toBe(true);
  });
});
