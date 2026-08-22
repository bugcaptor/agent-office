// src/renderer/labels/__tests__/gitBranchWatcher.test.ts
//
// 라벨용 cwd→브랜치 폴링의 순수 로직 테스트. 타이머/구독은 bootstrap 배선의
// 몫이라 여기서는 "무엇을 조회 대상으로 삼는가(liveAgentCwds)"와 "결과를 맵에
// 어떻게 반영하는가(nextGitBranches)"만 본다.
import { describe, expect, it } from "vitest";
import { liveAgentCwds, nextGitBranches } from "../gitBranchWatcher";
import type { GitBranchSource } from "../gitBranchWatcher";
import type { AgentProfile, AgentTaskLabel, SessionStatus } from "../../store/types";

function agent(id: string, cwd?: string): AgentProfile {
  return { id, name: id, role: "", seed: id, createdAt: 0, deskIndex: 0, cwd };
}

function src(
  rows: Array<{ id: string; cwd?: string; status: SessionStatus; labelCwd?: string }>
): GitBranchSource {
  const s: GitBranchSource = { agents: {}, sessions: {}, taskLabels: {} };
  for (const r of rows) {
    s.agents[r.id] = agent(r.id, r.cwd);
    s.sessions[r.id] = { status: r.status };
    if (r.labelCwd) s.taskLabels[r.id] = { sessionId: "s1", cwd: r.labelCwd } as AgentTaskLabel;
  }
  return s;
}

describe("liveAgentCwds", () => {
  it("라이브(starting/running) 세션의 cwd만 모은다", () => {
    const s = src([
      { id: "a", cwd: "/w/one", status: "running" },
      { id: "b", cwd: "/w/two", status: "starting" },
      { id: "c", cwd: "/w/three", status: "idle" },
      { id: "d", cwd: "/w/four", status: "exited" },
    ]);
    expect(liveAgentCwds(s)).toEqual(["/w/one", "/w/two"]);
  });

  it("세션 실제 cwd(taskLabels)가 프로필 cwd를 이기고, 중복은 하나로 접힌다", () => {
    const s = src([
      { id: "a", cwd: "/w/profile", status: "running", labelCwd: "/w/actual" },
      { id: "b", cwd: "/w/actual", status: "running" },
    ]);
    expect(liveAgentCwds(s)).toEqual(["/w/actual"]);
  });

  it("cwd 없는 에이전트는 대상이 아니다(백엔드 홈 폴백은 라벨 대상 아님)", () => {
    expect(liveAgentCwds(src([{ id: "a", status: "running" }]))).toEqual([]);
  });
});

describe("nextGitBranches", () => {
  it("브랜치가 있는 결과만 키로 남긴다", () => {
    const next = nextGitBranches(
      {},
      [
        { cwd: "/w/one", branch: "main" },
        { cwd: "/w/two", branch: null }, // 비저장소·detached·조회 실패
      ],
      ["/w/one", "/w/two"]
    );
    expect(next).toEqual({ "/w/one": "main" });
  });

  it("브랜치가 사라지면(detached 전환) 기존 키를 지운다", () => {
    const next = nextGitBranches(
      { "/w/one": "main" },
      [{ cwd: "/w/one", branch: null }],
      ["/w/one"]
    );
    expect(next).toEqual({});
  });

  it("라이브가 아닌 cwd는 결과에 있든 없든 가지치기된다", () => {
    const next = nextGitBranches(
      { "/w/dead": "old", "/w/one": "main" },
      [{ cwd: "/w/dead", branch: "still-there" }],
      ["/w/one"]
    );
    expect(next).toEqual({ "/w/one": "main" });
  });

  it("이번에 조회하지 않은 라이브 cwd는 이전 값을 유지한다(깜빡임 방지)", () => {
    const next = nextGitBranches(
      { "/w/one": "main" },
      [{ cwd: "/w/two", branch: "dev" }],
      ["/w/one", "/w/two"]
    );
    expect(next).toEqual({ "/w/one": "main", "/w/two": "dev" });
  });
});
