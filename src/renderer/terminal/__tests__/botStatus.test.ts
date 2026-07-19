// 봇 상태 문구·맨 셸 가드 헬퍼 단위 테스트(이슈 #57 후속).
import { describe, expect, it } from "vitest";
import { botStatusText, nextPollSeconds } from "../botStatusText";
import { looksLikeAgentRunning } from "../botGuard";
import type { BotAgentStatus } from "@shared/types";

function st(over: Partial<BotAgentStatus>): BotAgentStatus {
  return { running: true, phase: "watching", pollIntervalSec: 60, ...over };
}

describe("botStatusText", () => {
  it("error가 있으면 phase와 무관하게 오류 문구", () => {
    const t = botStatusText(st({ phase: "working", error: "tea 미로그인" }));
    expect(t.icon).toBe("⚠️");
    expect(t.title).toBe("봇 오류");
    expect(t.detail).toBe("tea 미로그인");
  });

  it("working은 이슈 번호를 제목에 넣는다", () => {
    expect(botStatusText(st({ phase: "working", issue: 12 })).title).toBe(
      "이슈 #12 처리 중"
    );
  });

  it("watching은 slug 명령 대기 문구", () => {
    expect(botStatusText(st({ phase: "watching", slug: "nova" })).detail).toBe(
      "/nova 명령을 기다리는 중"
    );
  });

  it("starting은 시작 중 문구", () => {
    expect(botStatusText(st({ phase: "starting" })).title).toBe("봇 시작 중…");
  });
});

describe("nextPollSeconds", () => {
  it("첫 폴링 전(lastPollAtMs 없음)이면 undefined", () => {
    expect(nextPollSeconds(st({}), 1000)).toBeUndefined();
  });

  it("다음 폴링까지 남은 초를 반올림해 반환", () => {
    const s = st({ lastPollAtMs: 10_000, pollIntervalSec: 60 });
    expect(nextPollSeconds(s, 10_000)).toBe(60);
    expect(nextPollSeconds(s, 40_000)).toBe(30);
  });

  it("주기를 지났으면 0(음수 아님)", () => {
    const s = st({ lastPollAtMs: 10_000, pollIntervalSec: 60 });
    expect(nextPollSeconds(s, 999_999)).toBe(0);
  });
});

describe("looksLikeAgentRunning", () => {
  it("버퍼가 없으면 false", () => {
    expect(looksLikeAgentRunning(undefined)).toBe(false);
    expect(looksLikeAgentRunning("")).toBe(false);
  });

  it("맨 셸 프롬프트로 보이면 false", () => {
    expect(looksLikeAgentRunning("bugcaptor@mac ~/dev/agent-office %  ")).toBe(false);
  });

  it("claude TUI 시그니처가 꼬리에 있으면 true", () => {
    expect(looksLikeAgentRunning("...\n? for shortcuts")).toBe(true);
    expect(looksLikeAgentRunning("working... esc to interrupt")).toBe(true);
  });

  it("스크롤백 앞쪽에만 있고 꼬리가 맨 셸이면(1500자 밖) false", () => {
    const stale = "claude code welcome\n" + "x".repeat(1600) + "\nuser@host % ";
    expect(looksLikeAgentRunning(stale)).toBe(false);
  });
});
