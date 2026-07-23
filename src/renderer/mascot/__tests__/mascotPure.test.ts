// 마스코트 창(이슈 #72)의 순수 파트 — 프로토콜 파서, 드래그 판정, 위치 결정,
// 리샘플 해상도 계산. DOM/Tauri 없이 검증 가능한 것만 여기에 모은다.
import { describe, expect, it } from "vitest";
import {
  HIDDEN_MASCOT_STATE,
  parseMascotState,
  sameMascotState,
  type MascotState,
} from "../protocol";
import { createDragDetector, DRAG_THRESHOLD_PX } from "../drag";
import {
  defaultPosition,
  isOnMonitor,
  MASCOT_FALLBACK_TASKBAR_INSET_PX,
  MASCOT_MARGIN_PX,
  readSavedPosition,
  resolvePosition,
  usableArea,
  writeSavedPosition,
  type MonitorRect,
} from "../position";
import { mascotDetailCell, mascotSheetDims, usesCustomSheet } from "../sheet";

const state = (patch: Partial<MascotState> = {}): MascotState => ({
  ...HIDDEN_MASCOT_STATE,
  visible: true,
  agentId: "a1",
  name: "테스터",
  seed: "seed-1",
  ...patch,
});

describe("protocol", () => {
  it("정상 페이로드를 파싱한다", () => {
    const s = state({ hasPending: true, spriteUpdatedAt: 42 });
    expect(parseMascotState(JSON.parse(JSON.stringify(s)))).toEqual(s);
  });

  it("형태가 아니면 null — 손상 페이로드에 마스코트가 깨지지 않는다", () => {
    expect(parseMascotState(null)).toBeNull();
    expect(parseMascotState("nope")).toBeNull();
    expect(parseMascotState({ agentId: "a" })).toBeNull(); // visible 없음
  });

  it("알 수 없는 타입의 필드는 null/false로 떨어뜨린다", () => {
    const parsed = parseMascotState({ visible: true, agentId: 7, hasPending: "yes" });
    expect(parsed).toMatchObject({ visible: true, agentId: null, hasPending: false });
  });

  it("sameMascotState는 한 필드만 달라도 다르다고 본다", () => {
    expect(sameMascotState(state(), state())).toBe(true);
    expect(sameMascotState(state(), state({ hasPending: true }))).toBe(false);
  });

});

describe("drag detector", () => {
  it("임계 이하 움직임은 클릭이다", () => {
    const d = createDragDetector();
    d.down(100, 100);
    expect(d.move(102, 101)).toBe("none");
    expect(d.up()).toBe("click");
  });

  it("임계를 넘으면 드래그를 시작하고 클릭은 취소된다", () => {
    const d = createDragDetector();
    d.down(100, 100);
    expect(d.move(100 + DRAG_THRESHOLD_PX + 1, 100)).toBe("start-drag");
    // 이미 드래그 중이면 다시 시작하지 않는다(중복 startDragging 방지).
    expect(d.move(200, 200)).toBe("none");
    expect(d.up()).toBe("none");
  });

  it("cancel 이후의 up은 클릭이 아니다", () => {
    const d = createDragDetector();
    d.down(0, 0);
    d.cancel();
    expect(d.up()).toBe("none");
  });
});

