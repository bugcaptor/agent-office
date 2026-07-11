import { describe, expect, it } from "vitest";
import { useAppStore } from "../appStore";

describe("applyActivityEvent: 서브에이전트 신호 무시", () => {
  it("sub-start/sub-stop는 timeTracking을 변경하지 않는다", () => {
    const before = useAppStore.getState().timeTracking["ag-x"];
    useAppStore.getState().applyActivityEvent({
      agentId: "ag-x",
      sessionId: "s1",
      kind: "sub-start",
      at: 1000,
    });
    const after = useAppStore.getState().timeTracking["ag-x"];
    expect(after).toBe(before); // 새 턴 상태 생성 안 됨(undefined 그대로)
  });
});
