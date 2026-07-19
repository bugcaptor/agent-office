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
  MAX_ITEMS_PER_AGENT,
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

  it("상한을 넘으면 오래된 항목부터 버린다", () => {
    const log = new WorkLog();
    for (let i = 0; i < MAX_ITEMS_PER_AGENT + 5; i++) {
      log.append("a1", { at: i, sessionId: "s1", kind: "tool", text: `t${i}` });
    }
    const items = log.items("a1");
    expect(items.length).toBe(MAX_ITEMS_PER_AGENT);
    expect(items[0].text).toBe("t5"); // 0~4가 밀려남
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
