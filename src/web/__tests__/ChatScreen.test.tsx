// @vitest-environment jsdom
//
// src/web/__tests__/ChatScreen.test.tsx
//
// 채팅 뷰가 지켜야 할 렌더 계약 넷: ①버블 좌우와 도구 줄 접기 ②확인 요청
// 카드와 퀵 키가 서버 allowlist 이름으로 나간다 ③읽기 전용은 입력·퀵 키가
// 아예 없다 ④전사가 없으면 터미널 폴백 안내.

import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";

import { ChatScreen } from "@web/ChatScreen";
import type { HostMsg, NotificationItem, RemoteAgent } from "@web/protocol";
import type { ConnState, WebRemoteSocket } from "@web/ws";

const agent: RemoteAgent = { agentId: "a1", name: "아다", role: "backend" };

/** onMessage/onState/rpc만 흉내내는 소켓 스텁. */
function fakeSocket() {
  const listeners = new Set<(m: HostMsg) => void>();
  const rpc = vi.fn((_cmd: string, _args?: unknown) => Promise.resolve(null));
  const socket = {
    rpc,
    onMessage(cb: (m: HostMsg) => void) {
      listeners.add(cb);
      return () => listeners.delete(cb);
    },
    onState(cb: (s: ConnState) => void) {
      cb("open");
      return () => {};
    },
    send: vi.fn(),
  } as unknown as WebRemoteSocket;
  // 서버 push는 React 밖에서 오는 이벤트라 act로 감싸 상태 반영을 흘려보낸다.
  const push = (msg: HostMsg) => {
    act(() => {
      for (const cb of [...listeners]) cb(msg);
    });
  };
  return { socket, rpc, push };
}

function chat(items: HostMsg extends never ? never : unknown): HostMsg {
  return items as HostMsg;
}

const noop = () => {};

afterEach(cleanup);

