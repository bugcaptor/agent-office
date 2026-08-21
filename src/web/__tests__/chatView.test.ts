// src/web/__tests__/chatView.test.ts
//
// 채팅 뷰의 순수 규칙. DOM 없이 도는 것들만 여기 있다(렌더는 ChatScreen.test).

import { describe, expect, it } from "vitest";
import {
  MAX_ITEMS,
  activityLine,
  applyChatFrame,
  isAtBottom,
  isQuestion,
  itemGlyph,
  toolSummary,
} from "@web/chatView";
import type { NotificationItem, TranscriptItem } from "@web/protocol";

const text = (t: string, role: "user" | "assistant" = "user"): TranscriptItem => ({
  role,
  kind: "text",
  text: t,
});

describe("applyChatFrame", () => {
  it("백필은 목록을 교체한다 — 재접속에 같은 대화가 두 벌 쌓이지 않는다", () => {
    const prev = [text("a"), text("b")];
    const next = applyChatFrame(prev, {
      items: [text("a"), text("b")],
      backfill: true,
    });
    expect(next.map((i) => i.text)).toEqual(["a", "b"]);
  });

  it("증분은 이어 붙인다", () => {
    const next = applyChatFrame([text("a")], { items: [text("b")] });
    expect(next.map((i) => i.text)).toEqual(["a", "b"]);
  });

  it("빈 증분은 참조를 유지한다(재렌더 유발 없음)", () => {
    const prev = [text("a")];
    expect(applyChatFrame(prev, { items: [] })).toBe(prev);
    expect(applyChatFrame(prev, {})).toBe(prev);
  });

  it("상한을 넘으면 오래된 것부터 버린다", () => {
    const many = Array.from({ length: MAX_ITEMS + 20 }, (_, i) => text(`m${i}`));
    const next = applyChatFrame([], { items: many, backfill: true });
    expect(next).toHaveLength(MAX_ITEMS);
    expect(next[next.length - 1].text).toBe(`m${MAX_ITEMS + 19}`);

    const appended = applyChatFrame(next, { items: [text("새것")] });
    expect(appended).toHaveLength(MAX_ITEMS);
    expect(appended[appended.length - 1].text).toBe("새것");
  });
});

describe("isAtBottom", () => {
  it("바닥 근처면 따라간다", () => {
    expect(isAtBottom({ scrollTop: 900, scrollHeight: 1000, clientHeight: 100 })).toBe(
      true
    );
    // 한 줄쯤 올라간 정도는 아직 바닥으로 친다.
    expect(isAtBottom({ scrollTop: 870, scrollHeight: 1000, clientHeight: 100 })).toBe(
      true
    );
  });

  it("위로 올려 읽는 중이면 따라가지 않는다", () => {
    expect(isAtBottom({ scrollTop: 200, scrollHeight: 1000, clientHeight: 100 })).toBe(
      false
    );
  });

  it("내용이 화면보다 짧으면 항상 바닥이다", () => {
    expect(isAtBottom({ scrollTop: 0, scrollHeight: 80, clientHeight: 100 })).toBe(true);
  });
});

describe("도구 줄", () => {
  it("이름과 첫 줄 요약을 붙인다", () => {
    expect(
      toolSummary({ role: "assistant", kind: "tool_use", text: "git status", toolName: "Bash" })
    ).toBe("Bash · git status");
    // 여러 줄이면 첫 줄만, 길면 자른다.
    const long = toolSummary({
      role: "user",
      kind: "tool_result",
      text: `${"x".repeat(200)}\n둘째 줄`,
    });
    expect(long.startsWith("결과 · ")).toBe(true);
    expect(long.endsWith("…")).toBe(true);
    expect(long).not.toContain("둘째 줄");
    // 본문이 없으면 이름만.
    expect(toolSummary({ role: "assistant", kind: "tool_use", text: "", toolName: "Read" })).toBe(
      "Read"
    );
  });

  it("오류 결과는 경고 글리프", () => {
    expect(itemGlyph({ role: "user", kind: "tool_result", text: "boom", isError: true })).toBe(
      "⚠️"
    );
    expect(itemGlyph({ role: "user", kind: "tool_result", text: "ok" })).toBe("↩︎");
    expect(itemGlyph({ role: "assistant", kind: "tool_use", text: "ls" })).toBe("🔧");
    expect(itemGlyph(text("안녕"))).toBe("🙋");
    expect(itemGlyph(text("네", "assistant"))).toBe("🤖");
  });
});

describe("알림 분류", () => {
  const notif = (source: string): NotificationItem => ({
    id: "n1",
    agentId: "a1",
    sessionId: "s1",
    message: "계속할까요?",
    at: 1,
    source,
  });

  it("hook만 확인 요청 카드다(stop/bell은 라인)", () => {
    expect(isQuestion(notif("hook"))).toBe(true);
    expect(isQuestion(notif("stop"))).toBe(false);
    expect(isQuestion(notif("bell"))).toBe(false);
  });
});

describe("activityLine", () => {
  it("턴 시작·도구 하트비트를 진행 라인으로", () => {
    expect(activityLine({ kind: "prompt" })?.text).toBe("⏳ 작업 중");
    expect(activityLine({ kind: "tool", text: "Bash: npm test" })?.text).toBe(
      "⏳ 작업 중 · 🔧 Bash: npm test"
    );
    expect(activityLine({ kind: "tool" })?.text).toBe("⏳ 작업 중");
    expect(activityLine({ kind: "resume" })?.text).toBe("⏳ 작업 중");
  });

  it("미니 캐릭터 전용 신호는 채팅에서 무시한다", () => {
    for (const kind of ["sub-start", "sub-stop", "sub-count", "unknown"]) {
      expect(activityLine({ kind })).toBeNull();
    }
    expect(activityLine({})).toBeNull();
  });
});
