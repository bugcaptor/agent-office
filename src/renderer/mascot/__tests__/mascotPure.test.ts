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
  anchorOf,
  clampToArea,
  defaultPosition,
  isOnMonitor,
  MASCOT_FALLBACK_TASKBAR_INSET_PX,
  MASCOT_MARGIN_PX,
  MASCOT_POS_KEY,
  readSavedAnchor,
  resolveAnchoredPosition,
  topLeftOf,
  usableArea,
  writeSavedAnchor,
  type MonitorRect,
} from "../position";
import { mascotDetailCell, mascotSheetDims, usesCustomSheet } from "../sheet";
import { computeMascotLayout, computeMascotWindowRect, foldOverflow } from "../layout";

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

  it("lights 부재는 하위호환으로 빈 배열이 된다", () => {
    const parsed = parseMascotState({ visible: true });
    expect(parsed?.lights).toEqual([]);
    expect(parsed?.lightsVertical).toBe(false);
  });

  it("lights 항목을 파싱하고, 상태값이 3종 밖이면 off로 강등한다", () => {
    const parsed = parseMascotState({
      visible: true,
      lights: [
        { id: "a1", label: "철수", state: "attention", clickAgentId: "a1" },
        { id: "p1", label: "repo", state: "bogus", clickAgentId: null },
      ],
      lightsVertical: true,
    });
    expect(parsed?.lights).toEqual([
      { id: "a1", label: "철수", state: "attention", clickAgentId: "a1", avatar: null },
      { id: "p1", label: "repo", state: "off", clickAgentId: null, avatar: null },
    ]);
    expect(parsed?.lightsVertical).toBe(true);
  });

  it("id/label이 없는 lights 항목은 개별적으로 드롭한다", () => {
    const parsed = parseMascotState({
      visible: true,
      lights: [{ id: "a1", label: "철수", state: "working" }, { id: "no-label" }, "nope"],
    });
    expect(parsed?.lights).toEqual([
      { id: "a1", label: "철수", state: "working", clickAgentId: null, avatar: null },
    ]);
  });

  it("lights 항목의 avatar를 파싱하고, agentId/seed가 없으면 null로 접는다", () => {
    const parsed = parseMascotState({
      visible: true,
      lights: [
        {
          id: "p1",
          label: "repo",
          state: "working",
          clickAgentId: "a1",
          avatar: {
            agentId: "a1",
            seed: "s1",
            archetype: "cat",
            colors: { hair: "#ff0000", bogus: 1 },
            spriteUpdatedAt: 42,
          },
        },
        { id: "p2", label: "repo2", state: "off", clickAgentId: null, avatar: { seed: "s2" } },
      ],
    });
    expect(parsed?.lights[0].avatar).toEqual({
      agentId: "a1",
      seed: "s1",
      archetype: "cat",
      colors: { hair: "#ff0000" },
      spriteUpdatedAt: 42,
    });
    expect(parsed?.lights[1].avatar).toBeNull();
  });

  it("sameMascotState는 avatar 차이도 감지한다(얼굴 교체가 dedupe에 먹히지 않게)", () => {
    const withAvatar = (spriteUpdatedAt: number | null) =>
      state({
        lights: [
          {
            id: "p1",
            label: "repo",
            state: "working",
            clickAgentId: "a1",
            avatar: { agentId: "a1", seed: "s1", archetype: null, colors: null, spriteUpdatedAt },
          },
        ],
      });
    expect(sameMascotState(withAvatar(1), withAvatar(1))).toBe(true);
    expect(sameMascotState(withAvatar(1), withAvatar(2))).toBe(false);
    expect(sameMascotState(withAvatar(1), state({ lights: [{ ...withAvatar(1).lights[0], avatar: null }] }))).toBe(false);
  });

  it("sameMascotState는 lights 항목 차이도 감지한다(dedupe 회귀)", () => {
    const a = state({
      lights: [{ id: "a1", label: "a", state: "working", clickAgentId: "a1", avatar: null }],
    });
    const b = state({
      lights: [{ id: "a1", label: "a", state: "attention", clickAgentId: "a1", avatar: null }],
    });
    expect(sameMascotState(a, state({ ...a }))).toBe(true);
    expect(sameMascotState(a, b)).toBe(false);
    expect(sameMascotState(a, state({ lights: [] }))).toBe(false);
  });

});

