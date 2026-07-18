// @vitest-environment jsdom
//
// src/renderer/terminal/__tests__/TerminalRegistry.test.ts
//
// Tests for the xterm keep-alive registry.
// `@xterm/xterm` and `@xterm/addon-fit` are mocked (jsdom can't do the
// canvas-based text measurement xterm needs), and `tauriApi` is mocked too —
// the original design skeleton's `window.api` is what an earlier task built
// as the `tauriApi` module, so the registry imports it directly rather than
// reading a `window.api` global.
//
// Coverage:
// - T5 keep-alive: `ensure()` called twice for the same agentId returns the
//   same `Terminal` instance, and `destroy()` unsubscribes `onData` exactly
//   once and disposes the term.
// - Wiring direction: backend push (`tauriApi.onData` callback) writes to
//   `term.write`; user keystrokes (`term.onData`) call `tauriApi.writeInput`.
// - `attach()` opens the term into the container exactly once, even across
//   repeated attach calls (remounts must not re-open / re-create anything).
// - `activate()`/`refit()` fit + report size + (activate only) focus, and
//   are no-ops before the term has been opened.

import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

const openMock = vi.fn();
const disposeMock = vi.fn();
const focusMock = vi.fn();
// xterm.write(data, cb?)의 cb는 write 큐가 비워지면 호출된다 — flushAndSerializeAll이
// 이 콜백을 기다리므로 목도 콜백을 즉시 불러 준다(§P1).
const writeMock = vi.fn((_data?: string, cb?: () => void) => cb?.());
const loadAddonMock = vi.fn();
const fitMock = vi.fn();
const pasteMock = vi.fn();
let selectionValue: string | undefined;

/** Minimal stand-in for `xterm.Terminal`: only what TerminalRegistry touches. */
class FakeTerminal {
  cols = 80;
  rows = 24;
  options: unknown;
  textarea = document.createElement("textarea");
  private dataHandler: ((d: string) => void) | undefined;
  private keyEventHandler: ((event: KeyboardEvent) => boolean) | undefined;
  constructor(options: unknown) {
    this.options = options;
  }
  loadAddon = loadAddonMock;
  open = openMock;
  dispose = disposeMock;
  focus = focusMock;
  write = writeMock;
  paste = pasteMock;
  onData(handler: (d: string) => void) {
    this.dataHandler = handler;
  }
  /** Test helper: simulate the user typing into the terminal. */
  emitInput(data: string) {
    this.dataHandler?.(data);
  }
  /** Test helper: simulate the IME finalizing a composed syllable. */
  emitCompositionEnd() {
    this.textarea.dispatchEvent(new Event("compositionend"));
  }
  attachCustomKeyEventHandler(handler: (event: KeyboardEvent) => boolean) {
    this.keyEventHandler = handler;
  }
  /** Test helper: simulate a key event reaching xterm's custom handler. */
  emitKeyEvent(event: KeyboardEvent): boolean | undefined {
    return this.keyEventHandler?.(event);
  }
  hasSelection() {
    return selectionValue !== undefined && selectionValue.length > 0;
  }
  getSelection() {
    return selectionValue ?? "";
  }
  // getPlainText(이슈 #42)가 읽는 buffer.active 최소 스텁. 기본은 빈 버퍼.
  buffer = {
    active: {
      length: 0,
      getLine(_i: number):
        | { translateToString(trimRight?: boolean): string; isWrapped: boolean }
        | undefined {
        return undefined;
      },
    },
  };
  /** Test helper: 버퍼 줄을 세팅한다. translateToString(true)는 실제 xterm처럼
   * 우측 공백을 떼도록 흉내낸다. */
  setBufferLines(lines: Array<{ text: string; isWrapped?: boolean }>) {
    this.buffer = {
      active: {
        length: lines.length,
        getLine(i: number) {
          const l = lines[i];
          if (!l) return undefined;
          return {
            translateToString: (trimRight?: boolean) =>
              trimRight ? l.text.replace(/\s+$/, "") : l.text,
            isWrapped: l.isWrapped ?? false,
          };
        },
      },
    };
  }
}

