// src/renderer/office/map/__tests__/deskAssignment.test.ts
//
// assignDesks must be order-independent (same agent set → same seat
// regardless of input order) and collision-free (no two agents share a
// seat). Overflow policy: agents beyond desk capacity remain unassigned.

import { describe, it, expect } from 'vitest';
import { assignDesks } from '../deskAssignment';
import { OFFICE_MAP } from '../mapData';

describe('desk assignment', () => {
  it('is order-independent and stable', () => {
    const ids = ['x', 'y', 'z', 'p', 'q'];
    const a = assignDesks(OFFICE_MAP, ids);
    const b = assignDesks(OFFICE_MAP, [...ids].reverse());
    for (const id of ids) expect(a.get(id)!.index).toBe(b.get(id)!.index);
  });

  it('assigns unique seats without collision', () => {
    const ids = Array.from({ length: OFFICE_MAP.desks.length }, (_, i) => `a${i}`);
    const m = assignDesks(OFFICE_MAP, ids);
    const seats = new Set([...m.values()].map((d) => d.index));
    expect(seats.size).toBe(ids.length);
  });

  it('is stable across many random shuffles of the same id set', () => {
    const ids = Array.from({ length: 8 }, (_, i) => `agent-${i}`);
    const baseline = assignDesks(OFFICE_MAP, ids);
    for (let trial = 0; trial < 5; trial++) {
      const shuffled = [...ids].sort(() => Math.random() - 0.5);
      const m = assignDesks(OFFICE_MAP, shuffled);
      for (const id of ids) expect(m.get(id)!.index).toBe(baseline.get(id)!.index);
    }
  });

  it('leaves overflow agents unassigned when ids exceed desk capacity', () => {
    const capacity = OFFICE_MAP.desks.length;
    const ids = Array.from({ length: capacity + 3 }, (_, i) => `overflow-${i}`);
    const m = assignDesks(OFFICE_MAP, ids);
    expect(m.size).toBe(capacity);
    // every assigned seat is still unique
    const seats = new Set([...m.values()].map((d) => d.index));
    expect(seats.size).toBe(capacity);
  });

  it('is deterministic across repeated calls (no hidden randomness)', () => {
    const ids = ['alpha', 'beta', 'gamma'];
    const a = assignDesks(OFFICE_MAP, ids);
    const b = assignDesks(OFFICE_MAP, ids);
    for (const id of ids) expect(a.get(id)!.index).toBe(b.get(id)!.index);
  });

  it('returns an empty map for an empty agent list', () => {
    const m = assignDesks(OFFICE_MAP, []);
    expect(m.size).toBe(0);
  });
});

describe('manual desk designation', () => {
  it('seats a manually designated agent at exactly that desk', () => {
    const ids = ['a', 'b', 'c'];
    const m = assignDesks(OFFICE_MAP, ids, new Map([['b', 3]]));
    expect(m.get('b')!.index).toBe(3);
  });

  it('keeps designated desks out of the auto pool (never claimed by others)', () => {
    // 정원 초과로 자동 배정이 모든 빈 책상을 채우게 해도, 지정된 책상은
    // 그 주인 몫으로 남아야 한다.
    const capacity = OFFICE_MAP.desks.length;
    const ids = Array.from({ length: capacity + 4 }, (_, i) => `agent-${i}`);
    const manual = new Map([['agent-0', 5]]);
    const m = assignDesks(OFFICE_MAP, ids, manual);
    for (const [id, slot] of m) {
      if (slot.index === 5) expect(id).toBe('agent-0');
    }
    expect(m.get('agent-0')!.index).toBe(5);
  });

  it('assigns unassigned agents only to never-designated desks, without collision', () => {
    const ids = ['a', 'b', 'c', 'd'];
    const manual = new Map([
      ['a', 0],
      ['b', 1],
    ]);
    const m = assignDesks(OFFICE_MAP, ids, manual);
    const indices = [...m.values()].map((d) => d.index);
    expect(new Set(indices).size).toBe(indices.length); // no collision
    expect(m.get('c')!.index).toBeGreaterThanOrEqual(2);
    expect(m.get('d')!.index).toBeGreaterThanOrEqual(2);
  });

  it('is order-independent with manual designations present', () => {
    const ids = ['x', 'y', 'z', 'p', 'q'];
    const manual = new Map([['z', 2]]);
    const a = assignDesks(OFFICE_MAP, ids, manual);
    const b = assignDesks(OFFICE_MAP, [...ids].reverse(), manual);
    for (const id of ids) expect(a.get(id)!.index).toBe(b.get(id)!.index);
  });

  it('treats an out-of-range manual index as unassigned (auto fallback)', () => {
    const m = assignDesks(OFFICE_MAP, ['a'], new Map([['a', 99]]));
    expect(m.get('a')).toBeDefined();
    expect(m.get('a')!.index).toBeLessThan(OFFICE_MAP.desks.length);
  });

  it('resolves duplicate designations to a single winner, the loser falls back to auto', () => {
    const m = assignDesks(
      OFFICE_MAP,
      ['a', 'b'],
      new Map([
        ['a', 4],
        ['b', 4],
      ]),
    );
    const indices = [...m.values()].map((d) => d.index);
    expect(new Set(indices).size).toBe(2);
    expect(indices).toContain(4);
  });
});

describe('office map data', () => {
  it('has the documented dimensions (20x14) and 8 desk slots', () => {
    expect(OFFICE_MAP.width).toBe(20);
    expect(OFFICE_MAP.height).toBe(14);
    expect(OFFICE_MAP.desks.length).toBe(8);
  });

  it('tiles array shape matches width/height ([ty][tx])', () => {
    expect(OFFICE_MAP.tiles.length).toBe(OFFICE_MAP.height);
    for (const row of OFFICE_MAP.tiles) expect(row.length).toBe(OFFICE_MAP.width);
  });

  it('every desk seat sits on a walkable tile directly ABOVE a DeskTop pair, facing down', () => {
    for (const desk of OFFICE_MAP.desks) {
      const { tx, ty } = desk.seat;
      expect(tx).toBeGreaterThanOrEqual(0);
      expect(ty).toBeGreaterThanOrEqual(0);
      expect(ty).toBeLessThan(OFFICE_MAP.height);
      // 캐릭터 정면이 보이도록 좌석은 책상 위쪽(북쪽) 타일, 시선은 아래(남쪽).
      expect(OFFICE_MAP.tiles[ty + 1][tx]).toBe(2 /* Tile.DeskTop */);
      expect(OFFICE_MAP.tiles[ty][tx]).toBe(0 /* Tile.Floor */);
      expect(desk.facing).toBe('down');
    }
  });

  it('desk indices are 0..N-1 with no gaps or duplicates', () => {
    const indices = OFFICE_MAP.desks.map((d) => d.index).sort((a, b) => a - b);
    expect(indices).toEqual(Array.from({ length: OFFICE_MAP.desks.length }, (_, i) => i));
  });
});
