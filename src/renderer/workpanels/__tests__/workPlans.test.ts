import { describe, expect, it } from "vitest";
import type { WorkItem } from "@shared/types";
import { planLayout } from "../workPlans";

const work = (id: string, assignee: string, order: number, dependsOn?: string[], status: WorkItem["status"] = "todo"): WorkItem =>
  ({ id, title: id, body: "", status, order, sessionIds: [], dependsOn, assigneeAgentId: assignee || undefined, createdAt: 1, updatedAt: 1 });
const keyOf = (item: WorkItem) => item.assigneeAgentId ?? "";

describe("plan layout", () => {
  it("사슬 맨 끝만 루트로 보고 같은 담당·미배정 업무를 진행 순서대로 아래에 들인다", () => {
    // a ← b ← c ← root, x는 다른 캐릭터
    const items = [work("root", "k", 0, ["a", "b", "c", "x"]), work("c", "", 1, ["b"]), work("b", "k", 2, ["a"]), work("a", "k", 3, undefined, "done"), work("x", "p", 4)];
    const { members, claimed } = planLayout(items, keyOf);
    expect([...members.keys()]).toEqual(["root"]);
    expect(members.get("root")!.map((i) => i.id)).toEqual(["a", "b", "c"]);
    expect([...claimed].sort()).toEqual(["a", "b", "c"]);
  });
  it("완료된 루트는 묶지 않고, 두 플랜에 걸린 업무는 앞선 루트가 가져간다", () => {
    const items = [work("s", "k", 0), work("r1", "k", 1, ["s"]), work("r2", "k", 2, ["s"]), work("r3", "k", 3, ["s"], "done")];
    const { members } = planLayout(items, keyOf);
    expect(members.get("r1")!.map((i) => i.id)).toEqual(["s"]);
    expect(members.get("r2")).toEqual([]);
    expect(members.has("r3")).toBe(false);
  });
  it("선행 작업이 모두 지워진 업무는 루트가 아니다", () => {
    expect(planLayout([work("r", "k", 0, ["gone"])], keyOf).members.size).toBe(0);
  });
});
