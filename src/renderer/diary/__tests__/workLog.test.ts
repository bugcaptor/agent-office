// src/renderer/diary/__tests__/workLog.test.ts
//
// 작업 로그 버퍼(append/상한/세션 필터/clear)와, taskLabels 구독 레코더의
// 멱등성(값이 실제로 바뀔 때만 append)·세션 리셋 경계·상한을 검증한다.
// store를 직접 조작해 구독을 발화시킨다(요약기 테스트와 동일 컨벤션).
import { beforeEach, describe, expect, it } from "vitest";

// 시간추적 정산에서 실 tauriApi(invoke)를 타지 않도록 모킹.
import { vi } from "vitest";
vi.mock("../../ipc/tauriApi", () => ({
  tauriApi: {
    appendSessionTurn: vi.fn(),
    setAppSettings: vi.fn().mockResolvedValue(undefined),
  },
}));

import { useAppStore } from "../../store/appStore";
import {
  FORMAT_BUDGET_CHARS,
  MAX_ITEMS_PER_SESSION,
  MAX_SESSIONS_PER_AGENT,
  WorkLog,
  formatWorkLog,
  installWorkLogRecorder,
  type WorkLogItem,
} from "../workLog";
import type { AgentTaskLabel } from "../../store/types";

function label(overrides: Partial<AgentTaskLabel> = {}): AgentTaskLabel {
  return { sessionId: "s1", ...overrides };
}

describe("WorkLog 버퍼", () => {
  it("append 후 items로 되읽는다", () => {
    const log = new WorkLog();
    log.append("a1", { at: 1, sessionId: "s1", kind: "prompt", text: "버그 고쳐줘" });
    expect(log.items("a1")).toEqual([
      { at: 1, sessionId: "s1", kind: "prompt", text: "버그 고쳐줘" },
    ]);
  });

  it("한 세션이 세션당 상한을 넘으면 그 세션의 오래된 항목부터 버린다", () => {
    const log = new WorkLog();
    for (let i = 0; i < MAX_ITEMS_PER_SESSION + 5; i++) {
      log.append("a1", { at: i, sessionId: "s1", kind: "tool", text: `t${i}` });
    }
    const items = log.items("a1");
    expect(items.length).toBe(MAX_ITEMS_PER_SESSION);
    expect(items[0].text).toBe("t5"); // 0~4가 밀려남
  });

  it("새 세션 활동이 아직 일기화 안 된 옛 세션 항목을 축출하지 않는다(#75)", () => {
    const log = new WorkLog();
    // 세션 A: 3항목(일기 자격). 이후 세션 B가 세션당 상한을 넘겨 쏟아부어도
    // 예전(에이전트 단위 상한) 동작이라면 A가 밀려났겠지만, 세션 인지형은 A를 보존.
    log.append("a1", { at: 1, sessionId: "A", kind: "prompt", text: "A1" });
    log.append("a1", { at: 2, sessionId: "A", kind: "tool", text: "A2" });
    log.append("a1", { at: 3, sessionId: "A", kind: "narration", text: "A3" });
    for (let i = 0; i < MAX_ITEMS_PER_SESSION + 10; i++) {
      log.append("a1", { at: 100 + i, sessionId: "B", kind: "tool", text: `B${i}` });
    }
    // A의 세 항목은 그대로 남아 있어야 한다.
    expect(log.items("a1", "A").map((i) => i.text)).toEqual(["A1", "A2", "A3"]);
    // B는 세션당 상한까지만.
    expect(log.items("a1", "B").length).toBe(MAX_ITEMS_PER_SESSION);
  });

  it("세션 개수가 상한을 넘으면 가장 오래된 세션을 통째로 버린다(#75)", () => {
    const log = new WorkLog();
    // MAX_SESSIONS_PER_AGENT + 2개의 서로 다른 세션을 각 1항목씩 순서대로.
    const total = MAX_SESSIONS_PER_AGENT + 2;
    for (let i = 0; i < total; i++) {
      log.append("a1", { at: i, sessionId: `s${i}`, kind: "tool", text: `t${i}` });
    }
    const kept = log.sessions("a1");
    expect(kept.length).toBe(MAX_SESSIONS_PER_AGENT);
    // 가장 오래된 두 세션(s0, s1)이 통째로 빠지고 최신 세션들만 남는다.
    expect(kept).not.toContain("s0");
    expect(kept).not.toContain("s1");
    expect(kept[0]).toBe("s2");
    expect(kept[kept.length - 1]).toBe(`s${total - 1}`);
  });

  it("sessionId로 필터하고, clear(sessionId)는 그 세션만 지운다", () => {
    const log = new WorkLog();
    log.append("a1", { at: 1, sessionId: "s1", kind: "prompt", text: "A" });
    log.append("a1", { at: 2, sessionId: "s2", kind: "prompt", text: "B" });
    expect(log.items("a1", "s2").map((i) => i.text)).toEqual(["B"]);
    log.clear("a1", "s1");
    expect(log.items("a1").map((i) => i.text)).toEqual(["B"]);
  });

  it("에이전트 간 격리된다", () => {
    const log = new WorkLog();
    log.append("a1", { at: 1, sessionId: "s1", kind: "prompt", text: "A" });
    log.append("a2", { at: 1, sessionId: "s1", kind: "prompt", text: "B" });
    expect(log.items("a1").map((i) => i.text)).toEqual(["A"]);
    expect(log.items("a2").map((i) => i.text)).toEqual(["B"]);
  });
});