class FakeFitAddon {
  fit = fitMock;
}

/** Minimal stand-in for `@xterm/addon-serialize`'s SerializeAddon — each
 * instance owns its own `serialize` mock (a fresh `vi.fn()` per `new`) so
 * tests can configure per-agent return values independently. */
class FakeSerializeAddon {
  serialize = vi.fn(() => "");
}

vi.mock("@xterm/xterm", () => ({ Terminal: FakeTerminal }));
vi.mock("@xterm/addon-fit", () => ({ FitAddon: FakeFitAddon }));
vi.mock("@xterm/addon-serialize", () => ({ SerializeAddon: FakeSerializeAddon }));

const writeInput = vi.fn();
const resize = vi.fn();
const onData = vi.fn();
let unsubscribeSpy: ReturnType<typeof vi.fn>;

vi.mock("../../ipc/tauriApi", () => ({
  tauriApi: {
    writeInput: (...args: unknown[]) => writeInput(...args),
    resize: (...args: unknown[]) => resize(...args),
    onData: (...args: unknown[]) => onData(...args),
  },
}));

const clipboardWriteText = vi.fn().mockResolvedValue(undefined);
const clipboardReadText = vi.fn().mockResolvedValue("");

beforeEach(() => {
  vi.resetModules();
  openMock.mockReset();
  disposeMock.mockReset();
  focusMock.mockReset();
  // 콜백 호출 구현을 유지해야 flushAndSerializeAll(§P1)의 write("", cb)가 resolve된다.
  writeMock.mockReset().mockImplementation((_data?: string, cb?: () => void) => cb?.());
  loadAddonMock.mockReset();
  fitMock.mockReset();
  pasteMock.mockReset();
  selectionValue = undefined;
  writeInput.mockReset();
  resize.mockReset();
  onData.mockReset();
  unsubscribeSpy = vi.fn();
  onData.mockReturnValue(unsubscribeSpy);
  // Deterministic, synchronous rAF so activate()'s post-layout fit doesn't
  // need a real animation frame in tests.
  vi.stubGlobal("requestAnimationFrame", (cb: FrameRequestCallback) => {
    cb(0);
    return 0;
  });
  clipboardWriteText.mockReset().mockResolvedValue(undefined);
  clipboardReadText.mockReset().mockResolvedValue("");
  vi.stubGlobal("navigator", {
    ...globalThis.navigator,
    clipboard: { writeText: clipboardWriteText, readText: clipboardReadText },
  });
});

/** Builds a minimal fake keydown/keyup event as attachCustomKeyEventHandler receives it. */
function makeKeyEvent(overrides: Partial<KeyboardEvent> & { key: string }): KeyboardEvent {
  return {
    type: "keydown",
    ctrlKey: false,
    metaKey: false,
    altKey: false,
    shiftKey: false,
    preventDefault: vi.fn(),
    ...overrides,
  } as unknown as KeyboardEvent;
}

async function importRegistry() {
  const mod = await import("../TerminalRegistry");
  return mod.terminalRegistry;
}

