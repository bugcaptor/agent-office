import { describe, expect, it } from "vitest";
import type { SessionEventRecord, SessionSummary, WorkItem } from "@shared/types";
import { buildSessionRows, filterSessionRows, folderName } from "../sessionRows";
import { normalizePanelLayout, panelExtent } from "../panelLayoutStore";
const now = 1_800_000_000_000;
function event(kind: SessionEventRecord["kind"], at: number, patch: Partial<SessionEventRecord> = {}): SessionEventRecord {
  return { kind, at, schemaVersion: 1, runId: "r1", seq: at, agentId: "a1", sessionId: "s1", ...patch };
}
const summary: SessionSummary = { sessionId: "s1", agentId: "a1", agentName: "Renamed", startedAt: now-1000, goal: "Fix regression", prompts: [{ at: now-900, text: "First prompt" }] };
describe("session rows", () => {
  it("시작 스냅샷을 보존하고 요약 본문·업무를 합친다", () => {
    const work: WorkItem = { id: "w1", title: "Task label", body: "", status: "todo", order: 0, sessionIds: ["s1"], createdAt: now, updatedAt: now };
    const rows = buildSessionRows([event("session_started", now-2000, { agentName: "Snapshot", cwd: "C:\\dev\\office" }), event("prompt",now-900,{origin:"bot"}), event("stop",now-100), event("session_state",now,{state:"exited"})], { s1: summary }, [work], [], now);
    expect(rows[0]).toMatchObject({ agentName: "Snapshot", cwd: "C:\\dev\\office", startedAt: now-2000, endedAt: now, workTitle: "Task label", goal: "Fix regression", prompts: [{ origin: "bot" }], stats: { turns: 1, workedMs: 800 } });
  });
  it("진행 중 세션은 위에 배치하고 열린 턴은 현재까지 계산한다", () => {
    const rows = buildSessionRows([event("session_started", now-1000), event("prompt",now-900), event("session_started",now-500,{sessionId:"s2"}), event("session_state",now-100,{sessionId:"s2",state:"disposed"})], {}, [], ["s1"], now);
    expect(rows[0]).toMatchObject({sessionId:"s1",live:true,stats:{turns:1,workedMs:900}});
    expect(rows[1].live).toBe(false);
  });
  it("중간 사용량은 턴을 늘리지 않고 모델별 비용을 이중 집계하지 않는다", () => {
    const rows = buildSessionRows([event("session_started",now-1000),event("prompt",now-900),event("usage",now-500,{partial:true,tokens:{input:1000,model:"unknown",byModel:[{input:1000,model:"claude-sonnet-4-5"}]}}),event("usage",now-100,{tokens:{input:500,model:"claude-sonnet-4-5"}}),event("stop",now)], {},[],[],now);
    expect(rows[0].stats).toMatchObject({turns:1,tokensIn:1500});
    expect(rows[0].usage).toHaveLength(2); expect(rows[0].stats?.costUsd).toBeCloseTo(0.0045);
  });
  it("삭제된 캐릭터와 요약이 없는 과거 세션을 표시한다", () => {
    const rows = buildSessionRows([event("session_started",now-1000,{agentName:"Deleted"})],{},[],[],now);
    expect(rows[0].agentName).toBe("Deleted"); expect(rows[0].prompts).toEqual([]);
    for (const query of ["deleted","regression","First PROMPT"]) {
      expect(filterSessionRows(buildSessionRows([], {s1:summary},[],[],now),{query:query==="deleted"?"renamed":query,agentIds:[],cwds:[],fromAt:0})).toHaveLength(1);
    }
    expect(filterSessionRows(rows,{query:"",agentIds:["missing"],cwds:[],fromAt:0})).toEqual([]);
    expect(buildSessionRows([event("session_started",now-301*86_400_000)],{},[],[],now)).toEqual([]);
  });
  it("경로와 저장 레이아웃의 경계를 처리한다", () => {
    expect(folderName("C:\\dev\\office\\")).toBe("office"); expect(folderName("/dev/office/")).toBe("office");
    expect(normalizePanelLayout({width:999,collapsed:true})).toEqual({width:480,collapsed:true});
    expect(normalizePanelLayout({width:NaN})).toEqual({width:280,collapsed:false});
    expect(panelExtent({width:280,collapsed:true})).toBe(24);
  });
});


it("제목 없는 연결 업무도 본문 제목으로 표시하고 검색한다", () => {
  const now = Date.now();
  const rows = buildSessionRows([], { s: { sessionId: "s", agentId: "a", agentName: "Ada", startedAt: now, prompts: [], workItemId: "w" } }, [
    { id: "w", title: "", body: "본문의 작업\n상세", status: "todo", order: 0, sessionIds: ["s"], createdAt: now, updatedAt: now },
  ], [], now);
  expect(rows[0].workTitle).toBe("본문의 작업 상세");
  expect(filterSessionRows(rows, { query: "본문의 작업", agentIds: [], cwds: [], fromAt: 0 })).toHaveLength(1);
});
