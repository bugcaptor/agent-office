// @vitest-environment jsdom
//
// src/renderer/office/__tests__/OfficeCanvas.test.tsx
//
// Tests for `OfficeCanvas` + the
// `useOfficeScene` hook it's built on, in particular no-leak behavior under
// React StrictMode's dev-mode mount -> cleanup -> re-mount double-invoke.
//
// Rendering through `<OfficeCanvas>` (rather than calling `useOfficeScene`
// via `renderHook`) is deliberate: the hook only sees a non-null
// `canvasRef.current` once a real `<canvas ref={canvasRef}/>` has committed,
// which is exactly what `OfficeCanvas` provides — the realistic mount path
// per the frozen `<OfficeCanvas bus={officeBus} profiles={agentList} />` contract.
//
// `../OfficeScene` is mocked entirely: a real one needs a WebGL/canvas-2d
// context unavailable in jsdom, and `OfficeScene`'s own dispose-safety
// guard is already covered by OfficeScene.test.ts. This test is only about
// the hook's `disposed`-flag orchestration built on top of it: does it call
// `destroy()` at the right times, and never call `syncAgents()` on a scene
// it has already abandoned.

import React from "react";
import { act, render } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { AgentProfile } from "../types";

const state = vi.hoisted(() => ({
  instances: [] as FakeOfficeSceneInstance[],
}));

interface FakeOfficeSceneInstance {
  destroy: ReturnType<typeof vi.fn>;
  syncAgents: ReturnType<typeof vi.fn>;
  resolveInit: () => void;
  canvas: HTMLCanvasElement;
}

vi.mock("../OfficeScene", () => {
  class FakeOfficeScene implements FakeOfficeSceneInstance {
    destroy = vi.fn();
    syncAgents = vi.fn();
    resolveInit: () => void = () => {};
    canvas: HTMLCanvasElement;

    constructor(opts: { canvas: HTMLCanvasElement }) {
      this.canvas = opts.canvas;
      state.instances.push(this);
    }

    init(): Promise<void> {
      return new Promise<void>((resolve) => {
        this.resolveInit = resolve;
      });
    }
  }
  return { OfficeScene: FakeOfficeScene };
});

const { OfficeCanvas } = await import("../OfficeCanvas");
const { createMockOfficeBus } = await import("../bus");

const profile = (id: string): AgentProfile => ({ id, name: id, role: "eng", seed: id });

/** Flushes microtasks so `scene.init().then(...)` callbacks run inside `act`. */
async function flushMicrotasks(): Promise<void> {
  await act(async () => {
    await Promise.resolve();
  });
}

beforeEach(() => {
  state.instances = [];
});

describe("OfficeCanvas / single mount (non-StrictMode)", () => {
  it("renders a <canvas>, constructs one scene, and syncs profiles both eagerly and once init resolves", async () => {
    // Note on the two `syncAgents` calls below: `useOfficeScene` has two
    // effects — the mount effect (empty deps) and a `[profiles]`-keyed sync
    // effect. Both run on the very first render, so the sync effect fires
    // once immediately (before `init()` has resolved) and once more from
    // the mount effect's `init().then()` callback once it has. This is
    // harmless here because `OfficeScene.syncAgents` is a guarded no-op
    // stub in this task (see its doc comment) — real per-init state (3H's
    // `OfficeWorld`) must apply the same guard.
    const bus = createMockOfficeBus();
    render(<OfficeCanvas bus={bus} profiles={[profile("a1")]} />);

    expect(document.querySelector("canvas")).not.toBeNull();
    expect(state.instances).toHaveLength(1);
    expect(state.instances[0].syncAgents).toHaveBeenCalledTimes(1);
    expect(state.instances[0].syncAgents).toHaveBeenCalledWith([profile("a1")]);
    expect(state.instances[0].destroy).not.toHaveBeenCalled();

    act(() => state.instances[0].resolveInit());
    await flushMicrotasks();

    expect(state.instances[0].syncAgents).toHaveBeenCalledTimes(2);
    expect(state.instances[0].destroy).not.toHaveBeenCalled();
  });

  it("syncs new profiles on prop changes without recreating the scene", async () => {
    const bus = createMockOfficeBus();
    const { rerender } = render(<OfficeCanvas bus={bus} profiles={[profile("a1")]} />);
    act(() => state.instances[0].resolveInit());
    await flushMicrotasks();

    rerender(<OfficeCanvas bus={bus} profiles={[profile("a1"), profile("a2")]} />);

    expect(state.instances).toHaveLength(1); // no new OfficeScene constructed
    expect(state.instances[0].syncAgents).toHaveBeenLastCalledWith([profile("a1"), profile("a2")]);
  });

  it("destroys the scene on unmount", async () => {
    const bus = createMockOfficeBus();
    const { unmount } = render(<OfficeCanvas bus={bus} profiles={[]} />);
    act(() => state.instances[0].resolveInit());
    await flushMicrotasks();

    unmount();

    expect(state.instances[0].destroy).toHaveBeenCalledTimes(1);
  });
});