describe("keep-alive (T5)", () => {
  it("ensure() returns the same Terminal instance across calls for the same agentId", async () => {
    const terminalRegistry = await importRegistry();
    const e1 = terminalRegistry.ensure("a1");
    const e2 = terminalRegistry.ensure("a1");

    expect(e1.term).toBe(e2.term);
    expect(terminalRegistry.has("a1")).toBe(true);
  });

  it("ensure() only constructs one xterm Terminal per agentId (no re-create, no dispose)", async () => {
    const terminalRegistry = await importRegistry();
    terminalRegistry.ensure("a1");
    terminalRegistry.ensure("a1");
    terminalRegistry.ensure("a1");

    expect(onData).toHaveBeenCalledTimes(1); // one onData subscription total
    expect(disposeMock).not.toHaveBeenCalled();
  });

  it("keeps separate agents independent", async () => {
    const terminalRegistry = await importRegistry();
    const a1 = terminalRegistry.ensure("a1");
    const a2 = terminalRegistry.ensure("a2");

    expect(a1.term).not.toBe(a2.term);
    expect(terminalRegistry.has("a1")).toBe(true);
    expect(terminalRegistry.has("a2")).toBe(true);
  });

  it("destroy() unsubscribes onData and disposes the term, and has() becomes false", async () => {
    const terminalRegistry = await importRegistry();
    terminalRegistry.ensure("a1");

    terminalRegistry.destroy("a1");

    expect(unsubscribeSpy).toHaveBeenCalledTimes(1);
    expect(disposeMock).toHaveBeenCalledTimes(1);
    expect(terminalRegistry.has("a1")).toBe(false);
  });

  it("destroy() on an unknown agentId is a no-op", async () => {
    const terminalRegistry = await importRegistry();
    expect(() => terminalRegistry.destroy("nope")).not.toThrow();
    expect(disposeMock).not.toHaveBeenCalled();
  });

  it("re-ensuring after destroy creates a fresh instance", async () => {
    const terminalRegistry = await importRegistry();
    const e1 = terminalRegistry.ensure("a1");
    terminalRegistry.destroy("a1");
    const e2 = terminalRegistry.ensure("a1");

    expect(e1.term).not.toBe(e2.term);
    expect(terminalRegistry.has("a1")).toBe(true);
  });
});

describe("data wiring direction", () => {
  it("backend push (tauriApi.onData) writes into the terminal", async () => {
    const terminalRegistry = await importRegistry();
    terminalRegistry.ensure("a1");

    const [, backendCb] = onData.mock.calls[0] as [string, (d: string) => void];
    backendCb("hello from pty");

    expect(writeMock).toHaveBeenCalledWith("hello from pty");
  });

  it("user keystrokes (term.onData) call tauriApi.writeInput(agentId, data)", async () => {
    const terminalRegistry = await importRegistry();
    const e = terminalRegistry.ensure("a1");

    (e.term as unknown as FakeTerminal).emitInput("ls\n");

    expect(writeInput).toHaveBeenCalledWith("a1", "ls\n");
  });

  it("subscribes onData for the correct agentId", async () => {
    const terminalRegistry = await importRegistry();
    terminalRegistry.ensure("a1");

    expect(onData).toHaveBeenCalledWith("a1", expect.any(Function));
  });
});

describe("attach", () => {
  it("opens the term into the container exactly once, even across repeated attach calls", async () => {
    const terminalRegistry = await importRegistry();
    const host = document.createElement("div");

    terminalRegistry.attach("a1", host);
    terminalRegistry.attach("a1", host);
    terminalRegistry.attach("a1", host);

    expect(openMock).toHaveBeenCalledTimes(1);
    const entry = terminalRegistry.get("a1");
    expect(entry).toBeDefined();
    expect(host.contains(entry!.container)).toBe(true);
  });

  it("does not re-append the container if it is already connected under the host", async () => {
    const terminalRegistry = await importRegistry();
    const host = document.createElement("div");
    terminalRegistry.attach("a1", host);
    const entry = terminalRegistry.get("a1")!;

    terminalRegistry.attach("a1", host);

    expect(host.children.length).toBe(1);
    expect(entry.container.parentElement).toBe(host);
  });
});

