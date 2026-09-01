// src/renderer/agent/__tests__/summonToDesk.test.ts
//
// "전체 자리로"(summonAllToDesk)와 그 짝인 소환 직후 알림 억제 창
// (summonSuppress)의 TDD. clockOut.test.ts와 같은 방식으로 sessionBridge는
// 모듈 모킹하고(실제 IPC/PTY는 범위 밖), 스토어는 진짜 useAppStore를 쓴다.
import { beforeEach, describe, expect, it, vi } from "vitest";
import { useAppStore } from "../../store/appStore";
import type { AgentProfile } from "../../store/types";
import { awayFromDeskIds } from "../../store/selectors";

const ensureSession = vi.fn();
vi.mock("../../ipc/sessionBridge", () => ({
  ensureSession: (...args: unknown[]) => ensureSession(...args),
}));

const { summonAllToDesk } = await import("../summonToDesk");
const {
  SUMMON_NOTIFY_SUPPRESS_MS,
  isNotifySuppressed,
  resetNotifySuppression,
  suppressNotifications,
} = await import("../summonSuppress");

function mkProfile(id: string): AgentProfile {
  return { id, name: `Agent ${id}`, role: "eng", seed: id, createdAt: Date.now(), deskIndex: 0 };
}

const initialState = useAppStore.getState();

beforeEach(() => {
  useAppStore.setState(initialState, true);
  ensureSession.mockClear();
  resetNotifySuppression();
});

describe("awayFromDeskIds", () => {
  it("출근했고 세션이 없는(엔트리 없음/idle/exited) 에이전트만 고른다", () => {
    const s = useAppStore.getState();
    s.addAgent(mkProfile("none"));
    s.addAgent(mkProfile("idle"));
    s.setSessionState({ agentId: "idle", status: "idle" });
    s.addAgent(mkProfile("exited"));
    s.setSessionState({ agentId: "exited", status: "exited" });
    s.addAgent(mkProfile("running"));
    s.setSessionState({ agentId: "running", status: "running" });
    s.addAgent(mkProfile("starting"));
    s.setSessionState({ agentId: "starting", status: "starting" });
    // addAgent가 세션 엔트리를 status:"starting"으로 심으므로, "엔트리 없음"
    // 케이스는 직접 지워서 만든다(부팅 직후 hydrate 상태와 같은 모양).
    const { none: _dropped, ...rest } = useAppStore.getState().sessions;
    useAppStore.setState({ sessions: rest });

    expect(awayFromDeskIds(useAppStore.getState())).toEqual(["none", "idle", "exited"]);
  });

  it("퇴근한 에이전트는 세션이 없어도 제외한다(캔버스에 아예 없다)", () => {
    const s = useAppStore.getState();
    s.addAgent(mkProfile("a1"));
    s.setSessionState({ agentId: "a1", status: "exited" });
    s.addAgent(mkProfile("a2"));
    s.clockOut("a2");

    expect(awayFromDeskIds(useAppStore.getState())).toEqual(["a1"]);
  });
});

describe("summonAllToDesk", () => {
  it("탕비실에 있는 전원에게 ensureSession을 부르고, 자리에 앉은 사람은 건드리지 않는다", () => {
    const s = useAppStore.getState();
    s.addAgent(mkProfile("a1"));
    s.setSessionState({ agentId: "a1", status: "idle" });
    s.addAgent(mkProfile("a2"));
    s.setSessionState({ agentId: "a2", status: "running" });
    s.addAgent(mkProfile("a3"));
    s.setSessionState({ agentId: "a3", status: "exited" });

    summonAllToDesk();

    expect(ensureSession.mock.calls.map((c) => c[0])).toEqual(["a1", "a3"]);
  });

  it("소환한 사람마다 알림 억제 창을 연다", () => {
    const s = useAppStore.getState();
    s.addAgent(mkProfile("a1"));
    s.setSessionState({ agentId: "a1", status: "exited" });
    s.addAgent(mkProfile("a2"));
    s.setSessionState({ agentId: "a2", status: "running" });

    summonAllToDesk();

    expect(isNotifySuppressed("a1")).toBe(true);
    expect(isNotifySuppressed("a2")).toBe(false);
  });

  it("대상이 없으면 아무것도 하지 않는다", () => {
    summonAllToDesk();
    expect(ensureSession).not.toHaveBeenCalled();
  });
});

describe("summonSuppress", () => {
  it("창 안에서는 억제, 만료 후에는 해제된다", () => {
    suppressNotifications("a1", SUMMON_NOTIFY_SUPPRESS_MS, 1000);

    expect(isNotifySuppressed("a1", 1000)).toBe(true);
    expect(isNotifySuppressed("a1", 1000 + SUMMON_NOTIFY_SUPPRESS_MS - 1)).toBe(true);
    expect(isNotifySuppressed("a1", 1000 + SUMMON_NOTIFY_SUPPRESS_MS)).toBe(false);
  });

  it("억제한 적 없는 에이전트는 항상 통과", () => {
    expect(isNotifySuppressed("nobody")).toBe(false);
  });

  it("연달아 소환해도 남은 창이 더 길면 줄어들지 않는다", () => {
    suppressNotifications("a1", 5000, 1000); // 6000까지
    suppressNotifications("a1", 1000, 2000); // 3000까지 — 무시되어야 한다

    expect(isNotifySuppressed("a1", 5500)).toBe(true);
  });

  it("기본 억제 시간은 3초다", () => {
    expect(SUMMON_NOTIFY_SUPPRESS_MS).toBe(3000);
  });
});
