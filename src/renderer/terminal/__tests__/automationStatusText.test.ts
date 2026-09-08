// 자동화 점검(kbm) 상태 문구 단위 테스트. botStatus.test.ts를 본떴다.
import { describe, expect, it } from "vitest";
import { automationStatusText } from "../automationStatusText";
import type { AutomationAgentStatus } from "@shared/types";

function st(over: Partial<AutomationAgentStatus>): AutomationAgentStatus {
  return {
    running: true,
    phase: "watching",
    cli: "claude",
    filePath: "/tmp/agent-office-automation.md",
    startedAtMs: 0,
    ...over,
  };
}

describe("automationStatusText", () => {
  it("failed phase는 실패 문구", () => {
    const t = automationStatusText(st({ running: false, phase: "failed" }));
    expect(t.icon).toBe("⚠️");
    expect(t.title).toBe("자동화 실패");
  });

  it("error가 있으면 phase와 무관하게 오류 상세를 보여준다", () => {
    const t = automationStatusText(
      st({ running: false, phase: "failed", error: "automation-session-lost" })
    );
    expect(t.detail).toBe("작업 도중 세션이 끊겼습니다");
  });

  it("사용자 중단은 실패가 아니라 중단 문구로 보인다", () => {
    const t = automationStatusText(st({ running: false, phase: "cancelled" }));
    expect(t.icon).toBe("⏹️");
    expect(t.title).toBe("자동화 중단됨");
    expect(t.detail).toBeUndefined();
  });

  it("알 수 없는 에러 코드는 원문을 그대로 보여준다", () => {
    const t = automationStatusText(
      st({ running: false, phase: "failed", error: "some raw ipc failure" })
    );
    expect(t.detail).toBe("some raw ipc failure");
  });

  it("errorUnknown 폴백: error가 없는데 failed면 알 수 없는 오류", () => {
    const t = automationStatusText(st({ running: false, phase: "failed", error: undefined }));
    expect(t.detail).toBe("알 수 없는 오류");
  });

  it("launching은 실행 중 문구", () => {
    expect(automationStatusText(st({ phase: "launching" })).title).toBe("LLM CLI 실행 중…");
  });

  it("waitingStartup은 기동 대기 문구", () => {
    expect(automationStatusText(st({ phase: "waitingStartup" })).title).toBe("기동 대기 중…");
  });

  it("injecting은 프롬프트 넣는 중 문구", () => {
    expect(automationStatusText(st({ phase: "injecting" })).title).toBe("프롬프트 넣는 중…");
  });

  it("watching은 결과 파일 기다리는 중 문구", () => {
    expect(automationStatusText(st({ phase: "watching" })).title).toBe("결과 파일 기다리는 중…");
  });

  it("timeoutDecision은 작업 시간 초과 문구", () => {
    const t = automationStatusText(st({ phase: "timeoutDecision" }));
    expect(t.icon).toBe("⏳");
    expect(t.title).toBe("작업 시간 초과 — 계속할까요?");
  });

  it("settling은 정리 중 문구", () => {
    const t = automationStatusText(st({ phase: "settling" }));
    expect(t.icon).toBe("⏳");
    expect(t.title).toBe("완료 후 정리 중…");
  });

  it("exiting은 세션 닫는 중 문구", () => {
    expect(automationStatusText(st({ phase: "exiting" })).title).toBe("LLM 세션 닫는 중…");
  });

  it("done은 완료 문구", () => {
    const t = automationStatusText(st({ running: false, phase: "done" }));
    expect(t.icon).toBe("✅");
    expect(t.title).toBe("자동화 완료");
  });

  it("cancelling은 중단 중 문구", () => {
    const t = automationStatusText(st({ phase: "cancelling" }));
    expect(t.icon).toBe("⏳");
    expect(t.title).toBe("자동화 중단 중…");
  });

  it("pendingReason이 humanTyping이면 보류 안내가 detail로 들어간다", () => {
    const t = automationStatusText(st({ phase: "injecting", pendingReason: "humanTyping" }));
    expect(t.title).toBe("프롬프트 넣는 중…");
    expect(t.detail).toBe("자동 입력 대기 — 입력창에 안 보낸 글이 남아 있습니다");
  });

  it("pendingReason이 anotherProducer이면 다른 작업 입력 중 안내가 들어간다", () => {
    const t = automationStatusText(st({ phase: "injecting", pendingReason: "anotherProducer" }));
    expect(t.title).toBe("프롬프트 넣는 중…");
    expect(t.detail).toBe("자동 입력 대기 — 다른 작업이 입력 중입니다");
  });

  it("automation-session-changed 에러가 매핑된다", () => {
    const t = automationStatusText(
      st({ running: false, phase: "failed", error: "automation-session-changed" })
    );
    expect(t.detail).toBe("세션이 변경되어 자동화를 중단했습니다");
  });
});