describe("activate / refit", () => {
  it("activate() fits, reports cols/rows, and focuses once opened", async () => {
    const terminalRegistry = await importRegistry();
    const host = document.createElement("div");
    terminalRegistry.attach("a1", host);

    const onResize = vi.fn();
    terminalRegistry.activate("a1", onResize);

    expect(fitMock).toHaveBeenCalledTimes(1);
    expect(onResize).toHaveBeenCalledWith(80, 24);
    expect(focusMock).toHaveBeenCalledTimes(1);
  });

  it("activate() is a no-op if the term has not been attached/opened yet", async () => {
    const terminalRegistry = await importRegistry();
    terminalRegistry.ensure("a1"); // ensure but never attach

    const onResize = vi.fn();
    terminalRegistry.activate("a1", onResize);

    expect(fitMock).not.toHaveBeenCalled();
    expect(onResize).not.toHaveBeenCalled();
    expect(focusMock).not.toHaveBeenCalled();
  });

  it("activate() on an unknown agentId does not throw", async () => {
    const terminalRegistry = await importRegistry();
    expect(() => terminalRegistry.activate("nope", vi.fn())).not.toThrow();
  });

  it("refit() fits and reports cols/rows without focusing", async () => {
    const terminalRegistry = await importRegistry();
    const host = document.createElement("div");
    terminalRegistry.attach("a1", host);

    const onResize = vi.fn();
    terminalRegistry.refit("a1", onResize);

    expect(fitMock).toHaveBeenCalledTimes(1);
    expect(onResize).toHaveBeenCalledWith(80, 24);
    expect(focusMock).not.toHaveBeenCalled();
  });

  it("refit() is a no-op before attach/open", async () => {
    const terminalRegistry = await importRegistry();
    terminalRegistry.ensure("a1");

    const onResize = vi.fn();
    terminalRegistry.refit("a1", onResize);

    expect(fitMock).not.toHaveBeenCalled();
    expect(onResize).not.toHaveBeenCalled();
  });
});

describe("serializeAll (session handoff snapshot)", () => {
  it("returns serialize() output for every live entry, keyed by agentId", async () => {
    const terminalRegistry = await importRegistry();
    const host = document.createElement("div");
    terminalRegistry.attach("a1", host);
    terminalRegistry.attach("a2", host);
    const e1 = terminalRegistry.get("a1")! as unknown as { serialize: FakeSerializeAddon };
    const e2 = terminalRegistry.get("a2")! as unknown as { serialize: FakeSerializeAddon };
    e1.serialize.serialize.mockReturnValue("SCREEN-A1");
    e2.serialize.serialize.mockReturnValue("SCREEN-A2");

    const result = terminalRegistry.serializeAll();

    expect(result).toEqual({ a1: "SCREEN-A1", a2: "SCREEN-A2" });
  });

  it("skips an entry whose serialize() throws, still returning the rest", async () => {
    const terminalRegistry = await importRegistry();
    const host = document.createElement("div");
    terminalRegistry.attach("a1", host);
    terminalRegistry.attach("a2", host);
    const e1 = terminalRegistry.get("a1")! as unknown as { serialize: FakeSerializeAddon };
    const e2 = terminalRegistry.get("a2")! as unknown as { serialize: FakeSerializeAddon };
    e1.serialize.serialize.mockImplementation(() => {
      throw new Error("serialize boom");
    });
    e2.serialize.serialize.mockReturnValue("SCREEN-A2");

    const result = terminalRegistry.serializeAll();

    expect(result).toEqual({ a2: "SCREEN-A2" });
  });

  it("returns an empty object when there are no live terminals", async () => {
    const terminalRegistry = await importRegistry();
    expect(terminalRegistry.serializeAll()).toEqual({});
  });
});

describe("flushAndSerializeAll (broker v2 §P1)", () => {
  it("flushes each terminal's write queue before serializing, keyed by agentId", async () => {
    const terminalRegistry = await importRegistry();
    const host = document.createElement("div");
    terminalRegistry.attach("a1", host);
    terminalRegistry.attach("a2", host);
    const e1 = terminalRegistry.get("a1")! as unknown as { serialize: FakeSerializeAddon };
    const e2 = terminalRegistry.get("a2")! as unknown as { serialize: FakeSerializeAddon };
    e1.serialize.serialize.mockReturnValue("SCREEN-A1");
    e2.serialize.serialize.mockReturnValue("SCREEN-A2");
    writeMock.mockClear();

    const result = await terminalRegistry.flushAndSerializeAll();

    expect(result).toEqual({ a1: "SCREEN-A1", a2: "SCREEN-A2" });
    // 각 터미널마다 flush용 write("", cb)가 한 번씩 호출됐다(콜백이 Promise를 resolve).
    expect(writeMock).toHaveBeenCalledWith("", expect.any(Function));
  });

  it("skips a terminal whose serialize() throws, still returning the rest", async () => {
    const terminalRegistry = await importRegistry();
    const host = document.createElement("div");
    terminalRegistry.attach("a1", host);
    terminalRegistry.attach("a2", host);
    const e1 = terminalRegistry.get("a1")! as unknown as { serialize: FakeSerializeAddon };
    const e2 = terminalRegistry.get("a2")! as unknown as { serialize: FakeSerializeAddon };
    e1.serialize.serialize.mockImplementation(() => {
      throw new Error("serialize boom");
    });
    e2.serialize.serialize.mockReturnValue("SCREEN-A2");

    const result = await terminalRegistry.flushAndSerializeAll();

    expect(result).toEqual({ a2: "SCREEN-A2" });
  });
});

