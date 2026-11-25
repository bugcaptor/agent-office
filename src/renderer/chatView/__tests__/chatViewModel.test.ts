import { describe, expect, it } from "vitest";
import type { ChatLiveStatus, ChatLogItem, ChatLogPage, PendingAsk } from "@shared/types";
import { STALL_MS, askQuestions, groupChatItems, joinAnswers, liveState, mergeChatPage, minSec, permissionSummary, runningTool } from "../chatViewModel";

let seq = 0;
const item = (over: Partial<ChatLogItem>): ChatLogItem => ({ seq: ++seq, role: "assistant", kind: "text", text: "", isError: false, sidechain: false, ...over });
const page = (epoch: number, items: ChatLogItem[], attached = true): ChatLogPage => ({ epoch, last: items[items.length - 1]?.seq ?? 0, attached, items });

describe("mergeChatPage", () => {
  it("appends within an epoch, replaces on a new epoch, and keeps the same object when nothing changed", () => {
    const a = item({ text: "a" });
    const b = item({ text: "b" });
    const first = mergeChatPage(null, page(1, [a]));
    const same = mergeChatPage(first, page(1, []));
    expect(same).toBe(first);
    const more = mergeChatPage(first, page(1, [a, b]));
    expect(more.items.map((i) => i.text)).toEqual(["a", "b"]);
    const fresh = mergeChatPage(more, page(2, [item({ text: "c" })]));
    expect(fresh.items.map((i) => i.text)).toEqual(["c"]);
  });
});

describe("groupChatItems", () => {
  it("pairs results with their calls in order and attaches images to the block before them", () => {
    const blocks = groupChatItems([
      item({ role: "user", text: "look" }),
      item({ role: "user", kind: "image", image: { id: "p", mediaType: "image/png", bytes: 1 } }),
      item({ kind: "tool_use", toolName: "Read", text: "a.png" }),
      item({ kind: "tool_use", toolName: "Bash", text: "ls" }),
      item({ role: "user", kind: "tool_result", text: "(image)" }),
      item({ role: "user", kind: "image", image: { id: "r", mediaType: "image/png", bytes: 1 } }),
      item({ role: "user", kind: "tool_result", text: "x" }),
      item({ text: "done" }),
      item({ role: "user", kind: "image", image: { id: "lone", mediaType: "image/png", bytes: 1 } }),
    ]);
    expect(blocks.map((b) => [b.head.kind, b.head.text || b.head.image?.id, b.children.map((c) => c.image?.id ?? c.text)])).toEqual([
      ["text", "look", ["p"]],
      ["tool_use", "a.png", ["(image)"]],
      ["tool_use", "ls", ["r", "x"]],
      ["text", "done", []],
      ["image", "lone", []],
    ]);
  });
});

describe("questions and permissions", () => {
  const ask = (toolName: string, toolInput: Record<string, unknown>): PendingAsk => ({ id: "ask-1", toolName, toolInput, canAlways: false, sidechain: false, at: 0 });

  it("reads questions defensively and joins picks with typed answers", () => {
    const qs = askQuestions(ask("AskUserQuestion", { questions: [
      { question: "Color?", header: "C", multiSelect: true, options: [{ label: "Red" }, { label: "Blue", description: "b" }, { nope: 1 }] },
      { header: "broken" },
    ] }));
    expect(qs).toEqual([{ question: "Color?", header: "C", multiSelect: true, options: [{ label: "Red", description: undefined }, { label: "Blue", description: "b" }] }]);
    expect(joinAnswers(qs, { "Color?": ["Red", "Blue"] }, { "Color?": " teal " })).toEqual({ "Color?": "Red, Blue, teal" });
    expect(joinAnswers(qs, {}, {})).toEqual({ "Color?": "" });
  });

  it("summarizes what each tool asks to do", () => {
    expect(permissionSummary(ask("Bash", { command: "ls", description: "list" }))).toEqual({ body: "ls", note: "list" });
    expect(permissionSummary(ask("Edit", { file_path: "/a", old_string: "x", new_string: "y" }))).toEqual({ target: "/a", body: "- x\n+ y" });
    expect(permissionSummary(ask("mcp__x", { path: "/p", k: 1 })).body).toContain('"k": 1');
  });
});

describe("liveState", () => {
  const status = (over: Partial<ChatLiveStatus>): ChatLiveStatus => ({ running: true, now: 100_000, queue: { items: [] }, ...over });

  it("tells thinking from a stalled or finished turn", () => {
    expect(liveState(status({ phase: "working", since: 58_000, outputAt: 99_500 }), "Bash")).toEqual({ kind: "thinking", elapsedMs: 42_000, tool: "Bash" });
    expect(liveState(status({ phase: "working", since: 10_000, outputAt: 100_000 - STALL_MS }))).toEqual({ kind: "stalled", elapsedMs: 90_000, silentMs: STALL_MS });
    expect(liveState(status({ phase: "idle" }))?.kind).toBe("idle");
    expect(liveState(status({ phase: "needs-human" }))?.kind).toBe("human");
    expect(liveState(status({ running: false }))?.kind).toBe("off");
    expect(liveState(status({}))).toBeNull();
    expect(minSec(125_400)).toEqual([2, 5]);
  });

  it("finds the main-chain tool still waiting for its result", () => {
    const call = item({ kind: "tool_use", toolName: "Bash", text: "npm test" });
    expect(runningTool(groupChatItems([call]))).toBe("Bash");
    expect(runningTool(groupChatItems([call, item({ role: "user", kind: "tool_result", text: "ok" })]))).toBeUndefined();
    expect(runningTool(groupChatItems([call, item({ kind: "tool_use", toolName: "Grep", text: "x", sidechain: true })]))).toBe("Bash");
    expect(runningTool(groupChatItems([item({ text: "done" })]))).toBeUndefined();
  });
});