describe("formatWorkLog", () => {
  it("종류 접두어와 목표를 붙여 사람이 읽는 텍스트로 만든다", () => {
    const items: WorkLogItem[] = [
      { at: 1, sessionId: "s1", kind: "prompt", text: "이슈 56 해줘", goal: "일기 기능" },
      { at: 2, sessionId: "s1", kind: "tool", text: "Bash: cargo test" },
      { at: 3, sessionId: "s1", kind: "narration", text: "테스트를 추가했다" },
    ];
    expect(formatWorkLog(items)).toBe(
      "- [지시] 이슈 56 해줘 (목표: 일기 기능)\n- [도구] Bash: cargo test\n- [진행] 테스트를 추가했다",
    );
  });

  describe("우선순위 스마트 축소(#66)", () => {
    // 항목 하나가 예산의 60%를 먹도록 큰 텍스트 — 둘은 함께 못 들어간다.
    const big = (marker: string) => marker + "x".repeat(Math.floor(FORMAT_BUDGET_CHARS * 0.6));

    it("예산 이내면 무손실로 전부 잇는다(기존 동등)", () => {
      const items: WorkLogItem[] = [
        { at: 1, sessionId: "s", kind: "prompt", text: "짧은 지시" },
        { at: 2, sessionId: "s", kind: "tool", text: "짧은 도구" },
      ];
      expect(formatWorkLog(items)).toBe("- [지시] 짧은 지시\n- [도구] 짧은 도구");
    });

    it("예산 초과 시 prompt(+목표)는 전량 보존한다", () => {
      const items: WorkLogItem[] = [
        { at: 1, sessionId: "s", kind: "prompt", text: "첫 지시", goal: "핵심 목표" },
        { at: 2, sessionId: "s", kind: "tool", text: big("T1") },
        { at: 3, sessionId: "s", kind: "tool", text: big("T2") },
        { at: 4, sessionId: "s", kind: "prompt", text: "둘째 지시" },
      ];
      const out = formatWorkLog(items);
      expect(out).toContain("- [지시] 첫 지시 (목표: 핵심 목표)");
      expect(out).toContain("- [지시] 둘째 지시");
    });

    it("남은 예산은 tool/narration을 최신 우선으로 채우되 출력은 시간순", () => {
      const items: WorkLogItem[] = [
        { at: 1, sessionId: "s", kind: "prompt", text: "지시" },
        { at: 2, sessionId: "s", kind: "tool", text: big("OLD") }, // 가장 과거 — 탈락 예상
        { at: 3, sessionId: "s", kind: "tool", text: big("NEW") }, // 최신 — 우선 보존
      ];
      const out = formatWorkLog(items);
      expect(out).toContain("NEW"); // 최신은 남는다
      expect(out).not.toContain("OLD"); // 과거는 예산 밖으로 탈락
      // 중략 표시가 탈락 위치(지시와 NEW 사이)에 들어간다.
      expect(out).toContain("(중략: 1개 항목)");
      const lines = out.split("\n");
      expect(lines[0]).toBe("- [지시] 지시"); // 시간순: 지시가 먼저
    });

    it("탈락 항목은 중략 한 줄로 접는다", () => {
      const items: WorkLogItem[] = [
        { at: 1, sessionId: "s", kind: "prompt", text: "지시" },
        { at: 2, sessionId: "s", kind: "tool", text: big("A") },
        { at: 3, sessionId: "s", kind: "tool", text: big("B") },
        { at: 4, sessionId: "s", kind: "tool", text: big("C") },
      ];
      const out = formatWorkLog(items);
      // A·B·C 중 예산상 일부만 남고 나머지는 하나의 중략 줄로.
      expect(out).toMatch(/\(중략: \d+개 항목\)/);
    });
  });
});