describe("markAdopted / redraw nudge", () => {
  beforeEach(() => {
    vi.useFakeTimers();
  });

  afterEach(() => {
    vi.useRealTimers();
  });

  it("does nothing extra on activate() for an agent that was never marked adopted", async () => {
    const terminalRegistry = await importRegistry();
    const host = document.createElement("div");
    terminalRegistry.attach("a1", host);

    terminalRegistry.activate("a1", vi.fn());

    expect(resize).not.toHaveBeenCalled();
    expect(fitMock).toHaveBeenCalledTimes(1); // just the normal activate() fit
  });

  it("adopted agent: first activate() fits, resizes to rows-1, then re-fits + reports real size after the delay", async () => {
    const terminalRegistry = await importRegistry();
    const host = document.createElement("div");
    terminalRegistry.attach("a1", host);
    terminalRegistry.markAdopted(["a1"]);

    const onResize = vi.fn();
    terminalRegistry.activate("a1", onResize);

    // Normal activate() fit + report already happened synchronously (rAF is sync in tests).
    expect(fitMock).toHaveBeenCalledTimes(1);
    expect(onResize).toHaveBeenCalledWith(80, 24);
    // The nudge fires the resize(cols, rows-1) call right away (before the 50ms wait).
    expect(resize).toHaveBeenCalledWith("a1", 80, 23);

    await vi.advanceTimersByTimeAsync(50);

    // Second fit + onResize call restores the real size — 2 fits, 2 onResize calls total.
    expect(fitMock).toHaveBeenCalledTimes(2);
    expect(onResize).toHaveBeenCalledTimes(2);
    expect(onResize).toHaveBeenLastCalledWith(80, 24);
  });

  it("only nudges once — a second activate() for the same agent does not re-trigger it", async () => {
    const terminalRegistry = await importRegistry();
    const host = document.createElement("div");
    terminalRegistry.attach("a1", host);
    terminalRegistry.markAdopted(["a1"]);

    terminalRegistry.activate("a1", vi.fn());
    await vi.advanceTimersByTimeAsync(50);
    resize.mockClear();

    terminalRegistry.activate("a1", vi.fn());
    await vi.advanceTimersByTimeAsync(50);

    expect(resize).not.toHaveBeenCalled();
  });

  it("keeps agents independent — marking a1 does not nudge a2", async () => {
    const terminalRegistry = await importRegistry();
    const host = document.createElement("div");
    terminalRegistry.attach("a1", host);
    terminalRegistry.attach("a2", host);
    terminalRegistry.markAdopted(["a1"]);

    terminalRegistry.activate("a2", vi.fn());
    await vi.advanceTimersByTimeAsync(50);

    expect(resize).not.toHaveBeenCalled();
  });
});

