// src/renderer/terminal/TerminalRegistry.ts
//
// Xterm keep-alive registry. Exactly one
// `Terminal` instance per agentId, created once on first `ensure()`/`attach()`
// and never disposed until an explicit `destroy()` (session removal) —
// hiding a terminal is a `display:none` toggle done by the (later) UI layer,
// never a dispose. Scrollback/buffer therefore survives tab switches and
// React remounts for free, since the instance itself never goes away.
//
// Deviation from the original design skeleton: that skeleton's `window.api`
// is what an earlier task built as the `tauriApi` module, so this
// imports `tauriApi` directly instead of reading a `window.api` global.
//
// Terminal cell font is a regular monospace font (pixel fonts are banned
// here — the pixel aesthetic is the UI chrome's job, not the terminal
// screen's).
import { Terminal } from "@xterm/xterm";
import "@xterm/xterm/css/xterm.css";
import { FitAddon } from "@xterm/addon-fit";
import { tauriApi } from "../ipc/tauriApi";
import { XTERM_THEME } from "./theme";

interface Entry {
  term: Terminal;
  fit: FitAddon;
  disposeData: () => void; // onData unsubscribe
  container: HTMLDivElement; // the actual DOM node TerminalMount attaches
  opened: boolean; // has term.open() been called?
  bindComposition: () => void;
}

class TerminalRegistry {
  private entries = new Map<string, Entry>();

  /** First open for a session: creates the Terminal. Already-open agents get the existing entry back (keep-alive guarantee). */
  ensure(agentId: string): Entry {
    let e = this.entries.get(agentId);
    if (e) return e;

    const term = new Terminal({
      theme: XTERM_THEME,
      fontFamily: '"SF Mono", "Menlo", "Consolas", "Liberation Mono", monospace',
      fontSize: 13,
      lineHeight: 1.2,
      cursorBlink: true,
      scrollback: 5000,
      allowProposedApi: true,
    });
    const fit = new FitAddon();
    term.loadAddon(fit);

    // User input -> PTY, with a Hangul/IME double-input guard.
    //
    // Windows Korean IME finalizes a syllable per keystroke. In WebView2 the
    // keyup for the composing key resets xterm's internal `_keyDownSeen`
    // BEFORE the composed `input` event fires, so xterm's `_inputEvent` guard
    // `(!ev.composed || !_keyDownSeen)` passes and emits the syllable a SECOND
    // time on top of the compositionend path — "여러번" -> "여여러러번번".
    // We can't patch xterm, so we drop the duplicate: an identical chunk
    // re-emitted right after a `compositionend`. Gated on compositionend, so
    // English/paste/normal keys are never affected; only ever drops the second
    // of two identical emissions (a singly-emitted char is never eaten, ㅋㅋ ok).
    const IME_COMMIT_WINDOW_MS = 80; // "a compositionend happened just now"
    const IME_DUP_ADJ_MS = 20; // the echo lands right on top of the commit
    let compositionEndedAt = -Infinity;
    let lastData = "";
    let lastDataAt = -Infinity;

    const writeInput = (data: string) => {
      const now = performance.now();
      const isImeDuplicate =
        now - compositionEndedAt < IME_COMMIT_WINDOW_MS &&
        data === lastData &&
        now - lastDataAt < IME_DUP_ADJ_MS;
      if (isImeDuplicate) {
        lastData = ""; // consume: never drop a third identical emission
        return;
      }
      lastData = data;
      lastDataAt = now;
      tauriApi.writeInput(agentId, data);
    };
    term.onData(writeInput);

    // The hidden textarea only exists after term.open(); attach() calls this.
    let compositionBound = false;
    const onCompositionEnd = () => {
      compositionEndedAt = performance.now();
    };
    const bindComposition = () => {
      if (compositionBound) return;
      const ta = term.textarea;
      if (!ta) return;
      compositionBound = true;
      ta.addEventListener("compositionend", onCompositionEnd);
    };

    // Copy/paste key handling (fires only while THIS terminal is focused, so it is
    // naturally scoped per-agent in the multi-terminal keep-alive registry).
    term.attachCustomKeyEventHandler((event) => {
      if (event.isComposing || event.keyCode === 229) return true; // let xterm/IME own composition
      if (event.type !== "keydown") return true; // ignore keypress/keyup
      const mod = event.ctrlKey || event.metaKey;
      if (!mod || event.altKey) return true; // AltGr / plain keys pass through
      const key = event.key.toLowerCase();

      // Copy — Ctrl+Shift+C always; bare Ctrl/Cmd+C only with a selection
      // (no selection → fall through to SIGINT).
      if (key === "c") {
        if (event.shiftKey || term.hasSelection()) {
          const sel = term.getSelection();
          if (sel) void navigator.clipboard.writeText(sel).catch(() => {});
          event.preventDefault();
          return false; // swallow — do not send ^C
        }
        return true; // no selection → SIGINT
      }

      // Paste — Ctrl/Cmd+V or Ctrl+Shift+V. term.paste normalizes newlines and
      // respects bracketed-paste mode; it feeds onData → writeInput for us.
      if (key === "v") {
        event.preventDefault();
        void navigator.clipboard
          .readText()
          .then((t) => {
            if (t) term.paste(t);
          })
          .catch(() => {});
        return false; // swallow — never send raw ^V
      }

      return true;
    });

    // PTY output -> screen (bypasses the store, writes directly — a
    // high-frequency stream that would otherwise cause a render storm).
    const disposeData = tauriApi.onData(agentId, (data) => term.write(data));

    const container = document.createElement("div");
    container.className = "terminal-mount-inner";

    e = { term, fit, disposeData, container, opened: false, bindComposition };
    this.entries.set(agentId, e);
    return e;
  }

  get(agentId: string): Entry | undefined {
    return this.entries.get(agentId);
  }

  has(agentId: string): boolean {
    return this.entries.has(agentId);
  }

  /** Attaches the (keep-alive) container to a DOM host, opening the term the first time only. */
  attach(agentId: string, host: HTMLElement): void {
    const e = this.ensure(agentId);
    if (!e.container.isConnected) host.appendChild(e.container);
    if (!e.opened) {
      e.term.open(e.container);
      e.bindComposition();
      e.opened = true;
    }
  }

  /** On show: fit + resize IPC + focus, deferred one frame so layout (display:none -> block) has settled. */
  activate(agentId: string, onResize: (cols: number, rows: number) => void): void {
    const e = this.entries.get(agentId);
    if (!e || !e.opened) return;
    requestAnimationFrame(() => {
      try {
        e.fit.fit();
        onResize(e.term.cols, e.term.rows);
        e.term.focus();
      } catch {
        /* container measured 0 (e.g. hidden again before the frame ran) */
      }
    });
  }

  /** ResizeObserver callback for the currently-active terminal only. */
  refit(agentId: string, onResize: (cols: number, rows: number) => void): void {
    const e = this.entries.get(agentId);
    if (!e || !e.opened) return;
    e.fit.fit();
    onResize(e.term.cols, e.term.rows);
  }

  /** Real teardown — only on explicit session removal. */
  destroy(agentId: string): void {
    const e = this.entries.get(agentId);
    if (!e) return;
    e.disposeData();
    e.term.dispose();
    e.container.remove();
    this.entries.delete(agentId);
  }
}

export const terminalRegistry = new TerminalRegistry();
