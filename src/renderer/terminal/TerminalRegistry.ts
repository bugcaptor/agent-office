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
import type { ITheme } from "@xterm/xterm";
import "@xterm/xterm/css/xterm.css";
import { FitAddon } from "@xterm/addon-fit";
import { SerializeAddon } from "@xterm/addon-serialize";
import { tauriApi } from "../ipc/tauriApi";
import { useAppStore } from "../store/appStore";
import { resolveXtermTheme } from "./theme";
import { createImeBridge } from "./imeBridge";
import { createScrollbackGuard, type ScrollbackGuard } from "./scrollbackGuard";

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
  // §#49: 이 터미널이 실제로 렌더(소비)한 raw 스트림 바이트 누적치. xterm write
  // 콜백(렌더 완료 시점)에서 chunk.bytes를 더한다. 스냅샷 offset =
  // base(백엔드 attach 시점) + 이 값. 복원 스냅샷 청크는 bytes=0이라 제외된다.
  // 세션 attach 시점(엔트리 생성)부터 0에서 시작한다.
  renderedBytes: number;
  // PTY 스트림에서 `ESC[3J`(스크롤백 지우기)를 떼어 내는 필터. pi 기본 TUI가
  // resize마다 스크롤백을 날리는 것을 막는다(scrollbackGuard.ts 헤더 참조).
  scrollbackGuard: ScrollbackGuard;
}

/** TIOCSWINSZ가 같은 크기면 SIGWINCH를 안 쏘는 문제를 강제 재도색으로 우회하는 데 걸리는 대기(ms). */
const REDRAW_NUDGE_DELAY_MS = 50;

class TerminalRegistry {
  private entries = new Map<string, Entry>();
  // 입양(adopt_detached_sessions)된 세션 — 다음 activate()에서 1회
  // redraw nudge(§핵심 6)를 태우고 스스로 제거한다.
  private pendingNudge = new Set<string>();
  // 현재 유효한 xterm 팔레트. App의 테마 효과가 setTheme()으로 밀어 넣고,
  // 그 전에 만들어지는 터미널은 스토어의 현재 선택에서 지연 해석한다
  // (스토어 초기값이 이미 localStorage 복원값이라 부팅 직후에도 정확하다).
  private theme: ITheme | null = null;

  /** 새 Terminal에 먹일 팔레트(미설정이면 스토어에서 1회 해석해 기억). */
  private currentTheme(): ITheme {
    if (!this.theme) {
      const s = useAppStore.getState();
      this.theme = resolveXtermTheme(s.theme, s.xtermTheme);
    }
    return this.theme;
  }

  /**
   * 테마 전환 시 라이브 재도색. 보관된 팔레트를 갱신하고(이후 생성되는
   * 터미널이 쓴다) keep-alive 중인 전 인스턴스에 재대입한다 — xterm 5.x는
   * `options.theme` 재대입으로 즉시 다시 그린다. 숨어 있는(display:none)
   * 터미널도 같이 갱신되므로 탭을 옮겨도 예전 색이 남지 않는다.
   */
  setTheme(theme: ITheme): void {
    this.theme = theme;
    for (const e of this.entries.values()) e.term.options.theme = theme;
  }

  /** First open for a session: creates the Terminal. Already-open agents get the existing entry back (keep-alive guarantee). */
  ensure(agentId: string): Entry {
    let e = this.entries.get(agentId);
    if (e) return e;

    const term = new Terminal({
      theme: this.currentTheme(),
      fontFamily:
        '"SF Mono", "Menlo", "Consolas", "Liberation Mono", monospace',
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

    // 컨테이너를 먼저 만든다 — 조합 브리지가 여기 캡처 리스너를 건다.
    const container = document.createElement("div");
    container.className = "terminal-mount-inner";

    // 키 입력 → PTY 배선 일체(플랫폼별 IME 병 포함)는 imeBridge가 맡는다.
    // 여기 남는 것은 "봇이 몰고 있으면 사람 입력을 막는다"는 앱 규칙뿐이다.
    const ime = createImeBridge({
      term,
      container,
      inputBlocked: () => useAppStore.getState().isBotDriven(agentId),
      send: (data) => tauriApi.writeInput(agentId, data),
    });

    // Copy/paste key handling (fires only while THIS terminal is focused, so it is
    // naturally scoped per-agent in the multi-terminal keep-alive registry).
    term.attachCustomKeyEventHandler((event) => {
      const composition = ime.onKeyEvent(event);
      if (composition !== null) return composition;
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
            // term.paste는 `textarea.value = ''`까지 한다(browser/Clipboard.ts).
            // keydown 시점의 리싱크는 이 시점보다 앞서므로 여기서 한 번 더 맞춘다 —
            // 안 그러면 다음 조합에서 사라진 길이만큼 DEL이 새어 나간다.
            ime.resync();
          })
          .catch(() => {});
        return false; // swallow — never send raw ^V
      }

      return true;
    });