describe("copy/paste key handling", () => {
  it("Ctrl+C with a selection copies to clipboard and swallows the key (returns false)", async () => {
    const terminalRegistry = await importRegistry();
    const e = terminalRegistry.ensure("a1");
    const fake = e.term as unknown as FakeTerminal;
    selectionValue = "hello";

    const event = makeKeyEvent({ key: "c", ctrlKey: true });
    const result = fake.emitKeyEvent(event);

    expect(clipboardWriteText).toHaveBeenCalledWith("hello");
    expect(event.preventDefault).toHaveBeenCalled();
    expect(result).toBe(false);
  });

  it("Ctrl+Shift+C copies even without a selection guard bypassed by shiftKey", async () => {
    const terminalRegistry = await importRegistry();
    const e = terminalRegistry.ensure("a1");
    const fake = e.term as unknown as FakeTerminal;
    selectionValue = "world";

    const event = makeKeyEvent({ key: "c", ctrlKey: true, shiftKey: true });
    const result = fake.emitKeyEvent(event);

    expect(clipboardWriteText).toHaveBeenCalledWith("world");
    expect(result).toBe(false);
  });

  it("bare Ctrl+C with no selection passes through (SIGINT still reaches the shell)", async () => {
    const terminalRegistry = await importRegistry();
    const e = terminalRegistry.ensure("a1");
    const fake = e.term as unknown as FakeTerminal;
    selectionValue = undefined;

    const event = makeKeyEvent({ key: "c", ctrlKey: true });
    const result = fake.emitKeyEvent(event);

    expect(clipboardWriteText).not.toHaveBeenCalled();
    expect(result).toBe(true);
  });

  it("Ctrl+V reads clipboard, calls term.paste, and swallows the key (returns false)", async () => {
    const terminalRegistry = await importRegistry();
    const e = terminalRegistry.ensure("a1");
    const fake = e.term as unknown as FakeTerminal;
    clipboardReadText.mockResolvedValue("pasted text");

    const event = makeKeyEvent({ key: "v", ctrlKey: true });
    const result = fake.emitKeyEvent(event);

    expect(event.preventDefault).toHaveBeenCalled();
    expect(result).toBe(false);
    await Promise.resolve();
    await Promise.resolve();
    expect(pasteMock).toHaveBeenCalledWith("pasted text");
  });

  it("keyup events are ignored and pass through", async () => {
    const terminalRegistry = await importRegistry();
    const e = terminalRegistry.ensure("a1");
    const fake = e.term as unknown as FakeTerminal;

    const event = makeKeyEvent({ key: "c", ctrlKey: true, type: "keyup" });
    const result = fake.emitKeyEvent(event);

    expect(clipboardWriteText).not.toHaveBeenCalled();
    expect(result).toBe(true);
  });

  it("plain keys without a modifier pass through untouched", async () => {
    const terminalRegistry = await importRegistry();
    const e = terminalRegistry.ensure("a1");
    const fake = e.term as unknown as FakeTerminal;

    const event = makeKeyEvent({ key: "c" });
    const result = fake.emitKeyEvent(event);

    expect(clipboardWriteText).not.toHaveBeenCalled();
    expect(result).toBe(true);
  });
});

describe("Hangul/IME double-input guard", () => {
  it("drops the second of two identical emissions right after compositionend", async () => {
    const terminalRegistry = await importRegistry();
    const host = document.createElement("div");
    terminalRegistry.attach("a1", host);
    const e = terminalRegistry.get("a1")!;
    const fake = e.term as unknown as FakeTerminal;

    fake.emitCompositionEnd();
    fake.emitInput("여");
    fake.emitInput("여");

    expect(writeInput).toHaveBeenCalledTimes(1);
    expect(writeInput).toHaveBeenCalledWith("a1", "여");
  });

  it("ㅋㅋ: repeated distinct compositionend+input pairs are never eaten", async () => {
    const terminalRegistry = await importRegistry();
    const host = document.createElement("div");
    terminalRegistry.attach("a1", host);
    const e = terminalRegistry.get("a1")!;
    const fake = e.term as unknown as FakeTerminal;

    fake.emitCompositionEnd();
    fake.emitInput("ㅋ");
    fake.emitInput("ㅋ"); // dropped duplicate

    expect(writeInput).toHaveBeenCalledTimes(1);

    fake.emitCompositionEnd();
    fake.emitInput("ㅋ");

    expect(writeInput).toHaveBeenCalledTimes(2);
  });

  it("does not affect input with no preceding compositionend (English/key-repeat)", async () => {
    const terminalRegistry = await importRegistry();
    const host = document.createElement("div");
    terminalRegistry.attach("a1", host);
    const e = terminalRegistry.get("a1")!;
    const fake = e.term as unknown as FakeTerminal;

    fake.emitInput("a");
    fake.emitInput("a");

    expect(writeInput).toHaveBeenCalledTimes(2);
  });

  it("a single emission after compositionend is never eaten", async () => {
    const terminalRegistry = await importRegistry();
    const host = document.createElement("div");
    terminalRegistry.attach("a1", host);
    const e = terminalRegistry.get("a1")!;
    const fake = e.term as unknown as FakeTerminal;

    fake.emitCompositionEnd();
    fake.emitInput("여");

    expect(writeInput).toHaveBeenCalledTimes(1);
  });

  it("isComposing keydown passes through the custom key handler untouched", async () => {
    const terminalRegistry = await importRegistry();
    const e = terminalRegistry.ensure("a1");
    const fake = e.term as unknown as FakeTerminal;

    const event = makeKeyEvent({ key: "c", ctrlKey: true, isComposing: true });
    const result = fake.emitKeyEvent(event);

    expect(clipboardWriteText).not.toHaveBeenCalled();
    expect(result).toBe(true);
  });

  it("keyCode 229 (IME composition) keydown passes through the custom key handler untouched", async () => {
    const terminalRegistry = await importRegistry();
    const e = terminalRegistry.ensure("a1");
    const fake = e.term as unknown as FakeTerminal;

    const event = makeKeyEvent({ key: "v", ctrlKey: true, keyCode: 229 });
    const result = fake.emitKeyEvent(event);

    expect(clipboardReadText).not.toHaveBeenCalled();
    expect(result).toBe(true);
  });
});

