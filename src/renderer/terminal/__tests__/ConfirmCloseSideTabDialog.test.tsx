// @vitest-environment jsdom
import { act, cleanup, render } from "@testing-library/react";
import { afterEach, describe, expect, it } from "vitest";
import { useAppStore } from "../../store/appStore";
import { useDiscussionSessionStore, resetDiscussionSessionStoreForTest } from "../../discussion/discussionSessionStore";
import { useTaskChatStore } from "../../workpanels/taskChatStore";
import { ConfirmCloseSideTabDialog } from "../ConfirmCloseSideTabDialog";

const sessionId = "discuss-d1.a";

afterEach(() => {
  cleanup();
  resetDiscussionSessionStoreForTest();
  useTaskChatStore.setState({ chats: {}, order: [], opening: {} });
  useAppStore.getState().closeModal();
});

describe("곁 세션 탭 닫기 확인", () => {
  it("확인창이 떠 있는 사이 탭이 스스로 사라지면 모달도 닫는다(단축키를 막지 않게)", () => {
    useDiscussionSessionStore.setState({
      sessions: {
        [sessionId]: {
          agentId: sessionId, discussionId: "d1", participantAgentId: "a", sessionId: "s1", cli: "claude", status: "running",
          collection: "submit", observed: true, openedAt: 0, submittedAt: 0,
        },
      },
      order: [sessionId],
    });
    useAppStore.getState().openModal({ kind: "confirm-close-discussion-session", agentId: sessionId });
    const { container } = render(<ConfirmCloseSideTabDialog />);
    expect(container.querySelector(".confirm-close-discussion-session-dialog")).not.toBeNull();

    act(() => useDiscussionSessionStore.setState({ sessions: {}, order: [] }));
    expect(container.querySelector(".confirm-close-discussion-session-dialog")).toBeNull();
    expect(useAppStore.getState().modal.kind).toBe("none");
  });

  it("없는 태스크 대화를 가리키는 확인창도 닫는다", () => {
    useAppStore.getState().openModal({ kind: "confirm-close-task-chat", agentId: "task-chat-t1" });
    render(<ConfirmCloseSideTabDialog />);
    expect(useAppStore.getState().modal.kind).toBe("none");
  });
});
