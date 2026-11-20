import { beforeEach, describe, expect, it, vi } from "vitest";
import type { Discussion } from "@shared/types";

const { api, run } = vi.hoisted(() => ({
  api: { loadDiscussions: vi.fn(), scanDiscussionInbox: vi.fn(), saveDiscussion: vi.fn() },
  run: { startDiscussion: vi.fn(), answerDiscussion: vi.fn() },
}));
vi.mock("../../ipc/tauriApi", () => ({ tauriApi: api }));
vi.mock("../discussionRun", () => run);
import { resetDiscussionStoreForTest, useDiscussionStore } from "../discussionStore";
import { handleDiscussionControl } from "../discussionControl";
import { receiveDiscussionInbox } from "../discussionInbox";

function discussion(id: string, patch: Partial<Discussion> = {}): Discussion {
  return {
    id, topic: "Where does the meeting room go?", context: "", moderatorAgentId: "m",
    participants: [{ agentId: "m", agent: { cli: "claude" } }, { agentId: "a", agent: { cli: "codex" } }],
    limits: { maxRounds: 4, maxMinutes: 30, maxCostUsd: 5, usageGuard: true }, state: "ready", round: 0, transcript: [],
    costUsd: 0, createdAt: 1, updatedAt: 1, ...patch,
  };
}

beforeEach(() => {
  Object.values(api).forEach((f) => f.mockReset());
  Object.values(run).forEach((f) => f.mockReset());
  resetDiscussionStoreForTest();
});

describe("CLI 토의 요청", () => {
  it("시작은 카드와 같은 함수를 부르고 막힌 이유를 그대로 돌려준다", async () => {
    useDiscussionStore.setState({ discussions: [discussion("d1")], status: "ready" });
    run.startDiscussion.mockReturnValueOnce(undefined).mockReturnValueOnce("discussion-busy");
    expect(await handleDiscussionControl({ requestId: "r", op: "start", discussionId: "d1" })).toBeUndefined();
    expect(await handleDiscussionControl({ requestId: "r", op: "start", discussionId: "d1" })).toBe("discussion-busy");
    expect(run.startDiscussion).toHaveBeenCalledWith("d1");
  });

  it("답은 사회자 질문을 기다릴 때만 받고 빈 답은 거절한다", async () => {
    useDiscussionStore.setState({ discussions: [discussion("d1", { state: "waiting_user" })], status: "ready" });
    run.answerDiscussion.mockReturnValueOnce(true).mockReturnValueOnce(false);
    expect(await handleDiscussionControl({ requestId: "r", op: "answer", discussionId: "d1", options: { text: " Budget is fine. " } })).toBeUndefined();
    expect(run.answerDiscussion).toHaveBeenCalledWith("d1", "Budget is fine.");
    expect(await handleDiscussionControl({ requestId: "r", op: "answer", discussionId: "d1", options: { text: "again" } })).toBe("discussion-not-waiting");
    expect(await handleDiscussionControl({ requestId: "r", op: "answer", discussionId: "d1", options: { text: "  " } })).toBe("discussion-answer-empty");
    expect(run.answerDiscussion).toHaveBeenCalledTimes(2);
  });

  it.each(["start", "answer"] as const)("%s 요청의 저장 실패를 호출자에게 알린다", async (op) => {
    const d = discussion("d1", { state: "running" });
    useDiscussionStore.setState({ discussions: [d], status: "ready" });
    api.saveDiscussion.mockRejectedValueOnce(new Error("disk full"));
    const commit = () => { void useDiscussionStore.getState().commit(d); };
    run.startDiscussion.mockImplementation(commit);
    run.answerDiscussion.mockImplementation(() => { commit(); return true; });
    await expect(handleDiscussionControl({ requestId: "r", op, discussionId: "d1", options: { text: "Yes" } }))
      .rejects.toThrow("disk full");
  });

  it("방금 가져온 토의가 스토어에 붙을 때까지 기다리고, 끝내 없으면 거절한다", async () => {
    useDiscussionStore.setState({ discussions: [], status: "ready" });
    setTimeout(() => useDiscussionStore.getState().adopt([discussion("d2")]), 150);
    expect(await handleDiscussionControl({ requestId: "r", op: "start", discussionId: "d2" }, 1000)).toBeUndefined();
    expect(run.startDiscussion).toHaveBeenCalledWith("d2");
    expect(await handleDiscussionControl({ requestId: "r", op: "start", discussionId: "nope" }, 50)).toBe("discussion-not-found");
    expect(await handleDiscussionControl({ requestId: "r", op: "confirm" as "start", discussionId: "d2" }, 0)).toBe("discussion-unknown-op");
  });
});

describe("토의 수신함", () => {
  it("새 토의만 붙이고 이미 아는 토의(실행기가 먼저 고친 것)는 덮지 않는다", async () => {
    const running = discussion("d1", { state: "running", round: 2 });
    useDiscussionStore.setState({ discussions: [running], status: "ready" });
    api.scanDiscussionInbox.mockResolvedValue([discussion("d1"), discussion("d3"), discussion("d3")]);
    await receiveDiscussionInbox();
    expect(useDiscussionStore.getState().discussions).toEqual([running, discussion("d3")]);
    expect(api.saveDiscussion).not.toHaveBeenCalled();
  });

  it("목록을 다 읽은 뒤에 가져오고, 읽지 못하면 가져오지 않는다", async () => {
    api.loadDiscussions.mockResolvedValue([discussion("d1")]);
    api.scanDiscussionInbox.mockResolvedValue([discussion("d4")]);
    await receiveDiscussionInbox();
    expect(useDiscussionStore.getState().discussions.map((d) => d.id)).toEqual(["d1", "d4"]);

    resetDiscussionStoreForTest();
    api.scanDiscussionInbox.mockClear();
    api.loadDiscussions.mockRejectedValue(new Error("discussions-corrupt"));
    await receiveDiscussionInbox();
    expect(api.scanDiscussionInbox).not.toHaveBeenCalled();
  });
});
