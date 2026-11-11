import { describe, expect, it } from "vitest";
import type { WorkItem } from "@shared/types";
import { dependencyCandidates, dependentCount, explainDependsOn, normalizeDependsOn, pendingPrerequisites } from "../workDependencies";

const work = (id: string, status: WorkItem["status"] = "todo", dependsOn?: string[]): WorkItem =>
  ({ id, title: id, body: "", status, order: 0, sessionIds: [], dependsOn, createdAt: 1, updatedAt: 1 });

describe("work dependencies", () => {
  // a ← b ← c (c는 b를, b는 a를 기다린다)
  const items = [work("a", "done"), work("b", "todo", ["a"]), work("c", "todo", ["b"]), work("d")];
  it("없는 id·자기 자신·중복·순환을 빼고 비면 undefined로 둔다", () => {
    expect(normalizeDependsOn("a", ["c", "d", "d", "a", "gone"], items)).toEqual(["d"]);
    expect(normalizeDependsOn("d", ["c", "a"], items)).toEqual(["c", "a"]);
    expect(normalizeDependsOn(undefined, ["c"], items)).toEqual(["c"]);
    expect(normalizeDependsOn("d", [], items)).toBeUndefined();
  });
  it("뺀 선행 작업의 사유를 알려주고 중복은 조용히 합친다", () => {
    expect(explainDependsOn("a", ["c", "d", "d", "a", "gone", "gone"], items)).toEqual({
      kept: ["d"], dropped: [{ id: "c", reason: "cycle" }, { id: "a", reason: "self" }, { id: "gone", reason: "unknown" }],
    });
    expect(explainDependsOn("d", undefined, items)).toEqual({ kept: undefined, dropped: [] });
  });
  it("기존 순환은 멈추고 후보마다 목표로 이어지는 경로를 따로 판정한다", () => {
    const cyclic = [
      work("root"), work("a", "todo", ["b"]), work("b", "todo", ["a"]),
      work("c", "todo", ["root"]), work("d", "todo", ["c"]),
    ];
    expect(explainDependsOn("root", ["a", "b", "c", "d"], cyclic)).toEqual({
      kept: ["a", "b"], dropped: [{ id: "c", reason: "cycle" }, { id: "d", reason: "cycle" }],
    });
    expect(dependencyCandidates("root", [], cyclic).map((i) => i.id)).toEqual(["a", "b"]);
  });
  it("완료되지 않은 선행 작업만 막고 지워진 업무는 무시한다", () => {
    expect(pendingPrerequisites(items[1], items)).toEqual([]);
    expect(pendingPrerequisites(items[2], items).map((i) => i.id)).toEqual(["b"]);
    expect(pendingPrerequisites({ dependsOn: ["gone"] }, items)).toEqual([]);
  });
  it("후보에서 자신·고른 것·완료·순환을 빼고 후속 작업 수를 센다", () => {
    expect(dependencyCandidates("b", [], items).map((i) => i.id)).toEqual(["d"]);
    expect(dependencyCandidates(undefined, ["d"], items).map((i) => i.id)).toEqual(["b", "c"]);
    expect(dependentCount("b", items)).toBe(1);
    expect(dependentCount("d", items)).toBe(0);
  });
});
