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
// xterm 사적 API: 숨은 동안 오염된 뷰포트 지오메트리를 되돌리는 강제 재동기화.
const syncScrollAreaMock = vi.fn();
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
  /**
   * xterm의 textarea `input` 리스너(=`_inputEvent`)가 실제로 이 이벤트를 받은
   * 횟수. 우리 코드가 컨테이너 캡처 단계에서 `stopPropagation()`으로 xterm을
   * 비켜 세웠는지 보는 창이다.
   */
  xtermInputSeen = 0;
  /** 실물 `_keyPressHandled`: keypress로 이미 보냈다는 표시. 서면 input을 버린다. */
  keyPressHandled = false;
  loadAddon = loadAddonMock;
  /** 실물 xterm의 사적 내부 — TerminalRegistry.syncViewport가 이 경로로 들어온다. */
  _core = { viewport: { syncScrollArea: syncScrollAreaMock } };
  /**
   * 실제 xterm처럼 textarea를 컨테이너 안에 넣고(=input이 조상을 거쳐 내려온다),
   * `_inputEvent`를 흉내 내는 리스너를 capture=true로, **우리 코드보다 먼저**
   * 건다(실물도 open()이 먼저다). insertText만 동기로 그대로 쏜다.
   */
  open = (el: HTMLElement) => {
    openMock(el);
    el.appendChild(this.textarea);
    this.textarea.addEventListener(
      "input",
      (ev) => {
        this.xtermInputSeen++;
        const e = ev as InputEvent;
        if (e.inputType !== "insertText" || !e.data) return;
        if (this.keyPressHandled) return;
        this.emitInput(e.data);
      },
      true,
    );
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
   * Test helper: WebKit이 한글 조합을 흘리는 방식 그대로 — textarea.value를 **먼저**
   * 새 값으로 바꾼 뒤 `input`을 쏜다. 미러가 보는 것은 inputType이 아니라 이 value다.
   */
  emitTextInput(inputType: string, data: string | null, value?: string) {
    if (value !== undefined) this.textarea.value = value;
    this.textarea.dispatchEvent(
      new InputEvent("input", { inputType, data, bubbles: true }),
    );
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
        : new CompositionEvent("compositionend", { data }),
    );
  }
  attachCustomKeyEventHandler(handler: (event: KeyboardEvent) => boolean) {
    this.keyEventHandler = handler;
  }
  /**
   * Test helper: simulate a key event reaching xterm's custom handler.
   * 실물 xterm은 커스텀 핸들러가 통과시킨 Enter/Ctrl+C keydown에서 textarea를
   * 비운다(browser/Terminal.ts `_keyDown`) — 미러의 기준선 리싱크가 이 비우기를
   * 제대로 따라잡는지 보려면 목도 같이 비워야 한다.
   */
  emitKeyEvent(event: KeyboardEvent): boolean | undefined {
    const result = this.keyEventHandler?.(event);
    if (
      result !== false &&
      event.type === "keydown" &&
      (event.key === "Enter" ||
        (event.ctrlKey && event.key.toLowerCase() === "c"))
    ) {
      this.textarea.value = "";
    }
    return result;
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
      getLine(
        _i: number,
      ):
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
vi.mock("@xterm/addon-serialize", () => ({
  SerializeAddon: FakeSerializeAddon,
}));

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
  writeMock
    .mockReset()
    .mockImplementation((_data?: string, cb?: () => void) => cb?.());
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
  Object.defineProperty(globalThis.navigator, "platform", {
    value,
    configurable: true,
  });
}