    // PTY output -> screen (bypasses the store, writes directly — a
    // high-frequency stream that would otherwise cause a render storm).
    // §#49: raw 스트림 바이트를 write 콜백(실제 렌더 완료 시점)에서 누적해
    // 스냅샷 offset 회계에 쓴다. flushAndSerializeAll이 write("", cb)로 큐를
    // 비운 직후 읽으면 렌더 완료분이 정확히 반영된다. bytes=0 복원 청크는
    // 자연히 제외된다.
    // `ESC[3J`(스크롤백 지우기)만 떼어 낸다 — pi 기본 TUI가 resize마다 앱
    // 터미널 히스토리를 통째로 지우는 것을 막는 유일한 앱 측 수단이다
    // (scrollbackGuard.ts 헤더에 근거와 대가). 붙들린 경계 조각은 렌더되는
    // 게 없으므로 renderedBytes 회계는 그대로 chunk.bytes를 더한다.
    const scrollbackGuard = createScrollbackGuard();
    const disposeData = tauriApi.onData(agentId, (data, bytes) => {
      term.write(scrollbackGuard.filter(data), () => {
        const cur = this.entries.get(agentId);
        if (cur) cur.renderedBytes += bytes;
      });
    });

    e = {
      term,
      fit,
      serialize,
      disposeData,
      container,
      opened: false,
      bindComposition: ime.bindComposition,
      renderedBytes: 0,
      scrollbackGuard,
    };
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
        await new Promise<void>((resolve) =>
          e.term.write(e.scrollbackGuard.flush(), () => resolve()),
        );
        out[agentId] = e.serialize.serialize();
      } catch {
        /* 이 터미널만 스킵 -- 나머지 스냅샷은 정상 전달 */
      }
    }
    return out;
  }

  /**
   * 웹 원격: 한 터미널만 flush+직렬화한다. 브라우저가 처음 붙을 때
   * 호스트가 `web-remote-snapshot-request`로 요청하는 화면 이미지 — 없으면 링버퍼
   * 리플레이로 폴백하므로, 실패는 조용히 undefined를 돌려주면 된다.
   * `flushAndSerializeAll`과 같은 이유로 직렬화 전에 write 큐를 비운다.
   */
  async flushAndSerialize(agentId: string): Promise<string | undefined> {
    const e = this.entries.get(agentId);
    if (!e) return undefined;
    try {
      await new Promise<void>((resolve) =>
        e.term.write(e.scrollbackGuard.flush(), () => resolve()),
      );
      return e.serialize.serialize();
    } catch {
      return undefined;
    }
  }

  /**
   * §#49: agentId -> 렌더러가 실제 렌더(소비)한 raw 스트림 바이트 누적치.
   * `flushAndSerializeAll()`과 같은 시점에 읽어(그게 write("", cb)로 큐를 비운
   * 직후라 렌더 완료분이 정확히 반영됨) 스냅샷 업로드/핸드오프에 offset으로
   * 동봉한다. 백엔드가 base(attach 오프셋)에 이 값을 더해 최종 offset을 만든다.
   */
  getRenderedBytes(): Record<string, number> {
    const out: Record<string, number> = {};
    for (const [agentId, e] of this.entries) out[agentId] = e.renderedBytes;
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
  activate(
    agentId: string,
    onResize: (cols: number, rows: number) => void,
  ): void {
    const e = this.entries.get(agentId);
    if (!e || !e.opened) return;
    requestAnimationFrame(() => {
      try {
        // 백엔드가 알고 있던 크기를 onResize(=setSessionSize)가 덮어쓰기 전에 읽어 둔다.
        const known = useAppStore.getState().sessions[agentId];
        e.fit.fit();
        // 숨어 있는 동안 오염된 스크롤 영역 높이를 여기서 되돌린다(syncViewport 주석).
        this.syncViewport(e.term);
        onResize(e.term.cols, e.term.rows);
        e.term.focus();
        if (this.pendingNudge.delete(agentId)) {
          // 방금 보낸 resize가 이미 크기를 바꿨으면 SIGWINCH가 이미 갔다 —
          // nudge는 불필요하고, 오히려 해롭다: nudge는 rows-1 → rows로 resize를
          // 두 번 더 쏘는데, pi 기본 TUI(regular)는 resize마다 `ESC[3J`로
          // 스크롤백을 지워 버린다(pi v0.84.2 PTY 실측). 크기가 그대로일 때만
          // (TIOCSWINSZ가 SIGWINCH를 안 쏘는 경우) nudge한다.
          const alreadyResized =
            known !== undefined &&
            (known.cols !== e.term.cols || known.rows !== e.term.rows);
          if (!alreadyResized) this.redrawNudge(agentId, e, onResize);
        }
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
    onResize: (cols: number, rows: number) => void,
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
    // 컨테이너가 한 행보다 작게 바뀌면 fit()이 resize를 안 내 뷰포트 지오메트리가
    // 그대로 남는다 — activate()와 같은 이유로 여기서도 재동기화한다.
    this.syncViewport(e.term);
    onResize(e.term.cols, e.term.rows);
  }

  /**
   * xterm 뷰포트 지오메트리 강제 재동기화.
   *
   * 오버레이가 닫혀 있거나(TerminalOverlay의 display:none) 다른 탭이 활성인 동안에도
   * PTY 출력은 계속 이 xterm에 write된다. 그때 xterm의 Viewport는 출력마다
   * `_innerRefresh()`를 돌며 스크롤 영역 높이를
   * `rowH * lines.length + (viewportElement.offsetHeight - canvas.height)`로 다시 쓰는데,
   * 숨겨져 있으면 `offsetHeight === 0`이라 **한 화면 높이만큼 짧은 값**으로 굳는다
   * (xterm.js #494). 다시 보일 때 fit()이 같은 cols/rows를 내면 FitAddon도
   * `Terminal.resize()`도 조기 반환해 `_afterResize()`의 syncScrollArea(true)가 안 불리고,
   * 스크롤백이 가득 찬 뒤에는 `syncScrollArea()`의 네 검사(버퍼 길이/뷰포트 높이/ydisp/
   * 셀 높이)가 전부 "변화 없음"이라 스스로 재측정할 기회도 없다. 결과: DOM 스크롤 범위가
   * ydisp=ybase에 필요한 값보다 한 화면 모자란 채 고정 → 휠을 조금만 올려도 한 화면씩
   * 튀고(_handleScroll이 scrollTop으로 절대 행을 역산한다), 아무리 내려도 바닥에 못 닿는다.
   *
   * syncScrollArea(true)를 강제로 부르면 2번 검사
   * (`_lastRecordedViewportHeight !== css.canvas.height`, 0 !== 실제 높이)가 걸려 즉시
   * 재계산된다. PTY로 나가는 resize가 없으므로 `ESC[3J`로 스크롤백을 지우는 TUI에도 무해하다.
   * 사적 API라 없으면 조용히 넘어간다.
   */
  private syncViewport(term: Terminal): void {
    try {
      (
        term as unknown as {
          _core?: { viewport?: { syncScrollArea?: (immediate?: boolean) => void } };
        }
      )._core?.viewport?.syncScrollArea?.(true);
    } catch {
      /* 사적 API 형태가 바뀌었거나 뷰포트가 아직 없다 — 스크롤 범위만 늦게 맞을 뿐 무해 */
    }
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