describe("installWorkLogRecorder", () => {
  let off: (() => void) | null = null;
  let target: WorkLog;

  beforeEach(() => {
    off?.();
    off = null;
    useAppStore.setState({ taskLabels: {}, timeTracking: {} });
    target = new WorkLog();
    let t = 0;
    off = installWorkLogRecorder({ now: () => ++t, target });
  });

  it("새 프롬프트가 오면 목표와 함께 prompt 항목을 append한다", () => {
    useAppStore.setState({
      taskLabels: { a1: label({ latestPromptText: "버그 고쳐줘", latestPromptAt: 100, goal: "버그 수정" }) },
    });
    const items = target.items("a1");
    expect(items).toHaveLength(1);
    expect(items[0]).toMatchObject({ kind: "prompt", text: "버그 고쳐줘", goal: "버그 수정" });
  });

  it("같은 프롬프트가 반복 발화돼도 한 번만 기록한다(멱등)", () => {
    const l = label({ latestPromptText: "고쳐줘", latestPromptAt: 100 });
    useAppStore.setState({ taskLabels: { a1: l } });
    // 도구 요약만 갱신 — 프롬프트는 그대로.
    useAppStore.setState({ taskLabels: { a1: { ...l, latestToolText: "Bash: ls" } } });
    const prompts = target.items("a1").filter((i) => i.kind === "prompt");
    expect(prompts).toHaveLength(1);
  });

  it("도구/내레이션은 값이 바뀔 때마다 append한다", () => {
    const base = label({ latestPromptText: "가", latestPromptAt: 1 });
    useAppStore.setState({ taskLabels: { a1: { ...base, latestToolText: "Bash: a" } } });
    useAppStore.setState({ taskLabels: { a1: { ...base, latestToolText: "Bash: b" } } });
    useAppStore.setState({ taskLabels: { a1: { ...base, latestAssistantText: "진행했다" } } });
    const kinds = target.items("a1").map((i) => `${i.kind}:${i.text}`);
    expect(kinds).toContain("tool:Bash: a");
    expect(kinds).toContain("tool:Bash: b");
    expect(kinds).toContain("narration:진행했다");
  });

  it("세션이 바뀌면 새 세션의 첫 프롬프트도 기록한다(경계 리셋)", () => {
    useAppStore.setState({
      taskLabels: { a1: label({ sessionId: "s1", latestPromptText: "첫", latestPromptAt: 1 }) },
    });
    // 새 세션은 latestPromptAt가 1로 겹쳐도 sessionId가 달라 새로 기록돼야 한다.
    useAppStore.setState({
      taskLabels: { a1: label({ sessionId: "s2", latestPromptText: "둘", latestPromptAt: 1 }) },
    });
    const prompts = target.items("a1").filter((i) => i.kind === "prompt");
    expect(prompts.map((i) => i.text)).toEqual(["첫", "둘"]);
    expect(target.items("a1", "s2").map((i) => i.text)).toEqual(["둘"]);
  });
});
