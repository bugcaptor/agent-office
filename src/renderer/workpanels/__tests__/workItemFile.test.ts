import { describe, expect, it, vi } from "vitest";
vi.mock("../../ipc/tauriApi", () => ({ tauriApi: {} }));
import type { WorkItem } from "@shared/types";
import { parseWorkItemsFile, serializeWorkItems, workItemsFileName } from "../workItemFile";

const item = (patch: Partial<WorkItem>): WorkItem => ({ id: "a", title: "A", body: "", status: "todo", order: 0, sessionIds: [], createdAt: 0, updatedAt: 0, ...patch });
const items = [
  item({ id: "d", title: "Done", status: "done" }),
  item({ id: "t2", title: "Second", order: 1, dependsOn: ["t1"], assigneeAgentId: "a1", agent: { cli: "codex", model: "m", effort: "high" } }),
  item({ id: "t1", title: "First", body: "line 1\nline 2", sessionIds: ["s1"], lastError: "work-send-busy", createdAt: Date.UTC(2026, 9, 1), updatedAt: Date.UTC(2026, 9, 2) }),
  item({ id: "r", title: "Review", status: "review" }),
];

describe("work items file", () => {
  it("고른 상태만 그룹·순서대로 저장 필드 전부를 사람이 읽는 들여쓴 JSON으로 쓴다", () => {
    const { json, count } = serializeWorkItems(items, ["todo", "review"], Date.UTC(2026, 9, 13));
    expect(count).toBe(3);
    expect(json).toContain('\n  "items": [\n    {\n      "id": "r"');
    const file = JSON.parse(json);
    expect(file).toMatchObject({ format: "agent-office-work-items", version: 1, exportedAt: "2026-10-13T00:00:00.000Z" });
    expect(file.items.map((i: WorkItem) => i.id)).toEqual(["r", "t1", "t2"]);
    expect(file.items[1]).toEqual({ id: "t1", title: "First", status: "todo", body: ["line 1", "line 2"], order: 0, sessionIds: ["s1"], lastError: "work-send-busy", createdAt: "2026-10-01T00:00:00.000Z", updatedAt: "2026-10-02T00:00:00.000Z" });
    expect(file.items[2]).toMatchObject({ dependsOn: ["t1"], assigneeAgentId: "a1", agent: { cli: "codex", model: "m", effort: "high" } });
  });
  it("내보낸 파일을 그대로 다시 읽으면 업무 정보가 보존된다", () => {
    const parsed = parseWorkItemsFile(serializeWorkItems(items, ["todo", "in_progress", "review", "done"]).json);
    expect(parsed.ok && parsed.skipped).toBe(0);
    if (!parsed.ok) return;
    expect(parsed.items.find((i) => i.sourceId === "t1")).toEqual({ sourceId: "t1", title: "First", body: "line 1\nline 2", status: "todo", assigneeAgentId: undefined, agent: undefined, dependsOn: undefined, createdAt: Date.UTC(2026, 9, 1) });
    expect(parsed.items.find((i) => i.sourceId === "t2")).toMatchObject({ dependsOn: ["t1"], assigneeAgentId: "a1", agent: { cli: "codex", model: "m", effort: "high" } });
  });
  it("손으로 고친 파일의 빈·틀린 항목은 건너뛰고 문자열 본문·모르는 상태·work-items.json 배열도 받는다", () => {
    const parsed = parseWorkItemsFile(JSON.stringify({ items: [{ title: " ", body: [] }, "x", { body: "only body", status: "later", agent: { cli: "nope" } }, { title: "T", createdAt: 5 }] }));
    expect(parsed).toMatchObject({ ok: true, skipped: 2, items: [{ title: "", body: "only body", status: "todo", agent: undefined }, { title: "T", createdAt: 5 }] });
    expect(parseWorkItemsFile(`﻿${JSON.stringify(items)}`)).toMatchObject({ ok: true, skipped: 0 });
  });
  it("JSON이 아니거나 다른 형식·새 버전·빈 목록은 오류로 알린다", () => {
    expect(parseWorkItemsFile("{")).toEqual({ ok: false, error: "invalid" });
    expect(parseWorkItemsFile('{"format":"agent-office-character","items":[]}')).toEqual({ ok: false, error: "format" });
    expect(parseWorkItemsFile('{"title":"x"}')).toEqual({ ok: false, error: "format" });
    expect(parseWorkItemsFile('{"version":2,"items":[{"title":"x"}]}')).toEqual({ ok: false, error: "version" });
    expect(parseWorkItemsFile('{"items":[{"title":""}]}')).toEqual({ ok: false, error: "empty" });
  });
  it("기본 파일 이름에 날짜를 넣는다", () => {
    expect(workItemsFileName(new Date(2026, 0, 5))).toBe("agent-office-work-items-20260105.json");
  });
});
