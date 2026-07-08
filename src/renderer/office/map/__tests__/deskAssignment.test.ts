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

  it('every desk seat sits on a floor-ish tile directly below a DeskTop pair', () => {
    for (const desk of OFFICE_MAP.desks) {
      const { tx, ty } = desk.seat;
      expect(tx).toBeGreaterThanOrEqual(0);
      expect(ty).toBeGreaterThanOrEqual(0);
      expect(ty).toBeLessThan(OFFICE_MAP.height);
      expect(desk.facing).toBe('up');
    }
  });

  it('desk indices are 0..N-1 with no gaps or duplicates', () => {
    const indices = OFFICE_MAP.desks.map((d) => d.index).sort((a, b) => a - b);
    expect(indices).toEqual(Array.from({ length: OFFICE_MAP.desks.length }, (_, i) => i));
  });
});
