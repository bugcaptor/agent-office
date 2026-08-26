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
  // xterm의 `_inputEvent` 가드가 보는 내부 플래그. 실물과 같은 자리에 둔다 —
  // 공개 `Terminal`은 래퍼일 뿐이고 이 플래그는 그 안의 `_core`에 산다(래퍼에
  // 쓰면 조용한 no-op이 된다).
  _core = { _keyDownSeen: false, _keyPressHandled: false };
  /** true면 xterm이 insertText를 통째로 버리는 상황을 흉내 낸다(가드가 무력해진 경우). */
  dropInsertText = false;
  loadAddon = loadAddonMock;
  /**
   * 실제 xterm처럼 textarea를 컨테이너 안에 넣고(=input이 조상으로 올라간다),
   * `_inputEvent`를 흉내 내는 리스너를 **우리 코드보다 먼저** 건다(실물도 open()이
   * 먼저다). insertText만 처리하고, stale 플래그가 서 있으면 통째로 버린다 —
   * 이 버림이 곧 "음절이 통째로 사라지는" 그 버그다.
   */
  open = (el: HTMLElement) => {
    openMock(el);
    el.appendChild(this.textarea);
    this.textarea.addEventListener("input", (ev) => {
      const e = ev as InputEvent;
      if (e.inputType !== "insertText" || !e.data) return;
      if (this.dropInsertText || this._core._keyDownSeen || this._core._keyPressHandled) return;
      this.emitInput(e.data);
    });
  };
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
  /**
   * Test helper: WebKit이 한글 조합을 흘리는 방식 그대로 textarea `input`을 쏜다.
   * 새 음절은 `insertText`, 조합 갱신은 `insertReplacementText`.
   */
  emitTextInput(inputType: string, data: string) {
    this.textarea.dispatchEvent(new InputEvent("input", { inputType, data }));
  }
  /**
   * Test helper: simulate the IME finalizing a composed syllable. `data`를 주면
   * 실제 브라우저처럼 커밋 문자열을 실은 CompositionEvent가 날아간다(워치독이
   * 보는 값). 안 주면 데이터 없는 이벤트 — 워치독은 조용하고 타이밍 가드만 선다.
   */
  emitCompositionEnd(data?: string) {
    this.textarea.dispatchEvent(
      data === undefined
        ? new Event("compositionend")
        : new CompositionEvent("compositionend", { data })
    );
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

/**
 * IME 우회 코드는 플랫폼별로 갈라진다(중복 가드=Windows, 커밋 유실 워치독=macOS).
 * 모듈 로드 시점에 `navigator.platform`을 읽으므로 importRegistry() *전에* 부른다.
 */
function setPlatform(value: string) {
  Object.defineProperty(globalThis.navigator, "platform", { value, configurable: true });
}

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

    const [, backendCb] = onData.mock.calls[0] as [string, (d: string, bytes: number) => void];
    backendCb("hello from pty", 13);

    // §#49: write now takes a completion callback so the renderer can count the
    // rendered raw bytes for snapshot offset accounting.
    expect(writeMock).toHaveBeenCalledWith("hello from pty", expect.any(Function));
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

describe("renderedBytes accumulation (§#49)", () => {
  it("accumulates each chunk's raw byte count on write, keyed by agentId", async () => {
    const terminalRegistry = await importRegistry();
    terminalRegistry.ensure("a1");
    const [, cb] = onData.mock.calls[0] as [string, (d: string, bytes: number) => void];

    cb("ab", 2);
    cb("cde", 3);

    expect(terminalRegistry.getRenderedBytes()).toEqual({ a1: 5 });
  });

  it("does not count bytes=0 restore-snapshot chunks", async () => {
    const terminalRegistry = await importRegistry();
    terminalRegistry.ensure("a1");
    const [, cb] = onData.mock.calls[0] as [string, (d: string, bytes: number) => void];

    cb("RESTORED-SCREEN-IMAGE", 0); // 복원 청크: base가 이미 이 지점을 가리키므로 제외
    cb("live", 4);

    expect(terminalRegistry.getRenderedBytes()).toEqual({ a1: 4 });
  });

  it("keeps agents independent and each new entry starts fresh at 0", async () => {
    const terminalRegistry = await importRegistry();
    terminalRegistry.ensure("a1");
    terminalRegistry.ensure("a2");
    const cbA1 = (onData.mock.calls[0] as [string, (d: string, b: number) => void])[1];
    const cbA2 = (onData.mock.calls[1] as [string, (d: string, b: number) => void])[1];

    cbA1("xxx", 3);
    cbA2("yy", 2);

    expect(terminalRegistry.getRenderedBytes()).toEqual({ a1: 3, a2: 2 });
  });

  it("counts only after the write completion callback runs (render completion, not enqueue)", async () => {
    const terminalRegistry = await importRegistry();
    terminalRegistry.ensure("a1");
    const [, cb] = onData.mock.calls[0] as [string, (d: string, b: number) => void];

    // Hold the completion callback: bytes must not count until the write queue
    // drains (that's the whole point of accounting on the write callback).
    const pending: Array<() => void> = [];
    writeMock.mockImplementationOnce((_d?: string, done?: () => void) => {
      if (done) pending.push(done);
    });

    cb("later", 5);
    expect(terminalRegistry.getRenderedBytes()).toEqual({ a1: 0 });

    pending.forEach((f) => f());
    expect(terminalRegistry.getRenderedBytes()).toEqual({ a1: 5 });
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

  // pi 기본 TUI(regular)는 resize마다 `ESC[2J ESC[H ESC[3J`로 스크롤백까지 지우고
  // 다시 그린다(pi v0.84.2 PTY 실측). nudge는 resize를 두 번 더 쏘므로, 방금 보낸
  // fit 결과가 이미 백엔드 크기와 다르면(=SIGWINCH가 이미 갔으면) 생략해야 한다.
  it("adopted agent: skips the nudge when the fit already changed the size the backend knew", async () => {
    const terminalRegistry = await importRegistry();
    const { useAppStore } = await import("../../store/appStore");
    useAppStore.setState({
      sessions: {
        // FakeTerminal은 80x24로 fit된다 — 백엔드가 알던 크기와 다르므로
        // activate()의 onResize 한 번으로 이미 SIGWINCH가 간다.
        a1: { agentId: "a1", status: "running", cols: 100, rows: 30, lastActivityAt: 0 },
      },
    });
    const host = document.createElement("div");
    terminalRegistry.attach("a1", host);
    terminalRegistry.markAdopted(["a1"]);

    const onResize = vi.fn();
    terminalRegistry.activate("a1", onResize);
    await vi.advanceTimersByTimeAsync(50);

    expect(onResize).toHaveBeenCalledTimes(1);
    expect(onResize).toHaveBeenCalledWith(80, 24);
    expect(resize).not.toHaveBeenCalled(); // rows-1 nudge 없음
    expect(fitMock).toHaveBeenCalledTimes(1);
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

describe("Hangul/IME double-input guard (Windows 전용)", () => {
  // WebView2 이중 입력 가드다. macOS에는 그 중복이 없고 오히려 진짜 입력을
  // 잡아먹으므로, 이 describe는 non-mac을 명시적으로 깔고 돈다.
  beforeEach(() => {
    setPlatform("Win32");
  });

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

describe("WebKit 한글 조합 미러링", () => {
  // macOS WebKit은 한글 IME에 composition 이벤트를 안 쏘고, 조합 갱신을
  // input(insertReplacementText)으로만 흘린다. xterm 5.5는 insertText만 처리하므로
  // 각 음절의 첫 자모만 나가고 나머지는 증발한다 — 그 갱신을 우리가 미러링한다.

  /** PTY로 나간 청크들을 실제 줄로 되돌린다(DEL은 한 글자 지움). */
  function applyToLine(chunks: string[]): string {
    const line: string[] = [];
    for (const ch of chunks.join("")) {
      if (ch === "\x7f") line.pop();
      else line.push(ch);
    }
    return line.join("");
  }

  function sentChunks(): string[] {
    return writeInput.mock.calls.map((c) => c[1] as string);
  }

  function mount() {
    const host = document.createElement("div");
    return { host };
  }

  it("insertText(새 음절)는 xterm이 보내고, 우리는 한 번 더 보내지 않는다", async () => {
    const terminalRegistry = await importRegistry();
    const { host } = mount();
    terminalRegistry.attach("a1", host);
    const fake = terminalRegistry.get("a1")!.term as unknown as FakeTerminal;

    fake.emitTextInput("insertText", "ㅎ");

    expect(sentChunks()).toEqual(["ㅎ"]);
  });

  it("조합 갱신은 DEL로 지우고 새 꼴을 쓴다", async () => {
    const terminalRegistry = await importRegistry();
    const { host } = mount();
    terminalRegistry.attach("a1", host);
    const fake = terminalRegistry.get("a1")!.term as unknown as FakeTerminal;

    fake.emitTextInput("insertText", "ㅎ"); // xterm이 "ㅎ"를 보낸 상태
    fake.emitTextInput("insertReplacementText", "하");
    fake.emitTextInput("insertReplacementText", "한");

    // 한 번의 write로 묶어 보낸다 — 중간 상태가 깜빡이지 않게.
    expect(sentChunks()).toEqual(["ㅎ", "\x7f하", "\x7f한"]);
  });

  it('"한글": WebKit 실측 시퀀스를 그대로 먹이면 줄이 정확히 복원된다', async () => {
    const terminalRegistry = await importRegistry();
    const { host } = mount();
    terminalRegistry.attach("a1", host);
    const fake = terminalRegistry.get("a1")!.term as unknown as FakeTerminal;

    // Safari 트레이스 그대로: 새 음절은 insertText, 갱신은 insertReplacementText.
    // insertText는 xterm이 PTY로 보내므로 테스트도 같이 흉내 낸다.
    const feed = (type: string, data: string) => fake.emitTextInput(type, data);
    feed("insertText", "ㅎ");
    feed("insertReplacementText", "하");
    feed("insertReplacementText", "한");
    feed("insertReplacementText", "한"); // 받침이 빠지기 직전 되돌림
    feed("insertText", "ㄱ");
    feed("insertReplacementText", "그");
    feed("insertReplacementText", "글");

    expect(applyToLine(sentChunks())).toBe("한글");
  });

  it("받침이 다음 음절로 넘어가도 줄이 정확하다", async () => {
    const terminalRegistry = await importRegistry();
    const { host } = mount();
    terminalRegistry.attach("a1", host);
    const fake = terminalRegistry.get("a1")!.term as unknown as FakeTerminal;

    const feed = (type: string, data: string) => fake.emitTextInput(type, data);
    // "한가": 한 -> 한ㄱ -> (ㄱ이 다음 음절로) 한/가
    feed("insertText", "ㅎ");
    feed("insertReplacementText", "하");
    feed("insertReplacementText", "한");
    feed("insertReplacementText", "한");
    feed("insertText", "ㄱ");
    feed("insertReplacementText", "가");

    expect(applyToLine(sentChunks())).toBe("한가");
  });

  it("조합 키가 아닌 keydown은 꼬리 추적을 접는다(Enter 뒤 갱신은 DEL 없이 쓴다)", async () => {
    const terminalRegistry = await importRegistry();
    const { host } = mount();
    terminalRegistry.attach("a1", host);
    const fake = terminalRegistry.get("a1")!.term as unknown as FakeTerminal;

    fake.emitTextInput("insertText", "ㅎ");
    fake.emitKeyEvent(makeKeyEvent({ key: "Enter", keyCode: 13 }));
    fake.emitTextInput("insertReplacementText", "하");

    expect(sentChunks()).toEqual(["ㅎ", "하"]);
  });

  it("조합 키(keyCode 229) keydown은 꼬리를 지우지 않는다", async () => {
    const terminalRegistry = await importRegistry();
    const { host } = mount();
    terminalRegistry.attach("a1", host);
    const fake = terminalRegistry.get("a1")!.term as unknown as FakeTerminal;

    fake.emitTextInput("insertText", "ㅎ");
    fake.emitKeyEvent(makeKeyEvent({ key: "ㅏ", keyCode: 229 }));
    fake.emitTextInput("insertReplacementText", "하");

    expect(sentChunks()).toEqual(["ㅎ", "\x7f하"]);
  });

  it("composition 이벤트가 오는 플랫폼에서는 비켜난다(xterm 몫)", async () => {
    const terminalRegistry = await importRegistry();
    const { host } = mount();
    terminalRegistry.attach("a1", host);
    const fake = terminalRegistry.get("a1")!.term as unknown as FakeTerminal;

    fake.textarea.dispatchEvent(new CompositionEvent("compositionstart", { data: "" }));
    fake.emitTextInput("insertReplacementText", "하");

    expect(writeInput).not.toHaveBeenCalled();
  });

  it("붙여넣기 같은 다른 inputType 뒤에는 DEL을 쏘지 않는다", async () => {
    const terminalRegistry = await importRegistry();
    const { host } = mount();
    terminalRegistry.attach("a1", host);
    const fake = terminalRegistry.get("a1")!.term as unknown as FakeTerminal;

    fake.emitTextInput("insertText", "ㅎ");
    fake.emitTextInput("insertFromPaste", "붙여넣기");
    fake.emitTextInput("insertReplacementText", "하");

    expect(sentChunks()).toEqual(["ㅎ", "하"]);
  });

  it("안전망: xterm이 insertText를 흘려도 음절이 살아남는다", async () => {
    const terminalRegistry = await importRegistry();
    const { host } = mount();
    terminalRegistry.attach("a1", host);
    const fake = terminalRegistry.get("a1")!.term as unknown as FakeTerminal;

    // 가드 청소가 무력해진 상황(예: 다음 xterm에서 내부 필드 이름이 바뀜).
    fake.dropInsertText = true;
    fake.emitTextInput("insertText", "ㅎ");
    fake.emitTextInput("insertReplacementText", "하");

    expect(sentChunks()).toEqual(["ㅎ", "\x7f하"]);
  });

  it("조합 input이 xterm에 닿기 전에 stale 가드 플래그를 내린다", async () => {
    const terminalRegistry = await importRegistry();
    const { host } = mount();
    terminalRegistry.attach("a1", host);
    const fake = terminalRegistry.get("a1")!.term as unknown as FakeTerminal;

    // 앞 키를 아직 안 뗀 상태(_keyDownSeen) + 직전에 ASCII를 친 상태(_keyPressHandled).
    // 둘 다 xterm의 `_inputEvent`가 insertText를 통째로 버리게 만든다.
    fake._core._keyDownSeen = true;
    fake._core._keyPressHandled = true;
    fake.emitTextInput("insertText", "ㅌ");

    expect(fake._core._keyDownSeen).toBe(false);
    expect(fake._core._keyPressHandled).toBe(false);
  });

  it("ASCII 본인의 input(직전 keypress)은 xterm 플래그를 건드리지 않는다", async () => {
    const terminalRegistry = await importRegistry();
    const { host } = mount();
    terminalRegistry.attach("a1", host);
    const fake = terminalRegistry.get("a1")!.term as unknown as FakeTerminal;

    // ASCII는 keydown -> keypress -> input 순서. keypress로 이미 보냈으므로
    // `_keyPressHandled`를 내리면 같은 글자가 두 번 나간다.
    fake.emitKeyEvent(makeKeyEvent({ key: "a", keyCode: 65 }));
    fake.emitKeyEvent(makeKeyEvent({ key: "a", type: "keypress" } as never));
    fake._core._keyPressHandled = true; // xterm이 keypress로 이미 보냈다는 표시
    fake.emitTextInput("insertText", "a");

    expect(fake._core._keyPressHandled).toBe(true);
    expect(writeInput).not.toHaveBeenCalled(); // 우리가 덧붙이지 않는다
  });

  it("composition 이벤트를 쏘는 플랫폼에서는 xterm 플래그에 손대지 않는다", async () => {
    const terminalRegistry = await importRegistry();
    const { host } = mount();
    terminalRegistry.attach("a1", host);
    const fake = terminalRegistry.get("a1")!.term as unknown as FakeTerminal;

    fake.textarea.dispatchEvent(new CompositionEvent("compositionstart", { data: "" }));
    fake.textarea.dispatchEvent(new CompositionEvent("compositionend", { data: "안" }));
    fake._core._keyDownSeen = true;
    fake.emitTextInput("insertText", "ㅌ");

    expect(fake._core._keyDownSeen).toBe(true);
  });

  it("봇 운전 중이면 조합 갱신도 나가지 않는다", async () => {
    const terminalRegistry = await importRegistry();
    const { useAppStore } = await import("../../store/appStore");
    useAppStore.setState({ botMode: { a1: { agentId: "a1" } as never } });
    const { host } = mount();
    terminalRegistry.attach("a1", host);
    const fake = terminalRegistry.get("a1")!.term as unknown as FakeTerminal;

    fake.emitTextInput("insertText", "ㅎ");
    fake.emitTextInput("insertReplacementText", "하");

    expect(writeInput).not.toHaveBeenCalled();
  });
});

describe("theme + font options", () => {
  it("constructs the Terminal with the current effective palette and a regular monospace font stack", async () => {
    const { resolveXtermTheme } = await import("../theme");
    const { useAppStore } = await import("../../store/appStore");
    const terminalRegistry = await importRegistry();

    const s = useAppStore.getState();
    const e = terminalRegistry.ensure("a1");
    const opts = (e.term as unknown as FakeTerminal).options as {
      theme: unknown;
      fontFamily: string;
    };

    expect(opts.theme).toEqual(resolveXtermTheme(s.theme, s.xtermTheme));
    expect(opts.fontFamily).toContain("SF Mono");
    expect(opts.fontFamily).toContain("Menlo");
    expect(opts.fontFamily).toContain("monospace");
  });

  it("setTheme은 살아있는 전 인스턴스를 재도색하고, 이후 만들어지는 터미널에도 적용된다", async () => {
    const { THEMES } = await import("../../theme/themes");
    const terminalRegistry = await importRegistry();

    const a = terminalRegistry.ensure("a1");
    const b = terminalRegistry.ensure("a2");

    terminalRegistry.setTheme(THEMES.pipboy.xterm);

    for (const e of [a, b]) {
      const opts = (e.term as unknown as FakeTerminal).options as { theme: unknown };
      expect(opts.theme).toEqual(THEMES.pipboy.xterm);
    }

    // 전환 이후 생성분도 같은 팔레트로 태어난다(재도색을 기다리지 않는다).
    const c = terminalRegistry.ensure("a3");
    expect((c.term as unknown as FakeTerminal).options).toMatchObject({
      theme: THEMES.pipboy.xterm,
    });
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
