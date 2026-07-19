// src/renderer/diary/__tests__/workLogPersister.test.ts
//
// 작업 로그 영속화(#60): WorkLog의 seed/onChange/sessions 신규 동작과,
// workLogPersister의 디바운스 저장·diaryEnabled 게이트·OFF→ON 일괄 flush·
// flushNow·부팅 복원(프루닝)을 검증한다. 디바운스는 fake timer로 몬다.
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

vi.mock("../../ipc/tauriApi", () => ({
  tauriApi: {
    saveWorkLog: vi.fn().mockResolvedValue(undefined),
    loadWorkLogs: vi.fn().mockResolvedValue({}),
    setAppSettings: vi.fn().mockResolvedValue(undefined),
  },
}));

import { useAppStore } from "../../store/appStore";
import { MAX_ITEMS_PER_AGENT, WorkLog } from "../workLog";
import {
  installWorkLogPersister,
  restoreWorkLogs,
  type WorkLogPersister,
} from "../workLogPersister";
import type { WorkLogItem } from "@shared/types";

function item(at: number, session = "s1", text = "x"): WorkLogItem {
  return { at, sessionId: session, kind: "prompt", text };
}

/** diaryEnabled를 세팅한다(다른 설정은 유지). */
function setDiary(enabled: boolean): void {
  const s = useAppStore.getState();
  useAppStore.setState({ appSettings: { ...s.appSettings, diaryEnabled: enabled } });
}

describe("WorkLog 신규 동작(#60)", () => {
  it("seed는 복원분을 앞에 붙이고 onChange를 발화하지 않는다", () => {
    const log = new WorkLog();
    const onChange = vi.fn();
    log.setOnChange(onChange);
    log.append("a1", item(10, "s2", "new"));
    onChange.mockClear();

    log.seed("a1", [item(1, "s1", "old1"), item(2, "s1", "old2")]);

    expect(log.items("a1").map((i) => i.text)).toEqual(["old1", "old2", "new"]);
    expect(onChange).not.toHaveBeenCalled();
  });

  it("seed도 상한을 지킨다(오래된 것부터 버림)", () => {
    const log = new WorkLog();
    const restored = Array.from({ length: MAX_ITEMS_PER_AGENT + 3 }, (_, i) => item(i, "s1", `r${i}`));
    log.seed("a1", restored);
    expect(log.items("a1")).toHaveLength(MAX_ITEMS_PER_AGENT);
    expect(log.items("a1")[0].text).toBe("r3");
  });

  it("append/clear는 실제 변화가 있을 때만 onChange를 발화한다", () => {
    const log = new WorkLog();
    const onChange = vi.fn();
    log.setOnChange(onChange);

    log.append("a1", item(1, "s1"));
    expect(onChange).toHaveBeenCalledTimes(1);

    log.clear("a1", "nope"); // 없는 세션 — 변화 없음
    expect(onChange).toHaveBeenCalledTimes(1);

    log.clear("a1", "s1"); // 실제 삭제
    expect(onChange).toHaveBeenCalledTimes(2);
  });

  it("sessions는 등장순 유니크 세션 목록을 준다", () => {
    const log = new WorkLog();
    log.append("a1", item(1, "s1"));
    log.append("a1", item(2, "s2"));
    log.append("a1", item(3, "s1"));
    expect(log.sessions("a1")).toEqual(["s1", "s2"]);
  });
});

