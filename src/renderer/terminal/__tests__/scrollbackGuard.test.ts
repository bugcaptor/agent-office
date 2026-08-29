import { describe, expect, it } from "vitest";
import { createScrollbackGuard, ERASE_SCROLLBACK } from "../scrollbackGuard";

describe("scrollbackGuard", () => {
  it("passes ordinary output through untouched", () => {
    const g = createScrollbackGuard();
    expect(g.filter("hello\r\nworld")).toBe("hello\r\nworld");
    expect(g.pending).toBe("");
  });

  it("strips ESC[3J but keeps ESC[2J and ESC[H (pi fullRender prologue)", () => {
    const g = createScrollbackGuard();
    // pi-tui가 resize마다 내보내는 그대로: 화면 지우기 + 홈 + 스크롤백 지우기.
    expect(g.filter("\x1b[?2026h\x1b[2J\x1b[H\x1b[3Jline")).toBe(
      "\x1b[?2026h\x1b[2J\x1b[Hline",
    );
  });

  it("strips every occurrence in one chunk", () => {
    const g = createScrollbackGuard();
    expect(g.filter(`a${ERASE_SCROLLBACK}b${ERASE_SCROLLBACK}c`)).toBe("abc");
  });

  it("strips a sequence split across chunk boundaries", () => {
    const g = createScrollbackGuard();
    expect(g.filter("keep\x1b[3")).toBe("keep");
    expect(g.pending).toBe("\x1b[3");
    expect(g.filter("Jmore")).toBe("more");
    expect(g.pending).toBe("");
  });

  it("releases a held partial that turns out not to be ESC[3J", () => {
    const g = createScrollbackGuard();
    expect(g.filter("x\x1b")).toBe("x");
    expect(g.filter("[2Jy")).toBe("\x1b[2Jy");
  });

  it("holds only a genuine prefix, never a longer tail", () => {
    const g = createScrollbackGuard();
    expect(g.filter("done\x1b[")).toBe("done");
    expect(g.pending).toBe("\x1b[");
    expect(g.filter("K")).toBe("\x1b[K");
  });

  it("flush() releases the held fragment exactly once", () => {
    const g = createScrollbackGuard();
    g.filter("a\x1b[3");
    expect(g.flush()).toBe("\x1b[3");
    expect(g.flush()).toBe("");
    // flush 뒤에 온 J는 더 이상 시퀀스를 이루지 않는다 — 그대로 흘려보낸다.
    expect(g.filter("J")).toBe("J");
  });
});
