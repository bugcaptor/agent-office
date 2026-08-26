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
      tauriApi.writeInput(agentId, data);
    };

    // ── (1.5) 키별 원장(ledger): xterm과 미러의 중복 발신 상쇄 ────────────
    // macOS WebKit에서는 "한 번의 키가 무엇을 보냈는가"를 **순서로 판별할 수
    // 없다**. IME가 매개하는 `input`은 그 키의 keydown/keypress보다 먼저 오기도
    // 하고 나중에 오기도 한다(이번 버그의 핵심 성질).
    //
    // 스페이스가 두 칸 들어가던 것이 그 증거다: xterm `evaluateKeyboardEvent`의
    // default 분기는 `ev.keyCode >= 48`이라 스페이스(32)는 `result.key`가 비어
    // `_keyDown`이 그냥 통과시키고, 결국 `_keyPress`가 charCode 32로 " "를 보낸다.
    // 그런데 조합을 커밋하는 스페이스의 `input`은 그 keypress보다 **먼저** 도착해서
    //   keydown(32) → input(" " 미러가 보냄) → keypress(32, xterm이 또 보냄)
    // 이 된다. "직전에 keypress가 있었나" 같은 순서 전제로는 이 방향을 못 막는다.
    //
    // 그래서 순서를 따지지 않는다. keydown마다 원장 두 칸을 비우고, 한쪽이 보낸
    // 청크를 반대쪽이 그대로 다시 보내려 하면 **정확히 같은 문자열 1회**만
    // 상쇄한다. 어느 쪽이 먼저 오든 결과가 같다. 같은 글자를 연달아 쳐도
    // (`ㅋㅋ`, 스페이스 연타) keydown마다 원장이 비므로 진짜 입력은 안 먹힌다.
    let xtermSent = ""; // 이번 키에서 xterm이 보낸 청크(미러가 상쇄할 몫)
    let mirrorSent = ""; // 이번 키에서 미러가 보낸 청크(xterm이 상쇄할 몫)

    // xterm 발신구. 미러는 `writeInput`을 직접 부르므로 이 경로로 오지 않는다 —
    // 여기 들어오는 건 keypress/제어키/`term.paste()` 등 순수 xterm의 결론뿐이다.
    term.onData((data) => {
      if (data && data === mirrorSent) {
        mirrorSent = ""; // 미러가 이미 보냈다 — 소비하고 버린다
        return;
      }
      writeInput(data);
      xtermSent = data;
    });

    // ── (2) macOS WKWebView 한글 조합: 우리가 **유일한** writer가 된다 ────
    // Tauri WKWebView는 한글 IME에 **composition 이벤트를 쏘지 않는다**.
    // 앱 안에서 뜬 실측 트레이스(2026-08-26, "가 나 다"):
    //   mirror prev=""      next="ㄱ"    keydown key="ㄱ" keyCode=229 isComposing=false
    //   mirror prev="ㄱ"    next="가"    keydown key="ㅏ" keyCode=229 isComposing=false
    //   compositionstart/update/end 0건, isComposing은 끝까지 false
    // 즉 조합은 숨은 textarea의 `value`가 바뀌며 `input`으로만 흘러나오고,
    // 커밋 시점에도 값이 그대로라 patch가 ""인 input이 한 번 더 올 뿐이다.
    //
    // ⚠ **Safari로 재면 다른 결과가 나온다.** 같은 WebKit인데도 Safari는
    // compositionstart/update/end를 정상으로 쏘고 inputType도 insertCompositionText다
    // (같은 날 실측). 이 영역은 반드시 **앱 안에서** 재야 한다 — Safari 트레이스를
    // 근거로 세운 예전 판(insertText/insertReplacementText 분기)이 계속 어긋났던 이유다.
    //
    // 이 한 줄에 쓰겠다고 나서는 주체가 셋이라 교통정리가 필요하다:
    //   (a) xterm `_inputEvent` — textarea의 capture 리스너. `insertText`만 동기로
    //       그대로 보낸다(조합 갱신은 통째로 증발).
    //   (b) xterm `CompositionHelper._handleAnyTextareaChanges` — `Terminal._keyDown`은
    //       "조합 중이 아닌데 keyCode 229가 왔으면" 이걸 부른다. 여기선
    //       `_isComposing`이 **항상** false라 한글 키마다 매번 돈다. 이 함수는
    //       `oldValue = textarea.value`를 찍고 `setTimeout(0)` 뒤 `newValue`와 비교해
    //       **비동기로** 제 나름의 결론을 쏜다(길어지면 `replace` diff, 짧아지면 DEL,
    //       길이가 같은데 다르면 newValue 통째로). 우리 미러와 경쟁하는 비동기 writer라
    //       반드시 막아야 한다.
    //   (c) 우리 미러(아래).
    //
    // 그래서 macOS에서는 (a)(b)를 둘 다 걷어내고 (c) 하나만 남긴다:
    //   · (b)는 커스텀 키 핸들러가 keyCode 229 keydown에 **false**를 돌려주면
    //     막힌다 — `_keyDown`은 커스텀 핸들러가 false면 `_compositionHelper
    //     .keydown()`을 부르기 전에 빠져나간다(xterm 5.5 browser/Terminal.ts).
    //     229 keydown에서 xterm이 하는 일은 저 호출 하나뿐이라 잃는 것도 없다.
    //   · (a)는 컨테이너의 **캡처** 단계에서 `stopPropagation()`으로 막는다.
    //     xterm은 textarea에 capture=true로 걸지만, 조상인 컨테이너의 캡처
    //     리스너가 언제나 먼저 돈다. 이 덕에 예전 판이 쓰던 xterm 사설 필드
    //     (`_keyDownSeen`/`_keyPressHandled`) 조작과 그 안전망이 통째로 사라졌다.
    //
    // 미러 자체는 inputType을 **믿지 않는다**. WebKit은 커밋 재전달·백스페이스
    // 등에서 insertText/insertReplacementText가 오락가락한다. 대신 "우리가 PTY에
    // 이미 반영해 둔 textarea 값"(prevValue)을 들고, 소유한 input마다 현재
    // `textarea.value`와 공통 접두사만 남긴 뒤
    // `DEL * (지울 길이) + 새 꼬리`를 **한 번의 write로** 보낸다. TUI 입장에선
    // 사람이 백스페이스로 고쳐 쓰는 것과 같아 조합이 화면에서 그대로 굴러가고,
    // 중간 상태가 깜빡이지도 않는다. 비교는 코드포인트 배열([...s])로 해서
    // 서로게이트 페어(이모지)가 반쪽 나지 않게 한다.
    //
    // 소유 판정:
    //   · imeComposing(진짜 composition 이벤트가 오는 IME — 일본어/중국어,
    //     Chrome/WebView2)이면 손 뗀다: stopPropagation도 write도 하지 않는다.
    //   · non-mac이면 손 뗀다 — 위 (1) 가드가 Windows 몫을 그대로 맡는다.
    //   · 그 외(맥 한글 조합·ASCII 본인의 input) → 우리 소유: stopPropagation +
    //     diff write. "xterm이 이미 같은 걸 보냈나"는 순서로 판별하지 않고 위
    //     (1.5) 원장으로 상쇄한다.
    const DEL = "\x7f";
    // WKWebView는 textarea 줄 끝 공백을 U+00A0(NBSP)로 채워 넣는다. 앱 실측:
    //   mirror patch=" " xtermSent=" " 인데도 상쇄 실패 → 공백 두 칸
    //   prev="가\u00a0" vs next="가 ㄴ" → 접두사가 1자만 일치 → 패치에 공백이 또 딸려 나감
    // xterm이 keypress로 보내는 건 U+0020이므로, textarea에서 읽는 값과 상쇄 비교는
    // 항상 이 정규화를 거친다. 사용자가 친 것은 어차피 평범한 공백이다.
    const NBSP = /\u00a0/g;
    const normalizeSpace = (v: string) => v.replace(NBSP, " ");
    const taValue = () => normalizeSpace(term.textarea?.value ?? "");
    let imeComposing = false;
    let pasteSeen = false;
    // 우리가 PTY에 이미 반영해 둔 textarea.value. 이 값과 현재 값의 차이가
    // 곧 PTY로 보낼 패치다.
    let prevValue = "";

    /**
     * 기준선 리싱크. xterm은 우리 모르게 textarea.value를 비운다 —
     * Enter/Ctrl+C의 `textarea.value = ''`(browser/Terminal.ts `_keyDown`)와
     * blur 핸들러가 그렇다. 커스텀 키 핸들러는 그 비우기 **전에** 불리므로
     * 지금 값을 읽으면 안 되고, 그 태스크의 동기 처리가 끝난 뒤에 읽어야 한다.
     * `input`은 별도 태스크라 마이크로태스크가 항상 그보다 먼저 끝난다.
     * 이 리싱크가 빠지면 다음 조합에서 사라진 길이만큼 DEL이 새어 나가
     * 진짜 프롬프트 글자를 지워 먹는다.
     */
    const resyncSoon = () => {
      const ta = term.textarea;
      if (!ta) return;
      queueMicrotask(() => {
        prevValue = taValue();
      });
    };

    const onContainerInput = (ev: Event) => {
      if (!IS_MAC) return; // Windows/Linux: 조합은 xterm+IME 몫
      const ta = term.textarea;
      if (!ta) return;
      if (imeComposing) return; // composition 경로가 살아 있는 IME — xterm 몫
      if (pasteSeen) {
        // 붙여넣기의 뒤끝 input. 내용은 xterm이 paste 이벤트에서 이미 보냈다.
        pasteSeen = false;
        prevValue = taValue();
        return;
      }
      const next = taValue();
      const before = [...prevValue];
      const after = [...next];
      let same = 0;
      while (
        same < before.length &&
        same < after.length &&
        before[same] === after[same]
      )
        same++;
      const patch =
        DEL.repeat(before.length - same) + after.slice(same).join("");

      ev.stopPropagation(); // (a) 차단: xterm `_inputEvent`가 이 이벤트를 못 본다
      prevValue = next;
      if (!patch) return;
      if (patch === normalizeSpace(xtermSent)) {
        // xterm이 이번 키에서 이미 똑같이 보냈다(스페이스의 keypress, ASCII 본인의
        // input…). 소비하고 버린다 — 기준선은 위에서 이미 갱신했다.
        xtermSent = "";
        return;
      }
      // 실측 트레이스 `keypress "," → input "ㄹ" → keyup ","`처럼 xterm이 보낸 것과
      // 다른 청크는 반드시 살아남는다 — 상쇄는 같은 문자열 1회뿐이다.
      writeInput(patch);
      mirrorSent = patch;
    };

    // The hidden textarea only exists after term.open(); attach() calls this.
    let compositionBound = false;
    const onCompositionStart = () => {
      imeComposing = true; // 이 플랫폼은 조합을 제대로 알려 준다 — 손 뗀다
      prevValue = taValue();
    };
    const onCompositionEnd = () => {
      imeComposing = false;
      compositionEndedAt = performance.now();
      // 커밋 문자열은 xterm이 보낸다(`_finalizeComposition`의 setTimeout(0)).
      // 우리는 그 결과를 기준선으로만 받아 적는다.
      resyncSoon();
    };
    const onTextareaBlur = () => {
      // xterm이 blur에서 textarea를 비운다 — 기준선도 같이 비워야 한다.
      resyncSoon();
    };
    const onTextareaPaste = () => {
      // xterm이 paste를 직접 처리해 PTY로 보내고 `textarea.value = ''`까지 한다
      // (browser/Clipboard.ts `paste`). 그 뒤 브라우저 기본 동작이 붙여넣은 글을
      // textarea에 넣으며 input을 한 번 더 쏘는데, 그건 이미 나간 내용이라
      // 우리가 diff로 또 보내면 안 된다 — 그 input은 기준선만 맞추고 넘긴다.
      // 마이크로태스크는 같은 태스크의 input보다 항상 나중이라 플래그가 남지 않는다.
      pasteSeen = true;
      const ta = term.textarea;
      if (!ta) return;
      queueMicrotask(() => {
        pasteSeen = false;
        prevValue = taValue();
      });
    };
    const bindComposition = () => {
      if (compositionBound) return;
      const ta = term.textarea;
      if (!ta) return;
      compositionBound = true;
      ta.addEventListener("compositionstart", onCompositionStart);
      ta.addEventListener("compositionend", onCompositionEnd);
      ta.addEventListener("blur", onTextareaBlur);
      ta.addEventListener("paste", onTextareaPaste);
      // input은 xterm의 textarea 리스너보다 **먼저** 받아야 한다(소유했을 때
      // stopPropagation으로 xterm을 통째로 비켜 세우기 때문). 같은 노드에 건
      // 리스너는 등록 순서를 따르는데 xterm이 open()에서 먼저 걸었으므로,
      // 조상인 컨테이너의 캡처 단계에 건다 — 캡처는 대상 노드의 리스너보다
      // 항상 앞선다.
      container.addEventListener("input", onContainerInput, true);
    };

    // Copy/paste key handling (fires only while THIS terminal is focused, so it is
    // naturally scoped per-agent in the multi-terminal keep-alive registry).
    term.attachCustomKeyEventHandler((event) => {
      // (1.5) 원장은 키 하나가 단위다. xterm이 이 키를 처리하기 **전에** 비운다.
      if (event.type === "keydown") {
        xtermSent = "";
        mirrorSent = "";
      }

      // 조합 키(keyCode 229)가 아닌 keydown은 xterm이 제어 시퀀스로 바꿔 보내
      // 줄 끝을 우리가 모르게 바꾸고(Enter/Backspace/화살표…), Enter·Ctrl+C는
      // textarea까지 비운다. 기준선을 그 뒤 값으로 다시 맞춘다.
      if (event.type === "keydown" && event.keyCode !== 229) resyncSoon();

      // (b) 차단: 이 false 하나가 `CompositionHelper._handleAnyTextareaChanges`
      // (setTimeout(0)으로 제멋대로 쓰는 세 번째 writer)를 통째로 막는다.
      // 진짜 조합 이벤트가 오는 IME(일본어/중국어)에서는 xterm이 조합을 제대로
      // 굴리고 있으므로 비켜 준다 — 거기서 막으면 조합 중 Enter가 통째로 증발한다.
      if (
        IS_MAC &&
        event.type === "keydown" &&
        !imeComposing &&
        (event.keyCode === 229 || event.isComposing)
      ) {
        return false;
      }
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
            // term.paste는 `textarea.value = ''`까지 한다(browser/Clipboard.ts).
            // keydown 시점의 리싱크는 이 시점보다 앞서므로 여기서 한 번 더 맞춘다 —
            // 안 그러면 다음 조합에서 사라진 길이만큼 DEL이 새어 나간다.
            resyncSoon();
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