describe("position", () => {
  const size = { width: 120, height: 140 };
  const mon = (patch: Partial<MonitorRect> = {}): MonitorRect => ({
    x: 0,
    y: 0,
    width: 1920,
    height: 1080,
    scaleFactor: 1,
    ...patch,
  });

  it("workArea가 없으면 전체 경계에서 하단만 어림 인셋만큼 줄인다(기존 동작 보존)", () => {
    expect(defaultPosition(mon(), size)).toEqual({
      x: 1920 - 120 - MASCOT_MARGIN_PX,
      y: 1080 - 140 - MASCOT_MARGIN_PX - MASCOT_FALLBACK_TASKBAR_INSET_PX,
    });
  });

  it("배율이 2인 모니터에서는 여백도 물리 픽셀로 환산된다", () => {
    expect(defaultPosition(mon({ scaleFactor: 2 }), size)).toEqual({
      x: 1920 - 120 - MASCOT_MARGIN_PX * 2,
      y: 1080 - 140 - (MASCOT_MARGIN_PX + MASCOT_FALLBACK_TASKBAR_INSET_PX) * 2,
    });
  });

  it("workArea가 있으면 그 우하단을 쓴다 — 하단 작업표시줄(이슈 #73)", () => {
    const m = mon({ workArea: { x: 0, y: 0, width: 1920, height: 1080 - 48 } });
    expect(defaultPosition(m, size)).toEqual({
      x: 1920 - 120 - MASCOT_MARGIN_PX,
      y: 1080 - 48 - 140 - MASCOT_MARGIN_PX,
    });
  });

  it("작업표시줄이 왼쪽/위쪽이어도 작업 영역 안에 잡힌다", () => {
    // 왼쪽 72px 작업표시줄: workArea가 오른쪽으로 밀리고 폭이 줄어든다.
    const left = mon({ workArea: { x: 72, y: 0, width: 1920 - 72, height: 1080 } });
    expect(defaultPosition(left, size)).toEqual({
      x: 1920 - 120 - MASCOT_MARGIN_PX,
      y: 1080 - 140 - MASCOT_MARGIN_PX,
    });
    // 위쪽 48px 작업표시줄: 아래 여백은 그대로, 세로 시작점만 내려간다.
    const top = mon({ workArea: { x: 0, y: 48, width: 1920, height: 1080 - 48 } });
    expect(defaultPosition(top, size)).toEqual({
      x: 1920 - 120 - MASCOT_MARGIN_PX,
      y: 48 + (1080 - 48) - 140 - MASCOT_MARGIN_PX,
    });
  });

  it("화면 안/밖 판정은 workArea가 아니라 전체 경계로 한다", () => {
    // 하단 작업표시줄 위에 놓인 마스코트 — 작업 영역 밖이지만 화면 안이다.
    const m = mon({ workArea: { x: 0, y: 0, width: 1920, height: 1080 - 48 } });
    const overTaskbar = { x: 1700, y: 1080 - 60 };
    expect(isOnMonitor(overTaskbar, size, m)).toBe(true);
    expect(resolvePosition(overTaskbar, size, [m], m)).toEqual(overTaskbar);
  });

  it("usableArea는 workArea를 그대로 돌려주고, 없으면 하단만 줄인다", () => {
    const wa = { x: 5, y: 6, width: 100, height: 200 };
    expect(usableArea(mon({ workArea: wa }))).toEqual(wa);
    expect(usableArea(mon({ scaleFactor: 2 }))).toEqual({
      x: 0,
      y: 0,
      width: 1920,
      height: 1080 - MASCOT_FALLBACK_TASKBAR_INSET_PX * 2,
    });
  });

  it("화면에 걸치는 저장 위치는 그대로 쓴다", () => {
    const saved = { x: 1700, y: 900 };
    expect(resolvePosition(saved, size, [mon()], mon())).toEqual(saved);
  });

  it("모니터가 사라져 화면 밖이 된 저장 위치는 주 모니터 기본 위치로 되돌린다", () => {
    const saved = { x: 3000, y: 400 }; // 떼어낸 외장 모니터 자리
    expect(resolvePosition(saved, size, [mon()], mon())).toEqual(defaultPosition(mon(), size));
  });

  it("모니터 조회가 비면 저장값을 믿는다", () => {
    const saved = { x: 3000, y: 400 };
    expect(resolvePosition(saved, size, [], null)).toEqual(saved);
  });

  it("저장값이 없으면 주 모니터 기본 위치", () => {
    expect(resolvePosition(null, size, [mon(), mon({ x: 1920 })], mon({ x: 1920 }))).toEqual(
      defaultPosition(mon({ x: 1920 }), size),
    );
  });

  it("경계에 살짝 걸친 위치는 화면 안으로 인정한다", () => {
    expect(isOnMonitor({ x: -119, y: 500 }, size, mon())).toBe(true);
    expect(isOnMonitor({ x: -500, y: 500 }, size, mon())).toBe(false);
  });

  it("localStorage 왕복 — 손상값은 없는 것으로 취급", () => {
    const store = new Map<string, string>();
    const storage = {
      getItem: (k: string) => store.get(k) ?? null,
      setItem: (k: string, v: string) => void store.set(k, v),
    };
    expect(readSavedPosition(storage)).toBeNull();
    writeSavedPosition(storage, { x: 10.6, y: 20.4 });
    expect(readSavedPosition(storage)).toEqual({ x: 11, y: 20 });
    store.set("agent-office.mascot.pos", "{oops");
    expect(readSavedPosition(storage)).toBeNull();
  });
});

describe("sheet (순수 계산)", () => {
  it("커스텀 시트 사용 여부는 spriteUpdatedAt으로 판정한다", () => {
    expect(usesCustomSheet({ agentId: "a", spriteUpdatedAt: 1 })).toBe(true);
    expect(usesCustomSheet({ agentId: "a", spriteUpdatedAt: null })).toBe(false);
    expect(usesCustomSheet({ agentId: null, spriteUpdatedAt: 1 })).toBe(false);
  });

  it("표준 4N×N 시트는 셀 크기를 보존하고, 예상 밖 크기는 64×16으로 폴백한다", () => {
    expect(mascotSheetDims(256, 64)).toEqual({ w: 256, h: 64 });
    expect(mascotSheetDims(64, 64)).toEqual({ w: 64, h: 16 });
  });

  it("고해상 셀은 표시 물리 크기까지만 프리필터하고, 저해상 셀은 그대로 둔다", () => {
    // 96px 표시 · dpr 2 → 물리 192px = 렌더 스케일 12 → D = min(N, 16·12)
    expect(mascotDetailCell(256, 2)).toBe(192);
    expect(mascotDetailCell(64, 2)).toBe(64); // 이미 작으면 확대 경로(nearest)
    expect(mascotDetailCell(256, 1)).toBe(96);
  });
});
