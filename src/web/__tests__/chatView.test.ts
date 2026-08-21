// src/web/__tests__/chatView.test.ts
//
// 채팅 뷰의 순수 규칙. DOM 없이 도는 것들만 여기 있다(렌더는 ChatScreen.test).

import { describe, expect, it } from "vitest";
import {
  COLLAPSE_CHARS,
  MAX_ECHOES,
  MAX_ITEMS,
  PREVIEW_LINES,
  activityLine,
  applyChatFrame,
  dedupEchoes,
  isAtBottom,
  isLongText,
  isQuestion,
  itemGlyph,
  previewText,
  promptEcho,
  pushEcho,
  removeEcho,
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

describe("호스트 입력 에코", () => {
  it("kind=prompt의 원문만 에코 후보다", () => {
    expect(promptEcho({ kind: "prompt", text: "  로그를 고쳐줘  " })).toBe("로그를 고쳐줘");
    // 훅 body에 프롬프트가 없으면 진행 라인만 뜨고 에코는 없다.
    expect(promptEcho({ kind: "prompt" })).toBeNull();
    expect(promptEcho({ kind: "prompt", text: "" })).toBeNull();
    expect(promptEcho({ kind: "prompt", text: "   " })).toBeNull();
    expect(promptEcho({ kind: "prompt", text: null })).toBeNull();
    // 다른 활동 신호는 에코가 아니다.
    for (const kind of ["tool", "resume", "sub-start", undefined]) {
      expect(promptEcho({ kind, text: "무언가" })).toBeNull();
    }
  });

  it("같은 문장은 한 번만 쌓인다 — 웹 낙관 에코와 호스트 미러가 겹친다", () => {
    const once = pushEcho([], "테스트를 돌려줘");
    expect(once).toEqual(["테스트를 돌려줘"]);
    // 정규화(trim) 기준으로 같으면 추가하지 않는다.
    expect(pushEcho(once, "  테스트를 돌려줘 ")).toBe(once);
    expect(pushEcho(once, "다른 말")).toEqual(["테스트를 돌려줘", "다른 말"]);
    // 빈 문장은 애초에 에코가 아니다.
    expect(pushEcho(once, "   ")).toBe(once);
  });

  it("상한을 넘으면 오래된 에코부터 버린다", () => {
    let list: string[] = [];
    for (let i = 0; i < MAX_ECHOES + 3; i += 1) list = pushEcho(list, `m${i}`);
    expect(list).toHaveLength(MAX_ECHOES);
    expect(list[0]).toBe("m3");
  });

  it("전송 실패는 에코를 되돌린다", () => {
    const list = pushEcho(pushEcho([], "가"), "나");
    expect(removeEcho(list, " 가 ")).toEqual(["나"]);
    // 없는 문장은 목록을 건드리지 않는다(참조 유지).
    expect(removeEcho(list, "다")).toBe(list);
  });

  it("전사에 나타난 유저 발화만큼만 소거한다", () => {
    const echoes = ["첫 말", "둘째 말"];
    const next = dedupEchoes(echoes, [
      text("첫 말"),
      text("에이전트 답", "assistant"),
      { role: "user", kind: "tool_result", text: "둘째 말" },
    ]);
    // tool_result는 유저 발화가 아니다 — "둘째 말"은 남는다.
    expect(next).toEqual(["둘째 말"]);

    // 매칭이 없으면 참조를 유지한다(불필요한 재렌더 방지).
    expect(dedupEchoes(echoes, [text("전혀 다른 말")])).toBe(echoes);
    expect(dedupEchoes([], [text("첫 말")])).toEqual([]);
  });

  it("같은 문장이 두 번 나오면 에코도 하나씩만 소거된다", () => {
    // pushEcho는 중복을 막지만 백필 전사에는 같은 문장이 여러 번 있을 수 있다.
    const echoes = ["반복"];
    expect(dedupEchoes(echoes, [text("반복"), text("반복")])).toEqual([]);
  });

  it("앞뒤 공백 차이는 같은 문장으로 본다", () => {
    expect(dedupEchoes(["보고서 써줘"], [text("  보고서 써줘\n")])).toEqual([]);
  });
});

describe("긴 본문 접기", () => {
  it("문자 수·줄 수 어느 쪽이든 넘으면 접는다", () => {
    expect(isLongText("짧다")).toBe(false);
    expect(isLongText("가".repeat(COLLAPSE_CHARS + 1))).toBe(true);
    expect(isLongText(Array.from({ length: 9 }, (_, i) => `line${i}`).join("\n"))).toBe(
      true
    );
    // 기준 이하 줄 수는 그대로 편다.
    expect(isLongText(Array.from({ length: 8 }, (_, i) => `line${i}`).join("\n"))).toBe(
      false
    );
  });

  it("미리보기는 앞부분만 남기고, 짧으면 항등이다", () => {
    const short = "한 줄";
    expect(previewText(short)).toBe(short);

    const many = Array.from({ length: 40 }, (_, i) => `line${i}`).join("\n");
    const head = previewText(many);
    expect(head.split("\n")).toHaveLength(PREVIEW_LINES);
    expect(head.startsWith("line0")).toBe(true);
    expect(head).not.toContain("line39");

    // 개행 없는 긴 한 줄은 문자 수로 자른다.
    const wall = "가".repeat(COLLAPSE_CHARS + 200);
    expect(previewText(wall)).toHaveLength(COLLAPSE_CHARS);
  });
});
