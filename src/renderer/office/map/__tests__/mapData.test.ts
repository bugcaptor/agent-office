// src/renderer/office/map/__tests__/mapData.test.ts
//
// The break room (탕비실) rectangle must remain valid: its
// BREAK_ROOM_RECT interior must be fully walkable so pathing.pickBreakTarget
// can safely rejection-sample inside it. Also pins the Tile enum's numeric
// values, since they're baked into the hardcoded GRID and any persisted data.

import { describe, expect, it } from "vitest";
import { BREAK_ROOM_RECT, MEETING_MAX_SEATS, OFFICE_MAP, QUEUE_SLOTS, Tile } from "../mapData";
import { isWalkable } from "../../world/pathing";

describe("Tile enum stability", () => {
  it("keeps existing members at their original numeric values", () => {
    expect(Tile.Floor).toBe(0);
    expect(Tile.Wall).toBe(1);
    expect(Tile.DeskTop).toBe(2);
    expect(Tile.Rug).toBe(3);
  });

  it("appends new members without renumbering existing ones", () => {
    expect(Tile.Plant).toBe(4);
    expect(Tile.Counter).toBe(5);
    expect(Tile.Table).toBe(6);
    expect(Tile.BossDesk).toBe(7);
    expect(Tile.MeetingFloor).toBe(8);
  });
});

describe("BREAK_ROOM_RECT", () => {
  it("is a non-empty rectangle fully inside the map bounds", () => {
    expect(BREAK_ROOM_RECT.w).toBeGreaterThan(0);
    expect(BREAK_ROOM_RECT.h).toBeGreaterThan(0);
    expect(BREAK_ROOM_RECT.x).toBeGreaterThanOrEqual(0);
    expect(BREAK_ROOM_RECT.y).toBeGreaterThanOrEqual(0);
    expect(BREAK_ROOM_RECT.x + BREAK_ROOM_RECT.w).toBeLessThanOrEqual(OFFICE_MAP.width);
    expect(BREAK_ROOM_RECT.y + BREAK_ROOM_RECT.h).toBeLessThanOrEqual(OFFICE_MAP.height);
  });

  it("every tile inside the rect is walkable (Floor or Rug)", () => {
    for (let ty = BREAK_ROOM_RECT.y; ty < BREAK_ROOM_RECT.y + BREAK_ROOM_RECT.h; ty++) {
      for (let tx = BREAK_ROOM_RECT.x; tx < BREAK_ROOM_RECT.x + BREAK_ROOM_RECT.w; tx++) {
        const t = OFFICE_MAP.tiles[ty][tx];
        expect([Tile.Floor, Tile.Rug], `tile (${tx},${ty}) = ${t}`).toContain(t);
      }
    }
  });

});

describe("OFFICE_MAP.meetingRoom (토의 회의실)", () => {
  const room = OFFICE_MAP.meetingRoom!;
  const inRect = (r: { x: number; y: number; w: number; h: number }, tx: number, ty: number) =>
    tx >= r.x && tx < r.x + r.w && ty >= r.y && ty < r.y + r.h;
  const key = (p: { tx: number; ty: number }) => `${p.tx},${p.ty}`;

  it("맵 안의 사각형이고, 안쪽은 회의실 바닥과 테이블뿐이며 테이블이 있다", () => {
    expect(room).toBeDefined();
    expect(room.rect.x + room.rect.w).toBeLessThanOrEqual(OFFICE_MAP.width);
    expect(room.rect.y + room.rect.h).toBeLessThanOrEqual(OFFICE_MAP.height);
    const kinds = new Set<Tile>();
    for (let ty = room.rect.y; ty < room.rect.y + room.rect.h; ty++) {
      for (let tx = room.rect.x; tx < room.rect.x + room.rect.w; tx++) kinds.add(OFFICE_MAP.tiles[ty][tx]);
    }
    expect([...kinds].sort()).toEqual([Tile.Table, Tile.MeetingFloor].sort());
  });

  it("회의실 바닥 타일은 사각형 밖에 없다(그림과 데이터가 같은 영역)", () => {
    for (let ty = 0; ty < OFFICE_MAP.height; ty++) {
      for (let tx = 0; tx < OFFICE_MAP.width; tx++) {
        if (OFFICE_MAP.tiles[ty][tx] === Tile.MeetingFloor) expect(inRect(room.rect, tx, ty), `(${tx},${ty})`).toBe(true);
      }
    }
  });

  it("의자는 토의 인원 상한(5)만큼이고, 서로 다른 걸을 수 있는 회의실 타일이며 바라보는 쪽에 테이블이 있다", () => {
    expect(room.seats).toHaveLength(MEETING_MAX_SEATS);
    expect(new Set(room.seats.map(key)).size).toBe(room.seats.length);
    for (const seat of room.seats) {
      expect(isWalkable(OFFICE_MAP, seat.tx, seat.ty)).toBe(true);
      expect(inRect(room.rect, seat.tx, seat.ty)).toBe(true);
      const [dx, dy] = seat.facing === "down" ? [0, 1] : seat.facing === "left" ? [-1, 0] : [1, 0];
      expect(OFFICE_MAP.tiles[seat.ty + dy][seat.tx + dx], key(seat)).toBe(Tile.Table);
    }
  });

  it("데스크 좌석·줄서기 슬롯·탕비실·보스 책상과 겹치지 않는다", () => {
    const deskSeats = new Set(OFFICE_MAP.desks.map((d) => key(d.seat)));
    const queue = new Set(QUEUE_SLOTS.map(key));
    for (let ty = room.rect.y; ty < room.rect.y + room.rect.h; ty++) {
      for (let tx = room.rect.x; tx < room.rect.x + room.rect.w; tx++) {
        expect(deskSeats.has(`${tx},${ty}`)).toBe(false);
        expect(queue.has(`${tx},${ty}`)).toBe(false);
        expect(inRect(BREAK_ROOM_RECT, tx, ty)).toBe(false);
        expect(inRect(OFFICE_MAP.bossDesk!, tx, ty)).toBe(false);
      }
    }
  });

  it("회의 테이블(Table)은 데스크 슬롯을 만들지 않는다 — 데스크 8개 그대로", () => {
    expect(OFFICE_MAP.desks).toHaveLength(8);
  });

  it("회의실 둘레는 막히지 않는다 — 사각형 바깥 한 칸 테두리에 걸을 수 있는 통로가 있다", () => {
    // 위(ty6)·아래(ty10 쪽)로 지나다니는 동선이 회의실 때문에 끊기지 않는지: 테두리 행·열이 모두 걸을 수 있다.
    const { x, y, w, h } = room.rect;
    for (let tx = x - 1; tx <= x + w; tx++) {
      expect(isWalkable(OFFICE_MAP, tx, y - 1), `north (${tx},${y - 1})`).toBe(true);
    }
    for (let ty = y; ty < y + h; ty++) {
      expect(isWalkable(OFFICE_MAP, x + w, ty), `east (${x + w},${ty})`).toBe(true);
    }
  });
});