describe("installWorkLogPersister", () => {
  let persister: WorkLogPersister | null = null;
  let log: WorkLog;
  let save: ReturnType<typeof vi.fn>;

  beforeEach(() => {
    vi.useFakeTimers();
    log = new WorkLog();
    save = vi.fn().mockResolvedValue(undefined);
    setDiary(true);
  });

  afterEach(() => {
    persister?.dispose();
    persister = null;
    vi.useRealTimers();
  });

  it("ON이면 변경 후 디바운스가 지나면 스냅샷을 저장한다", async () => {
    persister = installWorkLogPersister({ log, save, debounceMs: 1000 });
    log.append("a1", item(1, "s1", "hello"));

    expect(save).not.toHaveBeenCalled(); // 아직 디바운스 전
    await vi.advanceTimersByTimeAsync(1000);

    expect(save).toHaveBeenCalledWith("a1", [item(1, "s1", "hello")]);
  });

  it("디바운스 창 안의 연속 변경은 한 번만 저장한다(최신 상태로)", async () => {
    persister = installWorkLogPersister({ log, save, debounceMs: 1000 });
    log.append("a1", item(1, "s1", "a"));
    await vi.advanceTimersByTimeAsync(500);
    log.append("a1", item(2, "s1", "b"));
    await vi.advanceTimersByTimeAsync(1000);

    expect(save).toHaveBeenCalledTimes(1);
    expect(save).toHaveBeenCalledWith("a1", [item(1, "s1", "a"), item(2, "s1", "b")]);
  });

  it("OFF면 저장하지 않고, OFF→ON 전환 시 밀린 변경을 일괄 저장한다", async () => {
    setDiary(false);
    persister = installWorkLogPersister({ log, save, debounceMs: 1000 });
    log.append("a1", item(1, "s1", "off-write"));
    await vi.advanceTimersByTimeAsync(2000);
    expect(save).not.toHaveBeenCalled();

    setDiary(true); // 전환 → dirtyWhileOff flush
    await vi.advanceTimersByTimeAsync(1000);
    expect(save).toHaveBeenCalledWith("a1", [item(1, "s1", "off-write")]);
  });

  it("flushNow는 펜딩 저장을 즉시 수행한다", async () => {
    persister = installWorkLogPersister({ log, save, debounceMs: 10_000 });
    log.append("a1", item(1, "s1", "urgent"));
    expect(save).not.toHaveBeenCalled();

    await persister.flushNow();
    expect(save).toHaveBeenCalledWith("a1", [item(1, "s1", "urgent")]);
  });

  it("일기화로 비워지면(clear 전체) 빈 스냅샷으로 저장(백엔드가 파일 삭제)", async () => {
    persister = installWorkLogPersister({ log, save, debounceMs: 1000 });
    log.append("a1", item(1, "s1"));
    await vi.advanceTimersByTimeAsync(1000);
    save.mockClear();

    log.clear("a1"); // 전체 소진
    await vi.advanceTimersByTimeAsync(1000);
    expect(save).toHaveBeenCalledWith("a1", []);
  });
});

describe("restoreWorkLogs", () => {
  beforeEach(() => setDiary(true));

  it("스냅샷을 버퍼로 복원하고, 오래된 항목은 프루닝한다", async () => {
    const log = new WorkLog();
    const now = 100_000_000;
    const save = vi.fn().mockResolvedValue(undefined);
    const loadAll = vi.fn().mockResolvedValue({
      a1: [item(now - 1000, "s1", "fresh"), item(now - 999_999_999, "s1", "stale")],
    });

    await restoreWorkLogs({ log, loadAll, save, now: () => now, maxAgeMs: 1_000_000 });

    expect(log.items("a1").map((i) => i.text)).toEqual(["fresh"]);
    // 프루닝으로 줄었으니 디스크도 정리(ON).
    expect(save).toHaveBeenCalledWith("a1", [item(now - 1000, "s1", "fresh")]);
  });

  it("OFF면 복원은 하되 디스크 프루닝은 하지 않는다", async () => {
    setDiary(false);
    const log = new WorkLog();
    const now = 100_000_000;
    const save = vi.fn().mockResolvedValue(undefined);
    const loadAll = vi.fn().mockResolvedValue({
      a1: [item(now - 1000, "s1", "fresh"), item(now - 999_999_999, "s1", "stale")],
    });

    await restoreWorkLogs({ log, loadAll, save, now: () => now, maxAgeMs: 1_000_000 });

    expect(log.items("a1").map((i) => i.text)).toEqual(["fresh"]);
    expect(save).not.toHaveBeenCalled();
  });

  it("로드 실패는 조용히 넘어간다(빈 복원)", async () => {
    const log = new WorkLog();
    const loadAll = vi.fn().mockRejectedValue(new Error("io"));
    await restoreWorkLogs({ log, loadAll });
    expect(log.items("a1")).toEqual([]);
  });
});
