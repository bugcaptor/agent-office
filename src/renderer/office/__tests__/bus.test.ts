// src/renderer/office/__tests__/bus.test.ts
//
// Tests for the `OfficeBus` contract's
// dependency-free mock implementation.
//
// Coverage:
// - onNotificationChanged/onSessionStateChanged: multiple listeners all
//   receive triggered events; the returned unsubscribe function stops
//   further delivery to that listener without affecting others.
// - emitAgentClicked: records every call, in order.

import { describe, expect, it, vi } from "vitest";
import { createMockOfficeBus } from "../bus";

describe("createMockOfficeBus / onNotificationChanged", () => {
  it("delivers a triggered event to every subscribed listener", () => {
    const bus = createMockOfficeBus();
    const a = vi.fn();
    const b = vi.fn();
    bus.onNotificationChanged(a);
    bus.onNotificationChanged(b);

    bus.triggerNotificationChanged("agent-1", true);

    expect(a).toHaveBeenCalledWith("agent-1", true);
    expect(b).toHaveBeenCalledWith("agent-1", true);
  });

  it("stops delivering to a listener after it unsubscribes, without affecting others", () => {
    const bus = createMockOfficeBus();
    const a = vi.fn();
    const b = vi.fn();
    const unsubA = bus.onNotificationChanged(a);
    bus.onNotificationChanged(b);

    unsubA();
    bus.triggerNotificationChanged("agent-1", false);

    expect(a).not.toHaveBeenCalled();
    expect(b).toHaveBeenCalledWith("agent-1", false);
  });

  it("unsubscribe is idempotent (calling it twice does not throw)", () => {
    const bus = createMockOfficeBus();
    const unsub = bus.onNotificationChanged(vi.fn());
    unsub();
    expect(() => unsub()).not.toThrow();
  });
});

describe("createMockOfficeBus / onSessionStateChanged", () => {
  it("delivers a triggered event to every subscribed listener", () => {
    const bus = createMockOfficeBus();
    const a = vi.fn();
    bus.onSessionStateChanged(a);

    bus.triggerSessionStateChanged("agent-2", "running");

    expect(a).toHaveBeenCalledWith("agent-2", "running");
  });

  it("stops delivering after unsubscribe", () => {
    const bus = createMockOfficeBus();
    const a = vi.fn();
    const unsub = bus.onSessionStateChanged(a);
    unsub();

    bus.triggerSessionStateChanged("agent-2", "exited");

    expect(a).not.toHaveBeenCalled();
  });
});

describe("createMockOfficeBus / emitAgentClicked", () => {
  it("records every emitted agentId, in order", () => {
    const bus = createMockOfficeBus();
    bus.emitAgentClicked("agent-1");
    bus.emitAgentClicked("agent-2");
    bus.emitAgentClicked("agent-1");

    expect(bus.clickedAgentIds).toEqual(["agent-1", "agent-2", "agent-1"]);
  });

  it("agentHoverChanged: subscribers receive emitted hover payloads", () => {
    const bus = createMockOfficeBus();
    const seen: Array<[string | null, number, number]> = [];
    const off = bus.onAgentHoverChanged((id, x, y) => seen.push([id, x, y]));
    bus.emitAgentHoverChanged("a1", 100, 200);
    bus.emitAgentHoverChanged(null, 0, 0);
    off();
    bus.emitAgentHoverChanged("a2", 5, 5);
    expect(seen).toEqual([
      ["a1", 100, 200],
      [null, 0, 0],
    ]);
  });
});

describe("desk clicked channel", () => {
  it("emitDeskClicked가 구독자에게 (deskIndex, screenX, screenY)를 전달하고, 해제가 동작한다", () => {
    const bus = createMockOfficeBus();
    const seen: Array<[number, number, number]> = [];
    const off = bus.onDeskClicked((i, x, y) => seen.push([i, x, y]));

    bus.emitDeskClicked(3, 120, 80);
    expect(seen).toEqual([[3, 120, 80]]);

    off();
    bus.emitDeskClicked(1, 0, 0);
    expect(seen).toEqual([[3, 120, 80]]);
  });
});

describe("label anchors channel", () => {
  it("emitLabelAnchorsChanged가 구독자에게 같은 Map을 전달하고, 해제가 동작한다", () => {
    const bus = createMockOfficeBus();
    const seen: Array<ReadonlyMap<string, { x: number; y: number }>> = [];
    const off = bus.onLabelAnchorsChanged((m) => seen.push(m));

    const anchors = new Map([["a1", { x: 10, y: 20 }]]);
    bus.emitLabelAnchorsChanged(anchors);
    expect(seen).toHaveLength(1);
    expect(seen[0].get("a1")).toEqual({ x: 10, y: 20 });

    off();
    bus.emitLabelAnchorsChanged(anchors);
    expect(seen).toHaveLength(1);
  });
});

describe("OfficeBus: onSubagentCountChanged", () => {
  it("구독자가 triggerSubagentCountChanged로 (agentId, count)를 받는다", () => {
    const bus = createMockOfficeBus();
    const cb = vi.fn();
    const off = bus.onSubagentCountChanged(cb);
    bus.triggerSubagentCountChanged("a1", 2);
    expect(cb).toHaveBeenCalledWith("a1", 2);
    off();
    bus.triggerSubagentCountChanged("a1", 3);
    expect(cb).toHaveBeenCalledTimes(1);
  });
});