describe("layout", () => {
  // 치수: 타일 54×48, 간격 6, strip 여백 6, 스프라이트 영역 102(96+여유 6).
  it("가로 모드: strip 두께 60(48+여백12), n=4면 폭 246으로 스프라이트 폭을 넘어선다", () => {
    expect(computeMascotLayout({ lightCount: 4, vertical: false, hasSprite: true })).toEqual({
      width: 12 + 54 * 4 + 6 * 3,
      height: 102 + 60,
    });
    expect(computeMascotLayout({ lightCount: 8, vertical: false, hasSprite: true })).toEqual({
      width: 486,
      height: 102 + 60,
    });
  });

  it("가로 모드: 스프라이트 없으면 폭은 strip 폭 그대로, 높이는 strip 두께뿐", () => {
    expect(computeMascotLayout({ lightCount: 4, vertical: false, hasSprite: false })).toEqual({
      width: 12 + 54 * 4 + 6 * 3,
      height: 60,
    });
  });

  it("세로 모드: 폭은 max(스프라이트 폭, 타일 폭+여백), 높이는 스프라이트 + stripH", () => {
    expect(computeMascotLayout({ lightCount: 8, vertical: true, hasSprite: true })).toEqual({
      width: 120,
      height: 102 + (12 + 48 * 8 + 6 * 7),
    });
    expect(computeMascotLayout({ lightCount: 1, vertical: true, hasSprite: false })).toEqual({
      width: 54 + 12,
      height: 48 + 12,
    });
  });

  it("0칸이면 strip 두께가 0이라 스프라이트만큼만 남는다", () => {
    expect(computeMascotLayout({ lightCount: 0, vertical: false, hasSprite: true })).toEqual({
      width: 120,
      height: 102,
    });
    expect(computeMascotLayout({ lightCount: 0, vertical: false, hasSprite: false })).toEqual({
      width: 0,
      height: 0,
    });
  });

  it("foldOverflow: 상한 이하면 그대로, 넘으면 앞 (max-1)칸 + 칩 개수", () => {
    const lights = Array.from({ length: 12 }, (_, i) => `l${i}`);
    expect(foldOverflow(lights, 12)).toEqual({ shown: lights, overflowCount: 0 });

    const over = Array.from({ length: 13 }, (_, i) => `l${i}`);
    const folded = foldOverflow(over, 12);
    expect(folded.shown).toEqual(over.slice(0, 11));
    expect(folded.overflowCount).toBe(2);
  });

  it("foldOverflow: 기본 상한(MAX_LIGHTS=8)을 쓴다", () => {
    const over = Array.from({ length: 9 }, (_, i) => `l${i}`);
    const folded = foldOverflow(over);
    expect(folded.shown).toHaveLength(7);
    expect(folded.overflowCount).toBe(2);
  });

  describe("computeMascotWindowRect (C9)", () => {
    const mon = (patch: Partial<MonitorRect> = {}): MonitorRect => ({
      x: 0,
      y: 0,
      width: 1920,
      height: 1080,
      scaleFactor: 1,
      ...patch,
    });

    it("dpr 1: 물리 px = 논리 px, 앵커(하단중앙) 기준으로 재배치한다", () => {
      // 스프라이트만 있는(칸 0) 기존 120×102 창이 (100,898)에 있다고 하자.
      const rect = computeMascotWindowRect({
        lightCount: 0,
        vertical: false,
        hasSprite: true,
        dpr: 1,
        currentPos: { x: 100, y: 898 },
        currentSize: { width: 120, height: 102 },
        monitors: [mon()],
        primary: mon(),
      });
      // 앵커 = (100+60, 898+102) = (160, 1000). 새 크기도 120×102라
      // top-left는 그대로 (100, 898) — 칸 수가 그대로면 자리가 안 변한다.
      expect(rect).toEqual({ width: 120, height: 102, x: 100, y: 898 });
    });

    it("칸이 늘면 하단중앙 앵커를 유지한 채 위/옆으로만 커진다", () => {
      const rect = computeMascotWindowRect({
        lightCount: 4,
        vertical: false,
        hasSprite: true,
        dpr: 1,
        currentPos: { x: 100, y: 898 }, // 120×102일 때의 top-left
        currentSize: { width: 120, height: 102 },
        monitors: [mon()],
        primary: mon(),
      });
      // 새 크기 246×162(스프라이트102 + strip두께60, n=4의 strip 폭 246).
      // 앵커 (160,1000) 고정 → top-left = (160-123, 1000-162) = (37, 838).
      expect(rect).toEqual({ width: 246, height: 162, x: 37, y: 838 });
    });

    it("dpr 2면 물리 px로 2배 환산해서 계산한다", () => {
      const rect = computeMascotWindowRect({
        lightCount: 0,
        vertical: false,
        hasSprite: true,
        dpr: 2,
        currentPos: { x: 200, y: 1720 }, // 240×204 물리 창의 top-left
        currentSize: { width: 240, height: 204 },
        monitors: [mon({ scaleFactor: 2, width: 3840, height: 2160 })],
        primary: mon({ scaleFactor: 2, width: 3840, height: 2160 }),
      });
      expect(rect).toEqual({ width: 240, height: 204, x: 200, y: 1720 });
    });

    it("리사이즈로 화면 밖을 침범하면 현재 걸친 모니터 안으로 클램프한다", () => {
      const rect = computeMascotWindowRect({
        lightCount: 8,
        vertical: true, // 세로 8칸 → 매우 높은 창
        hasSprite: true,
        dpr: 1,
        currentPos: { x: 100, y: 10 }, // 화면 위쪽 끝 근처
        currentSize: { width: 120, height: 102 },
        monitors: [mon()],
        primary: mon(),
      });
      // 세로 모드 8칸: height = 102 + (12+48*8+6*7) = 102+438 = 540,
      // width = max(120, 58) = 120. 앵커를 그대로 따르면 top이 화면 밖(y<0)
      // 이라 클램프가 y=0으로 되돌린다.
      expect(rect.width).toBe(120);
      expect(rect.height).toBe(540);
      expect(rect.y).toBe(0);
    });

    it("모니터 목록이 비면 클램프 없이 앵커 결과를 그대로 쓴다", () => {
      const rect = computeMascotWindowRect({
        lightCount: 0,
        vertical: false,
        hasSprite: false,
        dpr: 1,
        currentPos: { x: -500, y: -500 },
        currentSize: { width: 120, height: 140 },
        monitors: [],
        primary: null,
      });
      expect(rect).toEqual({ width: 0, height: 0, x: -500 + 60, y: -500 + 140 });
    });

    it("dpr 1.5: strip-only → 스프라이트 등장 전환에서도 x/y가 정수다(B1 회귀)", () => {
      // 램프 3칸(스프라이트 없음)의 논리 폭 162가 dpr 1.5에서 물리 243(홀수)이
      // 된다. 여기서 칸이 사라지고 스프라이트만 남으면(물리 폭 180, 짝수) 앵커
      // 역산의 절반 나누기에서 .5가 남는다(리뷰 B1 실측 재현).
      const mon = { x: 0, y: 0, width: 1920, height: 1080, scaleFactor: 1.5 };
      const rect = computeMascotWindowRect({
        lightCount: 0,
        vertical: false,
        hasSprite: true,
        dpr: 1.5,
        currentPos: { x: 1000, y: 800 },
        currentSize: { width: 243, height: 90 },
        monitors: [mon],
        primary: mon,
      });
      expect(Number.isInteger(rect.x)).toBe(true);
      expect(Number.isInteger(rect.y)).toBe(true);
      expect(rect.x).toBe(1032); // Math.round(1000 + 243/2 - 180/2) = Math.round(1031.5)
    });

    it("dpr 1.5: 스프라이트 유지 + 칸 수 축소(strip 폭 홀수) 전환에서도 정수다(B1 회귀)", () => {
      // 칸이 많은 상태(물리 폭 441, 홀수)에서 0칸(스프라이트만, 물리 폭 180)으로
      // 줄어드는 반대 방향 — currentSize가 홀수인 쪽에서도 같은 문제가 난다.
      const mon = { x: 0, y: 0, width: 1920, height: 1080, scaleFactor: 1.5 };
      const rect = computeMascotWindowRect({
        lightCount: 0,
        vertical: false,
        hasSprite: true,
        dpr: 1.5,
        currentPos: { x: 1000, y: 800 },
        currentSize: { width: 441, height: 255 },
        monitors: [mon],
        primary: mon,
      });
      expect(Number.isInteger(rect.x)).toBe(true);
      expect(Number.isInteger(rect.y)).toBe(true);
    });
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
    expect(resolveAnchoredPosition(anchorOf(overTaskbar, size), size, [m], m)).toEqual(overTaskbar);
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

  it("앵커 왕복: anchorOf/topLeftOf는 서로 역함수다", () => {
    const pos = { x: 1700, y: 900 };
    expect(topLeftOf(anchorOf(pos, size), size)).toEqual(pos);
  });

  it("화면에 걸치는 저장 앵커는 그대로 쓴다", () => {
    const saved = { x: 1700, y: 900 };
    expect(resolveAnchoredPosition(anchorOf(saved, size), size, [mon()], mon())).toEqual(saved);
  });

  it("모니터가 사라져 화면 밖이 된 저장 앵커는 주 모니터 기본 위치로 되돌린다", () => {
    const saved = { x: 3000, y: 400 }; // 떼어낸 외장 모니터 자리
    expect(resolveAnchoredPosition(anchorOf(saved, size), size, [mon()], mon())).toEqual(
      defaultPosition(mon(), size),
    );
  });

  it("모니터 조회가 비면 저장 앵커를 믿는다", () => {
    const saved = { x: 3000, y: 400 };
    expect(resolveAnchoredPosition(anchorOf(saved, size), size, [], null)).toEqual(saved);
  });

  it("저장 앵커가 없으면 주 모니터 기본 위치", () => {
    expect(resolveAnchoredPosition(null, size, [mon(), mon({ x: 1920 })], mon({ x: 1920 }))).toEqual(
      defaultPosition(mon({ x: 1920 }), size),
    );
  });

  it("경계에 살짝 걸친 위치는 화면 안으로 인정한다", () => {
    expect(isOnMonitor({ x: -119, y: 500 }, size, mon())).toBe(true);
    expect(isOnMonitor({ x: -500, y: 500 }, size, mon())).toBe(false);
  });

  it("clampToArea: 화면 밖으로 나가면 모니터 안으로 밀어 넣고, 안이면 그대로 둔다", () => {
    const m = mon();
    expect(clampToArea({ x: -50, y: 2000 }, size, m)).toEqual({ x: 0, y: 1080 - 140 });
    expect(clampToArea({ x: 100, y: 100 }, size, m)).toEqual({ x: 100, y: 100 });
  });

  it("clampToArea: 창이 모니터보다 크면 좌상단을 모니터 좌상단에 맞춘다", () => {
    const small = mon({ width: 80, height: 80 });
    expect(clampToArea({ x: 500, y: 500 }, size, small)).toEqual({ x: 0, y: 0 });
  });

  it("localStorage 앵커 왕복 — 손상값은 없는 것으로 취급", () => {
    const store = new Map<string, string>();
    const storage = {
      getItem: (k: string) => store.get(k) ?? null,
      setItem: (k: string, v: string) => void store.set(k, v),
    };
    expect(readSavedAnchor(storage, size)).toBeNull();
    writeSavedAnchor(storage, { ax: 500.6, ay: 600.4 });
    expect(readSavedAnchor(storage, size)).toEqual({ ax: 501, ay: 600 });
    store.set(MASCOT_POS_KEY, "{oops");
    expect(readSavedAnchor(storage, size)).toBeNull();
  });

  it("구형 {x,y} 저장값은 assumedSize로 앵커 환산해 오차 없이 같은 자리로 복원한다(§0.4)", () => {
    const store = new Map<string, string>();
    store.set(MASCOT_POS_KEY, JSON.stringify({ x: 1700, y: 900 }));
    const storage = { getItem: (k: string) => store.get(k) ?? null };
    const migrated = readSavedAnchor(storage, size);
    expect(migrated).toEqual(anchorOf({ x: 1700, y: 900 }, size));
    // 마운트 직후 실측 크기(size)로 되읽으면 정확히 같은 top-left가 나온다.
    expect(topLeftOf(migrated!, size)).toEqual({ x: 1700, y: 900 });
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