describe("OfficeCanvas / React.StrictMode double-mount (dispose-correctness)", () => {
  it("abandons the first (StrictMode-simulated) scene without ever syncing it, keeps exactly one live scene", async () => {
    const bus = createMockOfficeBus();
    render(
      <React.StrictMode>
        <OfficeCanvas bus={bus} profiles={[profile("a1")]} />
      </React.StrictMode>,
    );

    // StrictMode's synchronous mount -> cleanup -> re-mount already ran
    // inside `render`'s `act`, so two scenes should exist by now.
    expect(state.instances).toHaveLength(2);
    const [first, second] = state.instances;

    // Cleanup for the first scene fires synchronously and unconditionally;
    // its `[profiles]` sync effect also fired once before being abandoned
    // (see the note in the non-StrictMode test above).
    expect(first.destroy).toHaveBeenCalledTimes(1);
    expect(first.syncAgents).toHaveBeenCalledTimes(1);
    expect(second.destroy).not.toHaveBeenCalled();
    expect(second.syncAgents).toHaveBeenCalledTimes(1);

    // Resolve both scenes' init() — the abandoned one must self-destroy
    // again (via the hook's `disposed` flag) and, critically, must NEVER
    // sync agents again post-abandonment; the surviving one syncs normally.
    act(() => {
      first.resolveInit();
      second.resolveInit();
    });
    await flushMicrotasks();

    expect(first.syncAgents).toHaveBeenCalledTimes(1); // unchanged: no post-abandonment sync
    expect(first.destroy).toHaveBeenCalledTimes(2); // cleanup (pre-init) + init.then (post-init)
    expect(second.syncAgents).toHaveBeenCalledTimes(2);
    expect(second.syncAgents).toHaveBeenCalledWith([profile("a1")]);
    expect(second.destroy).not.toHaveBeenCalled();
  });

  // Root-cause regression (black main view): the abandoned first
  // scene and the surviving second scene must never be constructed with the
  // *same* `<canvas>` DOM node. A real (unmocked) `OfficeScene.destroy()`
  // calls `Application.destroy(true, ...)` (removeView) once its `init()`
  // resolves post-abandonment; if both scenes shared one canvas element
  // (and therefore one underlying WebGL context, since a canvas can only
  // ever back a single rendering context), that deferred destroy tears the
  // context out from under the still-live second scene -- Pixi logs "Could
  // not retrieve shader source (WebGL context may be lost)" and the office
  // never paints (confirmed via headless-Chrome CDP: the office `<canvas>`
  // was entirely absent from the committed DOM). Giving every scene its own
  // privately-created canvas (never a single React-JSX-rendered one) makes
  // the two Pixi Applications fully independent, so the abandoned scene's
  // teardown cannot affect the surviving one.
  it("constructs the two StrictMode-doubled scenes with distinct <canvas> elements", () => {
    const bus = createMockOfficeBus();
    const { container } = render(
      <React.StrictMode>
        <OfficeCanvas bus={bus} profiles={[]} />
      </React.StrictMode>,
    );

    expect(state.instances).toHaveLength(2);
    const [first, second] = state.instances;
    expect(first.canvas).not.toBe(second.canvas);

    // The abandoned scene's canvas must not linger in the DOM once its
    // effect has cleaned up -- exactly one live <canvas> should remain
    // within this render's own container (other tests in this file don't
    // unmount, so `document`-wide queries would see their leftovers too).
    expect(container.querySelectorAll("canvas")).toHaveLength(1);
    expect(container.querySelectorAll("canvas")[0]).toBe(second.canvas);
  });
});
