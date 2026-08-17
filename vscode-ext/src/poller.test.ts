import { describe, expect, it, vi } from "vitest";

import type { ControlResponse, ListEntry, NotificationEntry } from "./controlClient";
import {
  BACKGROUND_INTERVAL_MS,
  Poller,
  parseProfilesJson,
  type ControlLike,
  type PollerDeps,
  type Snapshot,
} from "./poller";

const PROFILES = JSON.stringify({
  version: 1,
  agents: [
    { id: "a1", name: "에이다", role: "임베디드", cwd: "/dev/x" },
    { id: "a2", name: "밥", role: "" },
  ],
});

function notification(id: string, at: number): NotificationEntry {
  return {
    id,
    sessionId: "s1",
    agentId: "a1",
    source: "hook",
    message: "확인이 필요합니다",
    dedupKey: "k",
    at,
  };
}

/** 즉시 실행하지 않는 수동 타이머 — 틱을 테스트가 직접 돌린다. */
function manualTimers() {
  const queue: Array<() => void> = [];
  return {
    timers: {
      setTimeout: (fn: () => void) => {
        queue.push(fn);
        return queue.length;
      },
      clearTimeout: () => {
        queue.length = 0;
      },
    },
    queue,
  };
}

function makeDeps(client: ControlLike, over: Partial<PollerDeps> = {}): PollerDeps {
  return {
    resolveAppDataDir: () => "/app-data",
    createClient: () => client,
    readProfiles: async () => PROFILES,
    logRootExists: async () => true,
    pollIntervalMs: () => 2000,
    isFocused: () => true,
    now: () => 1_000,
    ...over,
  };
}

async function collectOnce(deps: PollerDeps): Promise<Snapshot> {
  const { timers } = manualTimers();
  const poller = new Poller({ ...deps, timers });
  const seen: Snapshot[] = [];
  poller.onSnapshot((s) => seen.push(s));
  // refresh()는 한 틱을 끝까지 기다려 준다(타이머 없이 결정적).
  await poller.refresh();
  poller.dispose();
  expect(seen).toHaveLength(1);
  return seen[0];
}

describe("parseProfilesJson", () => {
  it("version 1만 읽는다", () => {
    expect(parseProfilesJson(PROFILES).map((p) => p.id)).toEqual(["a1", "a2"]);
    expect(parseProfilesJson(JSON.stringify({ version: 2, agents: [{ id: "a1" }] }))).toEqual([]);
    expect(parseProfilesJson("{ 깨진 json")).toEqual([]);
    expect(parseProfilesJson(undefined)).toEqual([]);
  });

  it("name이 없으면 id로 대신한다", () => {
    const parsed = parseProfilesJson(JSON.stringify({ version: 1, agents: [{ id: "a9" }] }));
    expect(parsed).toEqual([{ id: "a9", name: "a9", role: "", cwd: undefined }]);
  });
});

describe("Poller", () => {
  it("list + 각 캐릭터 notifications를 한 스냅샷으로 모은다", async () => {
    const rows: ListEntry[] = [
      { agentId: "a1", name: "에이다", role: "임베디드", state: "running", sessionId: "s1" },
      { agentId: "a2", name: "밥", role: "리뷰" },
    ];
    const client: ControlLike = {
      list: async () => ({ status: "ok", data: rows }),
      notifications: async (agentId) => ({
        status: "ok",
        data: agentId === "a1" ? [notification("n1", 500)] : [],
      }),
    };
    const snapshot = await collectOnce(makeDeps(client));
    expect(snapshot.status).toBe("ok");
    expect(snapshot.entries.map((e) => e.agentId)).toEqual(["a1", "a2"]);
    expect(snapshot.entries[0].notifications.map((n) => n.id)).toEqual(["n1"]);
    expect(snapshot.entries[0].state).toBe("running");
    expect(snapshot.entries[1].notifications).toEqual([]);
    expect(snapshot.logRootMissing).toBe(false);
  });

  it("연결 실패면 profiles.json 폴백 + 사유를 담는다", async () => {
    const client: ControlLike = {
      list: async () => ({ status: "unreachable" }),
      notifications: async () => ({ status: "unreachable" }),
    };
    const snapshot = await collectOnce(makeDeps(client));
    expect(snapshot.status).toBe("unreachable");
    expect(snapshot.message).toContain("앱이 꺼져 있습니다");
    expect(snapshot.entries.map((e) => e.name)).toEqual(["에이다", "밥"]);
    expect(snapshot.entries.every((e) => e.notifications.length === 0)).toBe(true);
  });

  it("app_data가 없으면 폴백도 없고 로그 루트도 없음으로 본다", async () => {
    const client: ControlLike = {
      list: async () => ({ status: "no-app-data" }),
      notifications: async () => ({ status: "no-app-data" }),
    };
    const readProfiles = vi.fn(async () => PROFILES);
    const snapshot = await collectOnce(
      makeDeps(client, { resolveAppDataDir: () => undefined, readProfiles }),
    );
    expect(snapshot.entries).toEqual([]);
    expect(snapshot.logRootMissing).toBe(true);
    expect(readProfiles).not.toHaveBeenCalled();
  });

  it("서버가 거절하면(ok:false) 그 메시지를 그대로 보여 준다", async () => {
    const rejected: ControlResponse<ListEntry[]> = { status: "error", message: "거절" };
    const client: ControlLike = {
      list: async () => rejected,
      notifications: async () => ({ status: "error", message: "거절" }),
    };
    const snapshot = await collectOnce(makeDeps(client));
    expect(snapshot.status).toBe("error");
    expect(snapshot.message).toBe("거절");
  });

  it("간격: 포커스 중엔 설정값, 백그라운드·unreachable이면 10초로 늘린다", async () => {
    const okClient: ControlLike = {
      list: async () => ({ status: "ok", data: [] }),
      notifications: async () => ({ status: "ok", data: [] }),
    };
    const { timers } = manualTimers();

    const focused = new Poller({ ...makeDeps(okClient), timers });
    expect(focused.nextDelayMs()).toBe(2000);

    const background = new Poller({
      ...makeDeps(okClient, { isFocused: () => false }),
      timers,
    });
    expect(background.nextDelayMs()).toBe(BACKGROUND_INTERVAL_MS);

    // 설정이 최소값보다 작아도 500ms 아래로는 내려가지 않는다.
    const tooFast = new Poller({
      ...makeDeps(okClient, { pollIntervalMs: () => 10 }),
      timers,
    });
    expect(tooFast.nextDelayMs()).toBe(500);

    // unreachable을 한 번 겪으면 다음 틱부터 백오프.
    const downClient: ControlLike = {
      list: async () => ({ status: "unreachable" }),
      notifications: async () => ({ status: "unreachable" }),
    };
    const down = new Poller({ ...makeDeps(downClient), timers });
    expect(down.nextDelayMs()).toBe(2000);
    await down.refresh();
    expect(down.nextDelayMs()).toBe(BACKGROUND_INTERVAL_MS);
  });
});
