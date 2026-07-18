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
import { SerializeAddon } from "@xterm/addon-serialize";
import { tauriApi } from "../ipc/tauriApi";
import { XTERM_THEME } from "./theme";

interface Entry {
  term: Terminal;
  fit: FitAddon;
  // 세션 핸드오프(docs/session-handoff-design.md 빈틈 수정): 종료 시점의
  // 화면(스크롤백 포함)을 직렬화해 데몬에 실어 보내는 데 쓴다 — 데몬은
  // 핸드오프 *이후* 출력만 보관하므로, 그 이전 화면은 이 스냅샷이 아니면
  // 재입양 후 사라진다.
  serialize: SerializeAddon;
  disposeData: () => void; // onData unsubscribe
  container: HTMLDivElement; // the actual DOM node TerminalMount attaches
  opened: boolean; // has term.open() been called?
  bindComposition: () => void;
}

/** TIOCSWINSZ가 같은 크기면 SIGWINCH를 안 쏘는 문제를 강제 재도색으로 우회하는 데 걸리는 대기(ms). */
const REDRAW_NUDGE_DELAY_MS = 50;

class TerminalRegistry {
  private entries = new Map<string, Entry>();
  // 입양(adopt_detached_sessions)된 세션 — 다음 activate()에서 1회
  // redraw nudge(§핵심 6)를 태우고 스스로 제거한다.
  private pendingNudge = new Set<string>();

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
    const serialize = new SerializeAddon();
    term.loadAddon(serialize);

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

    e = { term, fit, serialize, disposeData, container, opened: false, bindComposition };
    this.entries.set(agentId, e);
    return e;
  }

  get(agentId: string): Entry | undefined {
    return this.entries.get(agentId);
  }

  /**
   * 세션 핸드오프: 종료 확인 모달에서 "터미널 유지하고 종료"를 고를 때
   * 호출 — 살아있는 모든 터미널의 화면(스크롤백 포함)을 직렬화해 agentId
   * 키로 반환한다. 데몬은 핸드오프 *이후* 출력만 링버퍼에 담으므로, 이
   * 스냅샷이 없으면 종료 직전 화면(예: ls 결과)이 재입양 후 사라진다.
   * 한 터미널의 직렬화 실패가 나머지를 막지 않도록 개별 try/catch로 스킵.
   */
  serializeAll(): Record<string, string> {
    const out: Record<string, string> = {};
    for (const [agentId, e] of this.entries) {
      try {
        out[agentId] = e.serialize.serialize();
      } catch {
        /* 이 터미널만 스킵 -- 나머지 스냅샷은 정상 전달 */
      }
    }
    return out;
  }

  /**
   * 세션 브로커 v2(§P1): 직렬화 *전에* 각 터미널의 xterm write 큐를 flush해,
   * 이미 도착했지만 아직 파싱/렌더 안 된 바이트까지 스냅샷에 반영한다 —
   * `term.write("", cb)`의 콜백은 큐가 비워진 뒤에 불린다. 이렇게 하면 스냅샷이
   * "앱이 실제로 여기까지 받았다"는 지점과 최대한 정합하고, 브로커가 그 오프셋
   * 이후만 리플레이해도 유실이 없다. quit/주기 업로더가 사용(둘 다 async 경로).
   * 한 터미널의 실패가 나머지를 막지 않도록 개별 try/catch로 스킵.
   */
  async flushAndSerializeAll(): Promise<Record<string, string>> {
    const out: Record<string, string> = {};
    for (const [agentId, e] of this.entries) {
      try {
        await new Promise<void>((resolve) => e.term.write("", () => resolve()));
        out[agentId] = e.serialize.serialize();
      } catch {
        /* 이 터미널만 스킵 -- 나머지 스냅샷은 정상 전달 */
      }
    }
    return out;
  }

  has(agentId: string): boolean {
    return this.entries.has(agentId);
  }

  /**
   * 이슈 #42: 현재 버퍼(스크롤백 포함)를 plain text로 추출한다. 아직 만들어지지
   * 않은(ensure 전) 터미널은 undefined. 각 줄은 translateToString(true)로 우측
   * 공백을 떼어 뽑고, 소프트랩(isWrapped)된 줄은 앞 줄에 개행 없이 이어붙여
   * xterm의 자동 줄바꿈이 하드 개행으로 굳지 않게 한다. 끝쪽 빈 줄은 트림하고
   * 마지막에 개행 하나를 붙여 파일이 개행으로 끝나게 한다.
   */
  getPlainText(agentId: string): string | undefined {
    const e = this.entries.get(agentId);
    if (!e) return undefined;
    const buf = e.term.buffer.active;
    const lines: string[] = [];
    for (let i = 0; i < buf.length; i++) {
      const line = buf.getLine(i);
      const text = line?.translateToString(true) ?? "";
      if (line?.isWrapped && lines.length > 0) {
        lines[lines.length - 1] += text; // 소프트랩: 앞 줄에 이어붙임
      } else {
        lines.push(text);
      }
    }
    return lines.join("\n").replace(/\n+$/, "") + "\n";
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
        if (this.pendingNudge.delete(agentId)) this.redrawNudge(agentId, e, onResize);
      } catch {
        /* container measured 0 (e.g. hidden again before the frame ran) */
      }
    });
  }

  /**
   * 입양된(adopt_detached_sessions) 세션들을 표시 — 각각 다음 activate()에서
   * 1회 redraw nudge가 발화한다. 부팅 시(bootstrap.ts) 1회 호출.
   */
  markAdopted(agentIds: Iterable<string>): void {
    for (const id of agentIds) this.pendingNudge.add(id);
  }

  /**
   * TIOCSWINSZ는 크기가 그대로면 SIGWINCH를 쏘지 않는다 — 데몬에서 되찾은
   * PTY 안의 TUI(vim/htop/claude 등)가 재시작 전 마지막 화면을 그대로 들고
   * 있어 재도색이 안 된다. fit()으로 확정한 실제 rows보다 1 작은 값으로
   * resize를 한 번 보내 강제로 다르게 만든 뒤, 살짝 기다렸다 다시 fit() +
   * onResize()로 원래 크기로 되돌린다 — SIGWINCH 2회로 TUI를 재도색시킨다.
   * 일반 셸에는 무해(그냥 프롬프트가 두 번 다시 그려질 뿐).
   */
  private redrawNudge(
    agentId: string,
    e: Entry,
    onResize: (cols: number, rows: number) => void
  ): void {
    if (e.term.rows <= 1) return; // too small to shrink by one row — skip
    tauriApi.resize(agentId, e.term.cols, e.term.rows - 1);
    setTimeout(() => {
      try {
        e.fit.fit();
        onResize(e.term.cols, e.term.rows);
      } catch {
        /* container gone by the time the nudge fired — harmless */
      }
    }, REDRAW_NUDGE_DELAY_MS);
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
    this.pendingNudge.delete(agentId);
  }
}

export const terminalRegistry = new TerminalRegistry();