/** Builds a minimal fake keydown/keyup event as attachCustomKeyEventHandler receives it. */
function makeKeyEvent(
  overrides: Partial<KeyboardEvent> & { key: string },
): KeyboardEvent {
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

    const [, backendCb] = onData.mock.calls[0] as [
      string,
      (d: string, bytes: number) => void,
    ];
    backendCb("hello from pty", 13);

    // §#49: write now takes a completion callback so the renderer can count the
    // rendered raw bytes for snapshot offset accounting.
    expect(writeMock).toHaveBeenCalledWith(
      "hello from pty",
      expect.any(Function),
    );
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

  it("activate() forces a viewport re-sync so a terminal hidden during output can scroll to the bottom again", async () => {
    // display:none인 동안 xterm Viewport가 offsetHeight 0으로 스크롤 영역 높이를
    // 짧게 굳혀 버린다(xterm.js #494). fit()이 같은 크기를 내면 xterm 내부
    // syncScrollArea가 안 불리므로 여기서 강제로 부른다.
    const terminalRegistry = await importRegistry();
    const host = document.createElement("div");
    terminalRegistry.attach("a1", host);

    terminalRegistry.activate("a1", vi.fn());

    expect(syncScrollAreaMock).toHaveBeenCalledWith(true);
  });

  it("refit() forces a viewport re-sync too", async () => {
    const terminalRegistry = await importRegistry();
    const host = document.createElement("div");
    terminalRegistry.attach("a1", host);

    terminalRegistry.refit("a1", vi.fn());

    expect(syncScrollAreaMock).toHaveBeenCalledWith(true);
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
    const e1 = terminalRegistry.get("a1")! as unknown as {
      serialize: FakeSerializeAddon;
    };
    const e2 = terminalRegistry.get("a2")! as unknown as {
      serialize: FakeSerializeAddon;
    };
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
    const e1 = terminalRegistry.get("a1")! as unknown as {
      serialize: FakeSerializeAddon;
    };
    const e2 = terminalRegistry.get("a2")! as unknown as {
      serialize: FakeSerializeAddon;
    };
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
    const e1 = terminalRegistry.get("a1")! as unknown as {
      serialize: FakeSerializeAddon;
    };
    const e2 = terminalRegistry.get("a2")! as unknown as {
      serialize: FakeSerializeAddon;
    };
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
    const e1 = terminalRegistry.get("a1")! as unknown as {
      serialize: FakeSerializeAddon;
    };
    const e2 = terminalRegistry.get("a2")! as unknown as {
      serialize: FakeSerializeAddon;
    };
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
    const [, cb] = onData.mock.calls[0] as [
      string,
      (d: string, bytes: number) => void,
    ];

    cb("ab", 2);
    cb("cde", 3);

    expect(terminalRegistry.getRenderedBytes()).toEqual({ a1: 5 });
  });

  it("does not count bytes=0 restore-snapshot chunks", async () => {
    const terminalRegistry = await importRegistry();
    terminalRegistry.ensure("a1");
    const [, cb] = onData.mock.calls[0] as [
      string,
      (d: string, bytes: number) => void,
    ];

    cb("RESTORED-SCREEN-IMAGE", 0); // 복원 청크: base가 이미 이 지점을 가리키므로 제외
    cb("live", 4);

    expect(terminalRegistry.getRenderedBytes()).toEqual({ a1: 4 });
  });

  it("keeps agents independent and each new entry starts fresh at 0", async () => {
    const terminalRegistry = await importRegistry();
    terminalRegistry.ensure("a1");
    terminalRegistry.ensure("a2");
    const cbA1 = (
      onData.mock.calls[0] as [string, (d: string, b: number) => void]
    )[1];
    const cbA2 = (
      onData.mock.calls[1] as [string, (d: string, b: number) => void]
    )[1];

    cbA1("xxx", 3);
    cbA2("yy", 2);

    expect(terminalRegistry.getRenderedBytes()).toEqual({ a1: 3, a2: 2 });
  });

  it("counts only after the write completion callback runs (render completion, not enqueue)", async () => {
    const terminalRegistry = await importRegistry();
    terminalRegistry.ensure("a1");
    const [, cb] = onData.mock.calls[0] as [
      string,
      (d: string, b: number) => void,
    ];

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
        a1: {
          agentId: "a1",
          status: "running",
          cols: 100,
          rows: 30,
          lastActivityAt: 0,
        },
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

  it("mac이 아니면 textarea input에 손대지 않는다(xterm/IME 몫)", async () => {
    const terminalRegistry = await importRegistry();
    const host = document.createElement("div");
    terminalRegistry.attach("a1", host);
    const fake = terminalRegistry.get("a1")!.term as unknown as FakeTerminal;

    fake.emitTextInput("insertText", "ㅎ", "ㅎ");
    fake.emitTextInput("insertReplacementText", "하", "하");

    // 캡처 단계에서 끊지 않았고(둘 다 xterm에 닿았다), 미러가 덧붙이지도 않았다.
    expect(fake.xtermInputSeen).toBe(2);
    expect(writeInput.mock.calls.map((c) => c[1])).toEqual(["ㅎ"]);
  });
});

describe("macOS WebKit 한글 조합 미러링", () => {
  // macOS WebKit은 한글 IME에 composition 이벤트를 안 쏘고, 조합을 hidden
  // textarea의 value 변화 + input 이벤트로만 흘린다. 게다가 xterm에는 세 번째
  // writer(`CompositionHelper._handleAnyTextareaChanges`)가 있어 "조합 중이
  // 아닌데 온 keyCode 229 keydown"마다 setTimeout(0)으로 제 나름의 결론을 쏜다.
  // 그래서 macOS에서는 우리가 유일한 writer가 되도록 xterm의 두 경로를 막고,
  // textarea.value 전체 diff로 미러링한다.
  beforeEach(() => {
    setPlatform("MacIntel");
  });

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

  async function mountMac() {
    const terminalRegistry = await importRegistry();
    const host = document.createElement("div");
    terminalRegistry.attach("a1", host);
    const fake = terminalRegistry.get("a1")!.term as unknown as FakeTerminal;
    /** WebKit 트레이스 한 줄: (inputType, data, 그 시점의 textarea.value). */
    const feed = (inputType: string, data: string | null, value: string) =>
      fake.emitTextInput(inputType, data, value);
    /** 키 하나의 keydown. 원장이 여기서 비워진다. */
    const keyDown = (key: string, keyCode: number) =>
      fake.emitKeyEvent(makeKeyEvent({ key, keyCode }));
    /**
     * xterm의 `_keyPress`: 커스텀 핸들러를 거친 뒤 charCode를 그대로
     * `triggerDataEvent`로 흘린다(= term.onData). 스페이스(32)가 이 경로로 나간다.
     */
    const xtermKeyPress = (key: string) => {
      fake.emitKeyEvent(makeKeyEvent({ key, type: "keypress" } as never));
      fake.emitInput(key);
    };
    return { terminalRegistry, fake, feed, keyDown, xtermKeyPress };
  }

  it('"한글": WebKit 실측 시퀀스를 그대로 먹이면 PTY 누적 결과가 정확히 "한글"', async () => {
    const { feed } = await mountMac();

    feed("insertText", "ㅎ", "ㅎ");
    feed("insertReplacementText", "하", "하");
    feed("insertReplacementText", "한", "한");
    feed("insertText", "ㄱ", "한ㄱ");
    feed("insertReplacementText", "그", "한그");
    feed("insertReplacementText", "글", "한글");

    // 공통 접두사는 건드리지 않고 바뀐 꼬리만 한 번의 write로 나간다.
    expect(sentChunks()).toEqual([
      "ㅎ",
      "\x7f하",
      "\x7f한",
      "ㄱ",
      "\x7f그",
      "\x7f글",
    ]);
    expect(applyToLine(sentChunks())).toBe("한글");
  });

  it('"계": 앞 자모가 줄에 남지 않는다(증상 "성ㄱ계" 회귀)', async () => {
    const { feed } = await mountMac();

    feed("insertText", "ㄱ", "ㄱ");
    feed("insertReplacementText", "계", "계");

    expect(sentChunks()).toEqual(["ㄱ", "\x7f계"]);
    expect(applyToLine(sentChunks())).toBe("계");
  });

  it("문장부호 커밋: 커밋 input이 keydown보다 먼저 와도 음절이 두 번 나가지 않는다", async () => {
    const { fake, feed } = await mountMac();
    feed("insertText", "ㅎ", "ㅎ");
    feed("insertReplacementText", "해", "해");

    // WebKit 실측: 조합의 input이 keydown보다 먼저 도착하는 순서.
    feed("insertText", "?", "해?");
    fake.emitKeyEvent(makeKeyEvent({ key: "?", keyCode: 229 }));

    expect(applyToLine(sentChunks())).toBe("해?"); // "해해?"가 아니다
  });

  it("문장부호 커밋: keydown이 먼저 와도 음절이 두 번 나가지 않는다", async () => {
    const { fake, feed } = await mountMac();
    feed("insertText", "ㅎ", "ㅎ");
    feed("insertReplacementText", "해", "해");

    // 순서가 뒤집히는 것이 이번 버그의 핵심이라 반대 순서도 고정한다.
    fake.emitKeyEvent(makeKeyEvent({ key: "?", keyCode: 229 }));
    feed("insertText", "?", "해?");

    expect(applyToLine(sentChunks())).toBe("해?");
  });

  it('"ㅋㅋ" 같은 같은 음절 연타가 먹히지 않는다', async () => {
    const { feed, keyDown } = await mountMac();

    keyDown("ㅋ", 229);
    feed("insertText", "ㅋ", "ㅋ");
    keyDown("ㅋ", 229);
    feed("insertText", "ㅋ", "ㅋㅋ");

    expect(sentChunks()).toEqual(["ㅋ", "ㅋ"]);
    expect(applyToLine(sentChunks())).toBe("ㅋㅋ");
  });

  it("조합 중 Backspace(229로 도착)가 DEL 한 번으로 미러된다", async () => {
    const { fake, feed } = await mountMac();
    feed("insertText", "ㅎ", "ㅎ");
    feed("insertReplacementText", "하", "하");
    feed("insertReplacementText", "한", "한");
    writeInput.mockClear();

    fake.emitKeyEvent(makeKeyEvent({ key: "Backspace", keyCode: 229 }));
    fake.emitTextInput("deleteContentBackward", null, "하");

    const patch = sentChunks().join("");
    expect([...patch].filter((c) => c === "\x7f")).toHaveLength(1);
    expect(applyToLine(["한", ...sentChunks()])).toBe("하");
  });

  it("Enter 뒤 xterm이 textarea를 비워도 다음 조합에서 DEL이 새어 나가지 않는다", async () => {
    const { fake, feed } = await mountMac();
    feed("insertText", "ㅎ", "ㅎ");

    // 실물 xterm은 Enter keydown에서 `textarea.value = ''`을 한다(목이 흉내 낸다).
    fake.emitKeyEvent(makeKeyEvent({ key: "Enter", keyCode: 13 }));
    await Promise.resolve(); // 기준선 리싱크(queueMicrotask)
    writeInput.mockClear();

    feed("insertText", "ㅎ", "ㅎ");

    expect(sentChunks()).toEqual(["ㅎ"]); // DEL 없음 — 진짜 프롬프트를 지우지 않는다
  });

  it("blur 뒤에도 DEL이 새어 나가지 않는다", async () => {
    const { fake, feed } = await mountMac();
    feed("insertText", "ㅎ", "ㅎ");

    fake.textarea.value = ""; // xterm의 blur 핸들러가 비운다
    fake.textarea.dispatchEvent(new Event("blur"));
    await Promise.resolve();
    writeInput.mockClear();

    feed("insertText", "ㅎ", "ㅎ");

    expect(sentChunks()).toEqual(["ㅎ"]);
  });

  it("keyCode 229 keydown에서 커스텀 핸들러가 false를 반환한다(_handleAnyTextareaChanges 차단)", async () => {
    const { fake } = await mountMac();

    // 이 false 하나가 `Terminal._keyDown`을 `_compositionHelper.keydown()` 앞에서
    // 끊는다 — 세 번째 writer가 아예 돌지 않는다.
    expect(fake.emitKeyEvent(makeKeyEvent({ key: "ㅎ", keyCode: 229 }))).toBe(
      false,
    );
  });

  it("우리가 소유한 input은 xterm의 `_inputEvent`에 닿지 않는다", async () => {
    const { fake, feed } = await mountMac();

    feed("insertText", "ㅎ", "ㅎ");

    expect(fake.xtermInputSeen).toBe(0); // 캡처 단계에서 끊었다
    expect(sentChunks()).toEqual(["ㅎ"]); // 그래도 한 번은 나갔다
  });

  it("진짜 composition 이벤트가 오는 IME에서는 손을 뗀다(xterm 몫)", async () => {
    const { fake, feed } = await mountMac();

    fake.textarea.dispatchEvent(
      new CompositionEvent("compositionstart", { data: "" }),
    );
    feed("insertText", "あ", "あ");

    expect(fake.xtermInputSeen).toBe(1); // stopPropagation 하지 않았다
    expect(sentChunks()).toEqual(["あ"]); // xterm이 보낸 한 번뿐
  });

  it("조합 중(composition 이벤트가 오는 IME) 229 keydown은 막지 않는다", async () => {
    const { fake } = await mountMac();

    // 여기서 막으면 조합 중 Enter가 통째로 증발한다 — xterm이 조합을 굴리게 둔다.
    fake.textarea.dispatchEvent(
      new CompositionEvent("compositionstart", { data: "" }),
    );

    expect(fake.emitKeyEvent(makeKeyEvent({ key: "a", keyCode: 229 }))).toBe(
      true,
    );
  });

  it("ASCII 본인의 input은 xterm의 keypress와 상쇄돼 한 번만 나간다", async () => {
    const { fake, keyDown, xtermKeyPress } = await mountMac();

    // ASCII는 keydown -> keypress -> input 순서. keypress로 xterm이 이미 보냈다.
    keyDown("a", 65);
    xtermKeyPress("a");
    fake.emitTextInput("insertText", "a", "a");

    expect(sentChunks()).toEqual(["a"]);
  });

  it('xterm이 보낸 것과 다른 청크는 상쇄되지 않는다(실측 트레이스 keypress "," -> input "ㄹ")', async () => {
    const { fake, keyDown, xtermKeyPress } = await mountMac();

    // WebKit 실측: keypress ","로 xterm이 ","를 보낸 뒤, 다음 조합의 input이
    // 그 키의 keydown보다 **먼저** 도착한다. 상쇄는 같은 문자열 1회뿐이라
    // "ㄹ"은 반드시 살아남아야 한다.
    keyDown(",", 188);
    xtermKeyPress(",");
    fake.emitTextInput("insertText", "ㄹ", "ㄹ");

    expect(sentChunks()).toEqual([",", "ㄹ"]);
  });

  it("조합 뒤 스페이스: input이 keypress보다 **먼저** 와도 한 칸만 들어간다", async () => {
    const { feed, keyDown, xtermKeyPress } = await mountMac();
    feed("insertText", "ㅎ", "ㅎ");
    feed("insertReplacementText", "해", "해");
    writeInput.mockClear();

    // 스페이스(32)는 `evaluateKeyboardEvent`의 `keyCode >= 48` 조건에 걸려
    // `_keyDown`을 그냥 통과하고 `_keyPress`로 나간다. 그런데 조합을 커밋하는
    // 스페이스의 input은 그 keypress보다 먼저 온다 — 이게 "두 칸" 버그였다.
    keyDown(" ", 32);
    feed("insertText", " ", "해 ");
    xtermKeyPress(" ");

    expect(sentChunks()).toEqual([" "]);
  });

  it("조합 뒤 스페이스: keypress가 먼저 와도 한 칸만 들어간다", async () => {
    const { feed, keyDown, xtermKeyPress } = await mountMac();
    feed("insertText", "ㅎ", "ㅎ");
    feed("insertReplacementText", "해", "해");
    writeInput.mockClear();

    keyDown(" ", 32);
    xtermKeyPress(" ");
    feed("insertText", " ", "해 ");

    expect(sentChunks()).toEqual([" "]);
  });

  it("영문 상태의 스페이스 연타는 친 만큼 들어간다", async () => {
    const { keyDown, xtermKeyPress } = await mountMac();

    // input 이벤트 없이 keypress만 오는 평범한 경로. 원장이 keydown마다 비므로
    // 같은 " "가 연달아 와도 상쇄되지 않는다.
    keyDown(" ", 32);
    xtermKeyPress(" ");
    keyDown(" ", 32);
    xtermKeyPress(" ");

    expect(sentChunks()).toEqual([" ", " "]);
  });

  it("조합 뒤 스페이스를 연달아 쳐도 친 만큼 들어간다", async () => {
    const { feed, keyDown, xtermKeyPress } = await mountMac();
    feed("insertText", "ㅎ", "ㅎ");
    feed("insertReplacementText", "해", "해");
    writeInput.mockClear();

    keyDown(" ", 32);
    feed("insertText", " ", "해 ");
    xtermKeyPress(" ");
    keyDown(" ", 32);
    feed("insertText", " ", "해  ");
    xtermKeyPress(" ");

    expect(sentChunks()).toEqual([" ", " "]);
  });

  // ── NBSP 정규화 회귀(§normalizeSpace) ────────────────────────────────────
  // 앱 실측 트레이스(수정 전): WKWebView는 textarea 줄 끝 공백을 U+00A0(NBSP)로
  // 채운다. xterm의 `_keyPress`가 보내는 건 평범한 U+0020이라, 정규화 없이
  // 문자 그대로 비교/diff하면 (1) 원장 상쇄가 실패해 공백이 두 칸 들어가고,
  // (2) 다음 음절의 diff에서 접두사가 어긋나 공백이 패치에 또 딸려 나온다.
  //   keydown " " keyCode=32          → 원장 초기화
  //   xterm.onData " "  → PTY " "     ← U+0020 (정상, xterm의 _keyPress)
  //   mirror patch=" " xtermSent=" "  → 상쇄 실패 → PTY 한 번 더   ← 공백 두 칸
  //   mirror patch=" ㄴ" prev="가 " next="가 ㄴ"  ← 접두사가 1자만 일치
  // 아래 테스트들은 이 트레이스를 textarea에 **실제 U+00A0**을 넣어 재현한다.
  it('조합 뒤 스페이스: textarea가 NBSP(U+00A0)로 채워져도 PTY 공백은 정확히 한 칸(앱 실측 트레이스)', async () => {
    const { feed, keyDown, xtermKeyPress } = await mountMac();
    feed("insertText", "가", "가");
    writeInput.mockClear();

    keyDown(" ", 32);
    xtermKeyPress(" "); // xterm _keyPress: term.onData로 U+0020 " "를 먼저 보냄
    feed("insertText", " ", "가 "); // WKWebView: 실제 값은 NBSP

    expect(sentChunks()).toEqual([" "]); // 정규화로 상쇄 성공 -> 공백 두 칸이 아니다
  });

  it('그 다음 음절: prevValue가 NBSP 정규화된 "가 "일 때 "가 ㄴ"의 패치는 공백 없이 "ㄴ"뿐이다(회귀: "성ㄱ계"류 접두사 어긋남)', async () => {
    const { feed, keyDown, xtermKeyPress } = await mountMac();
    feed("insertText", "가", "가");
    keyDown(" ", 32);
    xtermKeyPress(" ");
    feed("insertText", " ", "가 "); // prevValue가 정규화된 "가 "(U+0020)로 갱신됨
    writeInput.mockClear();

    keyDown("ㄴ", 78);
    feed("insertText", "ㄴ", "가 ㄴ"); // 다음 음절 시작

    expect(sentChunks()).toEqual(["ㄴ"]); // 공백이 다시 딸려 나가지 않는다
  });

  it('"가 나 다" 전체 실측 시퀀스: PTY 누적이 정확히 "가 나 다"(공백 U+0020 하나씩)', async () => {
    const { feed, keyDown, xtermKeyPress } = await mountMac();

    feed("insertText", "가", "가");
    keyDown(" ", 32);
    xtermKeyPress(" ");
    feed("insertText", " ", "가 ");

    keyDown("ㄴ", 78);
    feed("insertText", "ㄴ", "가 ㄴ");
    keyDown("ㅏ", 65);
    feed("insertReplacementText", "나", "가 나");

    keyDown(" ", 32);
    xtermKeyPress(" ");
    feed("insertText", " ", "가 나 ");

    keyDown("ㄷ", 68);
    feed("insertText", "ㄷ", "가 나 ㄷ");
    keyDown("ㅏ", 65);
    feed("insertReplacementText", "다", "가 나 다");

    expect(applyToLine(sentChunks())).toBe("가 나 다");
  });

  it("NBSP가 섞인 리싱크(blur 시점 값이 NBSP를 품음)에도 다음 조합에서 DEL이 새어 나가지 않는다", async () => {
    const { fake, feed, keyDown } = await mountMac();
    feed("insertText", "가", "가");
    keyDown(" ", 32);
    fake.emitInput(" "); // xterm 몫의 스페이스 발신(키프레스 대역)
    feed("insertText", " ", "가 ");

    // xterm의 blur 핸들러가 실제로는 textarea.value를 비우지만(browser/Clipboard.ts
    // 류 정리 경로), WKWebView 특유의 트레일링 NBSP가 비워지기 전에 리싱크가 먼저
    // 값을 읽는 경우를 흉내 — resyncSoon이 raw NBSP를 그대로 기준선으로 삼으면
    // 다음 조합의 diff에서 접두사가 어긋나 DEL이 샌다.
    fake.textarea.value = "가 ";
    fake.textarea.dispatchEvent(new Event("blur"));
    await Promise.resolve(); // resyncSoon의 queueMicrotask
    writeInput.mockClear();

    keyDown("ㄴ", 78);
    feed("insertText", "ㄴ", "가 ㄴ");

    expect(sentChunks()).toEqual(["ㄴ"]); // DEL 없음 — 진짜 프롬프트를 지우지 않는다
  });

  it("봇 운전 중이면 조합 갱신도 나가지 않는다", async () => {
    const terminalRegistry = await importRegistry();
    const { useAppStore } = await import("../../store/appStore");
    useAppStore.setState({ botMode: { a1: { agentId: "a1" } as never } });
    const host = document.createElement("div");
    terminalRegistry.attach("a1", host);
    const fake = terminalRegistry.get("a1")!.term as unknown as FakeTerminal;

    fake.emitTextInput("insertText", "ㅎ", "ㅎ");
    fake.emitTextInput("insertReplacementText", "하", "하");

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
      const opts = (e.term as unknown as FakeTerminal).options as {
        theme: unknown;
      };
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