describe("theme + font options", () => {
  it("constructs the Terminal with the pixel/retro theme and a regular monospace font stack", async () => {
    const { XTERM_THEME } = await import("../theme");
    const terminalRegistry = await importRegistry();

    const e = terminalRegistry.ensure("a1");
    const opts = (e.term as unknown as FakeTerminal).options as {
      theme: unknown;
      fontFamily: string;
    };

    expect(opts.theme).toEqual(XTERM_THEME);
    expect(opts.fontFamily).toContain("SF Mono");
    expect(opts.fontFamily).toContain("Menlo");
    expect(opts.fontFamily).toContain("monospace");
  });
});

describe("getPlainText (이슈 #42)", () => {
  it("아직 만들어지지 않은 터미널은 undefined", async () => {
    const terminalRegistry = await importRegistry();
    expect(terminalRegistry.getPlainText("nope")).toBeUndefined();
  });

  it("일반 줄들을 개행으로 join한다", async () => {
    const terminalRegistry = await importRegistry();
    const e = terminalRegistry.ensure("a1");
    (e.term as unknown as FakeTerminal).setBufferLines([
      { text: "line1" },
      { text: "line2" },
      { text: "line3" },
    ]);
    expect(terminalRegistry.getPlainText("a1")).toBe("line1\nline2\nline3\n");
  });

  it("isWrapped 줄은 앞 줄에 개행 없이 이어붙인다(소프트랩 보존)", async () => {
    const terminalRegistry = await importRegistry();
    const e = terminalRegistry.ensure("a1");
    // 긴 토큰이 소프트랩된 상황: "verylong"이 두 셀 줄로 쪼개져도(뒤 줄
    // isWrapped) 하드 개행 없이 한 줄로 복원돼야 한다.
    (e.term as unknown as FakeTerminal).setBufferLines([
      { text: "very" },
      { text: "long", isWrapped: true },
      { text: "next" },
    ]);
    expect(terminalRegistry.getPlainText("a1")).toBe("verylong\nnext\n");
  });

  it("끝쪽 빈 줄을 트리밍하고 개행 하나로 끝맺는다", async () => {
    const terminalRegistry = await importRegistry();
    const e = terminalRegistry.ensure("a1");
    (e.term as unknown as FakeTerminal).setBufferLines([
      { text: "content" },
      { text: "" },
      { text: "   " }, // translateToString(true)가 공백 줄로 트림
      { text: "" },
    ]);
    expect(terminalRegistry.getPlainText("a1")).toBe("content\n");
  });
});
