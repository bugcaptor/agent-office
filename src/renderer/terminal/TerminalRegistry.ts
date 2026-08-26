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
import { IS_MAC } from "../shared/platform";

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

    // User input -> PTY.
    //
    // ── (1) Windows 이중 입력 가드 ────────────────────────────────────────
    // WebView2에서는 조합 키의 keyup이 xterm 내부 `_keyDownSeen`을 composed
    // `input` 이벤트보다 먼저 리셋해 `_inputEvent` 가드
    // `(!ev.composed || !_keyDownSeen)`가 뚫리고, compositionend 경로 위에 음절이
    // 한 번 더 나간다 — "여러번" -> "여여러러번번". xterm을 고칠 수 없으니
    // compositionend 직후에 똑같은 청크가 연달아 나오면 둘째를 버린다.
    // macOS는 애초에 composition 이벤트가 안 오므로(아래 (2)) 이 가드가 걸릴
    // 일이 없고, 걸린다면 진짜 입력을 먹는 쪽이라 아예 꺼 둔다.
    let emitSeq = 0; // PTY로 나간 청크 수 — (3)의 안전망이 "xterm이 보냈나"를 본다
    const IME_COMMIT_WINDOW_MS = 80; // "a compositionend happened just now"
    const IME_DUP_ADJ_MS = 20; // the echo lands right on top of the commit
    let compositionEndedAt = -Infinity;
    let lastData = "";
    let lastDataAt = -Infinity;

    const writeInput = (data: string) => {
      // 봇 운전 중인 탭은 로컬 키 입력을 차단한다(이슈 #57). 봇 자신의 주입은
      // 백엔드 write_input을 직접 거치므로 이 게이트를 통과하지 않는다 — 여기서
      // 막히는 건 사람이 xterm에 타이핑/붙여넣기 하는 경로뿐이다.
      if (useAppStore.getState().isBotDriven(agentId)) {
        return;
      }
      const now = performance.now();
      const isImeDuplicate =
        !IS_MAC &&
        now - compositionEndedAt < IME_COMMIT_WINDOW_MS &&
        data === lastData &&
        now - lastDataAt < IME_DUP_ADJ_MS;
      if (isImeDuplicate) {
        lastData = ""; // consume: never drop a third identical emission
        return;
      }
      lastData = data;
      lastDataAt = now;
      emitSeq++;
      tauriApi.writeInput(agentId, data);
    };
    term.onData(writeInput);

    // ── (2) WebKit 한글 조합 미러링 ───────────────────────────────────────
    // macOS WebKit(WKWebView/Safari)은 한글 IME에 **composition 이벤트를 아예
    // 쏘지 않는다**(Safari 실측 트레이스: compositionstart/update/end 0건).
    // 대신 조합을 이렇게 흘린다 — "한글" 입력 시:
    //   input insertText            "ㅎ"  ← 새 음절 시작
    //   input insertReplacementText "하"  ← 조합 갱신(줄 끝 글자를 갈아끼움)
    //   input insertReplacementText "한"
    //   input insertText            "ㄱ"  ← 다음 음절 시작
    // xterm 5.5의 `_inputEvent`는 `insertText`만 처리하므로 각 음절의 **첫
    // 자모만** PTY로 나가고 나머지 조합은 통째로 증발한다("한글입력" ->
    // "ㅎㄱㅇㄹ"). 앞 음절 받침이 뒤로 넘어갈 때(`한ㄱ`+ㅏ -> `한`/`가`)만
    // 완성 음절이 insertText로 도착해 살아남는다 — "가나다라"만 멀쩡해 보이는
    // 이유다.
    //
    // 그래서 조합 갱신을 우리가 직접 미러링한다: PTY 줄 끝에 우리가 보내 둔
    // 꼬리(imeTail)를 기억하고, 갱신이 오면 공통 접두사만 남기고 DEL로 지운 뒤
    // 새 꼴을 이어 쓴다. TUI 입장에선 사람이 백스페이스로 고쳐 쓰는 것과 같아
    // 조합이 화면에서 그대로 굴러간다. 한 번의 write로 묶어 보내 중간 상태가
    // 깜빡이지 않게 한다.
    //
    // composition 이벤트가 정상으로 오는 플랫폼(Chrome/WebView2)에서는
    // `insertReplacementText`가 오지 않으므로 이 경로는 아예 돌지 않는다.
    // 그래도 조합 중이면 xterm에 맡기도록 명시적으로 비켜 준다.
    // ── (3) xterm `_inputEvent` 가드의 stale 플래그 청소 ─────────────────
    // 위 (2)를 고쳐도 음절이 **통째로** 빠지는 일이 남는다. Safari 실측에서
    // insertText 28건 중 2건이 xterm에서 조용히 버려졌다. `_inputEvent`의 가드가
    //   `ev.data && inputType === "insertText" && (!ev.composed || !_keyDownSeen)`
    // 를 통과한 뒤 다시 `if (_keyPressHandled) return false`로 막는데, WebKit은
    // 조합의 `input`을 keydown보다 **먼저** 쏘므로 두 플래그가 늘 *앞 키*의
    // 상태로 남아 있다:
    //   · 앞 키를 놓기 전에 다음 키를 누르면(굴려치기) `_keyDownSeen`이 true인
    //     채로 도착 — trace: keydown ㅌ → input "트" → keyup ㅌ, "트" 증발.
    //   · 직전에 ASCII/문장부호를 쳤으면 `_keyPressHandled`가 true로 남는다 —
    //     trace: "," 전송 → input "ㄹ" → keyup ",", "ㄹ" 증발.
    // 그래서 input이 xterm에 닿기 전(컨테이너 캡처 단계)에 두 플래그를 내려
    // 준다. 방금 keypress가 있었던 입력, 즉 ASCII 본인의 input은 건드리지 않아야
    // xterm이 keypress로 이미 보낸 글자가 두 번 나가지 않는다.
    //
    // xterm의 내부 필드를 건드리는 유일한 자리다. 공개 API로는 저 가드를 우회할
    // 수 없고(이벤트를 가로채면 ASCII 경로까지 우리가 떠안아야 한다), 필드가
    // 사라지면 조용히 손대지 않는 쪽으로 물러난다 — 그 경우 증상만 예전으로
    // 돌아가고 다른 것이 깨지지는 않는다. composition 이벤트가 정상으로 오는
    // 플랫폼(Chrome/WebView2)에서는 아예 손대지 않는다.
    let keyPressPending = false;
    let sawComposition = false;
    let emitSeqAtInputStart = 0; // 이번 input 이벤트 직전의 emitSeq
    let xtermOwnsInput = false; // 이번 input은 xterm이 이미 책임졌다(ASCII·조합 경로)
    const clearStaleInputGuards = () => {
      emitSeqAtInputStart = emitSeq;
      xtermOwnsInput = false;
      if (sawComposition) {
        xtermOwnsInput = true;
        return;
      }
      if (keyPressPending) {
        keyPressPending = false; // ASCII 본인의 input — xterm이 keypress로 이미 보냈다
        xtermOwnsInput = true;
        return;
      }
      // 플래그는 공개 `Terminal` 래퍼가 아니라 그 안의 `_core`(browser/Terminal)에
      // 있다. 래퍼에 그냥 쓰면 조용한 no-op이 된다 — 실제로 한 번 당했다.
      const core = (term as unknown as { _core?: unknown })._core ?? term;
      const priv = core as { _keyDownSeen?: boolean; _keyPressHandled?: boolean };
      if (typeof priv._keyDownSeen === "boolean") priv._keyDownSeen = false;
      if (typeof priv._keyPressHandled === "boolean") priv._keyPressHandled = false;
    };

    const DEL = "\x7f";
    let imeTail = ""; // PTY 줄 끝에 보내 놓은, 아직 조합 중인 꼬리
    let imeComposing = false;

    const onTextareaInput = (ev: InputEvent) => {
      if (imeComposing) return; // composition 경로가 살아 있는 플랫폼 — xterm 몫
      if (ev.inputType === "insertText") {
        // 새 음절의 시작. 정상이면 xterm의 `_inputEvent`가 방금(이 이벤트 안에서)
        // PTY로 보냈으니 우리는 "줄 끝이 지금 이것"이라고 기억만 한다.
        //
        // 안전망: (3)이 무력해지면(예: 다음 xterm에서 필드 이름이 바뀌면) xterm이
        // 이 음절을 통째로 버린다. 그때는 우리가 직접 보낸다 — xterm이 이 이벤트
        // 동안 아무 것도 안 보냈고, 이 입력이 xterm 소유가 아닐 때만.
        const data = ev.data ?? "";
        if (!xtermOwnsInput && data && emitSeq === emitSeqAtInputStart) writeInput(data);
        imeTail = data;
        return;
      }
      if (ev.inputType !== "insertReplacementText") {
        imeTail = ""; // 붙여넣기·삭제 등 — 꼬리 추적을 포기한다
        return;
      }
      const next = ev.data ?? "";
      const before = [...imeTail];
      const after = [...next];
      let same = 0;
      while (same < before.length && same < after.length && before[same] === after[same]) same++;
      const patch = DEL.repeat(before.length - same) + after.slice(same).join("");
      if (patch) writeInput(patch);
      imeTail = next;
    };

    // The hidden textarea only exists after term.open(); attach() calls this.
    let compositionBound = false;
    const onCompositionStart = () => {
      imeComposing = true;
      sawComposition = true; // 이 플랫폼은 조합을 제대로 알려 준다 — (3)에서 손 뗀다
      imeTail = "";
    };
    const onCompositionEnd = () => {
      imeComposing = false;
      compositionEndedAt = performance.now();
    };
    const onTextareaBlur = () => {
      imeTail = ""; // 포커스가 떠나면 줄 끝을 더 이상 알 수 없다
    };
    const bindComposition = () => {
      if (compositionBound) return;
      const ta = term.textarea;
      if (!ta) return;
      compositionBound = true;
      ta.addEventListener("compositionstart", onCompositionStart);
      ta.addEventListener("compositionend", onCompositionEnd);
      // xterm보다 나중에 등록된다 — insertText 경로에서 xterm이 먼저 보내고
      // 우리가 꼬리를 기억하는 순서가 되어야 맞다.
      ta.addEventListener("input", onTextareaInput as EventListener);
      ta.addEventListener("blur", onTextareaBlur);
      // 반대로 (3)은 xterm보다 **먼저** 돌아야 한다. 같은 노드에 건 리스너는
      // 등록 순서를 따르므로(xterm이 먼저 등록했다) 조상인 컨테이너의 캡처
      // 단계에 건다 — 캡처는 대상 노드의 리스너보다 항상 앞선다.
      container.addEventListener("input", clearStaleInputGuards, true);
    };

    // Copy/paste key handling (fires only while THIS terminal is focused, so it is
    // naturally scoped per-agent in the multi-terminal keep-alive registry).
    term.attachCustomKeyEventHandler((event) => {
      // (3)의 판별: ASCII는 keydown -> keypress -> input 순서라 input 시점에
      // keypress가 방금 있었다. 조합 키는 input이 keydown보다 먼저 와서 없다.
      if (event.type === "keydown") keyPressPending = false;
      else if (event.type === "keypress") keyPressPending = true;

      // 조합 키(keyCode 229)가 아닌 keydown은 xterm이 제어 시퀀스로 바꿔 보내
      // 줄 끝을 우리가 모르게 바꾼다(Enter/Backspace/화살표…). 꼬리 추적을 접는다.
      // 조합 키의 input은 keydown보다 **먼저** 오므로(WebKit 실측) 이 리셋이
      // 같은 키의 조합 갱신을 지우는 일은 없다.
      if (event.type === "keydown" && event.keyCode !== 229) imeTail = "";
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
    // §#49: raw 스트림 바이트를 write 콜백(실제 렌더 완료 시점)에서 누적해
    // 스냅샷 offset 회계에 쓴다. flushAndSerializeAll이 write("", cb)로 큐를
    // 비운 직후 읽으면 렌더 완료분이 정확히 반영된다. bytes=0 복원 청크는
    // 자연히 제외된다.
    const disposeData = tauriApi.onData(agentId, (data, bytes) => {
      term.write(data, () => {
        const cur = this.entries.get(agentId);
        if (cur) cur.renderedBytes += bytes;
      });
    });

    const container = document.createElement("div");
    container.className = "terminal-mount-inner";

    e = {
      term,
      fit,
      serialize,
      disposeData,
      container,
      opened: false,
      bindComposition,
      renderedBytes: 0,
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
        await new Promise<void>((resolve) => e.term.write("", () => resolve()));
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
      await new Promise<void>((resolve) => e.term.write("", () => resolve()));
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
  activate(agentId: string, onResize: (cols: number, rows: number) => void): void {
    const e = this.entries.get(agentId);
    if (!e || !e.opened) return;
    requestAnimationFrame(() => {
      try {
        // 백엔드가 알고 있던 크기를 onResize(=setSessionSize)가 덮어쓰기 전에 읽어 둔다.
        const known = useAppStore.getState().sessions[agentId];
        e.fit.fit();
        onResize(e.term.cols, e.term.rows);
        e.term.focus();
        if (this.pendingNudge.delete(agentId)) {
          // 방금 보낸 resize가 이미 크기를 바꿨으면 SIGWINCH가 이미 갔다 —
          // nudge는 불필요하고, 오히려 해롭다: nudge는 rows-1 → rows로 resize를
          // 두 번 더 쏘는데, pi 기본 TUI(regular)는 resize마다 `ESC[3J`로
          // 스크롤백을 지워 버린다(pi v0.84.2 PTY 실측). 크기가 그대로일 때만
          // (TIOCSWINSZ가 SIGWINCH를 안 쏘는 경우) nudge한다.
          const alreadyResized =
            known !== undefined && (known.cols !== e.term.cols || known.rows !== e.term.rows);
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