describe("ChatScreen", () => {
  it("진입하면 그 캐릭터의 전사를 구독한다", () => {
    const { socket, rpc } = fakeSocket();
    render(
      <ChatScreen
        socket={socket}
        agent={agent}
        permission="input"
        notifications={[]}
        onBack={noop}
        onOpenTerminal={noop}
        onClearNotifications={noop}
      />
    );
    expect(rpc).toHaveBeenCalledWith("chat.follow", { agentId: "a1" });
  });

  it("버블과 접힌 도구 줄을 그린다", () => {
    const { socket, push } = fakeSocket();
    const { container } = render(
      <ChatScreen
        socket={socket}
        agent={agent}
        permission="input"
        notifications={[]}
        onBack={noop}
        onOpenTerminal={noop}
        onClearNotifications={noop}
      />
    );
    push(
      chat({
        type: "chat",
        agentId: "a1",
        backfill: true,
        items: [
          { role: "user", kind: "text", text: "로그를 고쳐줘" },
          { role: "assistant", kind: "text", text: "확인하겠습니다." },
          {
            role: "assistant",
            kind: "tool_use",
            text: "git status",
            toolName: "Bash",
          },
          { role: "user", kind: "tool_result", text: "boom", isError: true },
        ],
      })
    );

    expect(container.querySelector(".bubble.user")?.textContent).toBe("로그를 고쳐줘");
    expect(container.querySelector(".bubble.assistant")?.textContent).toBe(
      "확인하겠습니다."
    );
    // 도구는 한 줄로 접혀 있다 — 본문은 펼쳐야 나온다.
    expect(screen.queryByText("git status")).toBeNull();
    const head = screen.getByText(/Bash · git status/);
    fireEvent.click(head);
    expect(container.querySelector(".tool-body")?.textContent).toBe("git status");
    // 오류 결과는 경고색 클래스를 단다.
    expect(container.querySelector(".tool-line.tool-error")).not.toBeNull();

    // 다른 캐릭터의 프레임은 무시한다.
    push(
      chat({
        type: "chat",
        agentId: "other",
        items: [{ role: "user", kind: "text", text: "남의 말" }],
      })
    );
    expect(screen.queryByText("남의 말")).toBeNull();
  });

  it("확인 요청은 카드로 고정되고 퀵 키가 서버 이름으로 나간다", () => {
    const { socket, rpc } = fakeSocket();
    const question: NotificationItem = {
      id: "n1",
      agentId: "a1",
      sessionId: "s1",
      message: "계속 진행할까요?",
      at: 1,
      source: "hook",
    };
    const done: NotificationItem = { ...question, id: "n2", message: "끝났습니다", source: "stop" };
    const { container } = render(
      <ChatScreen
        socket={socket}
        agent={agent}
        permission="input"
        notifications={[question, done]}
        onBack={noop}
        onOpenTerminal={noop}
        onClearNotifications={noop}
      />
    );
    expect(container.querySelector(".ask-card")?.textContent).toContain(
      "계속 진행할까요?"
    );
    // stop 알림은 카드가 아니라 흘러가는 라인이다.
    expect(container.querySelector(".notice-line")?.textContent).toContain("끝났습니다");

    fireEvent.click(screen.getByRole("button", { name: "y" }));
    expect(rpc).toHaveBeenCalledWith("chat.keys", { agentId: "a1", keys: ["y"] });
    fireEvent.click(screen.getByRole("button", { name: "^C" }));
    expect(rpc).toHaveBeenCalledWith("chat.keys", { agentId: "a1", keys: ["ctrl-c"] });
  });

  it("입력칸은 문장을 통째로 보내고 비운다", () => {
    const { socket, rpc } = fakeSocket();
    const { container } = render(
      <ChatScreen
        socket={socket}
        agent={agent}
        permission="input"
        notifications={[]}
        onBack={noop}
        onOpenTerminal={noop}
        onClearNotifications={noop}
      />
    );
    const input = container.querySelector(".chat-input input") as HTMLInputElement;
    fireEvent.change(input, { target: { value: "  테스트를 돌려줘  " } });
    fireEvent.submit(container.querySelector(".chat-input") as HTMLFormElement);
    expect(rpc).toHaveBeenCalledWith("chat.send", {
      agentId: "a1",
      text: "테스트를 돌려줘",
    });
    expect(input.value).toBe("");
  });

  it("읽기 전용은 입력칸도 퀵 키도 없다", () => {
    const { socket } = fakeSocket();
    const { container } = render(
      <ChatScreen
        socket={socket}
        agent={agent}
        permission="readOnly"
        notifications={[
          {
            id: "n1",
            agentId: "a1",
            sessionId: "s1",
            message: "계속할까요?",
            at: 1,
            source: "hook",
          },
        ]}
        onBack={noop}
        onOpenTerminal={noop}
        onClearNotifications={noop}
      />
    );
    expect(container.querySelector(".chat-input")).toBeNull();
    expect(container.querySelector(".ask-keys")).toBeNull();
    // 카드 자체(무엇을 묻는지)는 보인다.
    expect(container.querySelector(".ask-card")).not.toBeNull();
  });

  it("전사가 없으면 터미널 폴백을 안내한다", () => {
    const { socket, push } = fakeSocket();
    const onOpenTerminal = vi.fn();
    render(
      <ChatScreen
        socket={socket}
        agent={agent}
        permission="input"
        notifications={[]}
        onBack={noop}
        onOpenTerminal={onOpenTerminal}
        onClearNotifications={noop}
      />
    );
    push(chat({ type: "chat", agentId: "a1", unavailable: true }));
    expect(screen.getByText(/전사가 없어 채팅을 표시할 수 없습니다/)).toBeTruthy();
    fireEvent.click(screen.getByRole("button", { name: "터미널로 보기" }));
    expect(onOpenTerminal).toHaveBeenCalled();
  });

  it("활동 이벤트는 진행 라인으로 뜨고 새 대화가 오면 사라진다", () => {
    const { socket, push } = fakeSocket();
    const { container } = render(
      <ChatScreen
        socket={socket}
        agent={agent}
        permission="input"
        notifications={[]}
        onBack={noop}
        onOpenTerminal={noop}
        onClearNotifications={noop}
      />
    );
    push(
      chat({
        type: "activity",
        agentId: "a1",
        payload: { kind: "tool", text: "Bash: npm test" },
      })
    );
    expect(container.querySelector(".activity-line")?.textContent).toBe(
      "⏳ 작업 중 · 🔧 Bash: npm test"
    );
    push(
      chat({
        type: "chat",
        agentId: "a1",
        items: [{ role: "assistant", kind: "text", text: "끝냈습니다" }],
      })
    );
    expect(container.querySelector(".activity-line")).toBeNull();
  });
});
