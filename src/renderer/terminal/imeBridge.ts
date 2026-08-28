// src/renderer/terminal/imeBridge.ts
//
// 터미널 한 대의 **키 입력 → PTY** 배선. 조합(IME) 때문에 세 플랫폼이 서로
// 다른 병을 앓아서, 이 계층만 따로 떼어 놓았다.
//
// 원래 `TerminalRegistry.ensure()` 안에 통째로 들어 있었다. 그 결과 356줄짜리
// 함수가 됐고, 무엇보다 **앱을 띄우지 않으면 한 줄도 검증할 수 없었다** —
// Safari로 재면 WKWebView와 다른 결과가 나오는 영역이라 브라우저 실험도
// 근거가 되지 못한다. 여기로 빼면서 플랫폼(`isMac`)과 시계(`now`)를 인자로
// 받게 했으니, 두 갈래 모두 가짜 이벤트로 태울 수 있다.
//
// 상태(원장·기준선·조합 플래그)를 한 클로저가 전부 쥐고 있다는 점은 그대로다 —
// 이것들은 서로를 상쇄하려고 존재하는 값이라 나눌 수 없다.
import type { Terminal } from "@xterm/xterm";

import { IS_MAC } from "../shared/platform";

export interface ImeBridgeDeps {
  term: Terminal;
  /** xterm이 붙는 컨테이너. `input`을 **캡처** 단계에서 가로채는 데 쓴다. */
  container: HTMLElement;
  /** 지금 이 탭이 로컬 키 입력을 받지 않는가(봇 운전 중 등). */
  inputBlocked: () => boolean;
  /** 실제로 PTY에 내보내는 경로. */
  send: (data: string) => void;
  /** 기본값은 실행 중인 플랫폼. 테스트가 두 갈래를 모두 태우려고 연다. */
  isMac?: boolean;
  /** 기본값은 `performance.now`. 중복 판정 창(window)을 테스트에서 조종한다. */
  now?: () => number;
}

export interface ImeBridge {
  /** `term.open()` 이후에 불러야 한다 — 숨은 textarea가 그때 생긴다. 멱등. */
  bindComposition: () => void;
  /**
   * 조합과 얽힌 키 처리. 반환값의 뜻:
   *   `false`/`true` — 최종 결론(각각 xterm에게 감추기 / xterm에게 넘기기)
   *   `null`         — 조합과 무관한 키다. 나머지 판단은 호출자 몫.
   */
  onKeyEvent: (event: KeyboardEvent) => boolean | null;
  /**
   * 기준선 리싱크. xterm이 우리 모르게 `textarea.value`를 비우는 일을 한
   * **직후**(예: `term.paste()`)에 호출자가 불러 준다.
   */
  resync: () => void;
}

export function createImeBridge(deps: ImeBridgeDeps): ImeBridge {
  const { term, container, inputBlocked, send } = deps;
  const isMac = deps.isMac ?? IS_MAC;
  const now = deps.now ?? (() => performance.now());

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
    if (inputBlocked()) {
      return;
    }
    const at = now();
    const isImeDuplicate =
      !isMac &&
      at - compositionEndedAt < IME_COMMIT_WINDOW_MS &&
      data === lastData &&
      at - lastDataAt < IME_DUP_ADJ_MS;
    if (isImeDuplicate) {
      lastData = ""; // consume: never drop a third identical emission
      return;
    }
    lastData = data;
    lastDataAt = at;
    send(data);
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
    if (!isMac) return; // Windows/Linux: 조합은 xterm+IME 몫
    const ta = term.textarea;
    if (!ta) return;
    if (imeComposing) return; // composition 경로가 살아 있는 IME — xterm 몫
    if (pasteSeen) {
      // 붙여넣기의 뒤끝 input. 내용은 xterm이 paste 이벤트에서 이미 보냈다.
      // 여기서 전파까지 끊지 않으면 xterm `_inputEvent`가 WebKit의
      // insertText를 다시 onData로 흘려 같은 문자열이 연속으로 붙는다.
      ev.stopPropagation();
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
    compositionEndedAt = now();
    // 커밋 문자열은 xterm이 보낸다(`_finalizeComposition`의 setTimeout(0)).
    // 우리는 그 결과를 기준선으로만 받아 적는다.
    resyncSoon();
  };
  const onTextareaBlur = () => {
    // xterm이 blur에서 textarea를 비운다 — 기준선도 같이 비워야 한다.
    resyncSoon();
  };
  const onTextareaPaste = (ev: ClipboardEvent) => {
    // xterm이 paste를 직접 처리해 PTY로 보내고 `textarea.value = ''`까지 한다
    // (browser/Clipboard.ts `paste`). macOS에서는 그 뒤 브라우저 기본 동작이
    // 붙여넣은 글을 textarea에 또 넣고 insertText input을 내는 경우가 있다.
    // xterm 리스너가 먼저 등록돼 이미 발신을 마쳤으므로 기본 동작을 취소하고,
    // 혹시 WebKit이 후속 input까지 내더라도 pasteSeen 분기에서 전파를 끊는다.
    if (!isMac) return;
    ev.preventDefault();
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

  const onKeyEvent = (event: KeyboardEvent): boolean | null => {
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
      isMac &&
      event.type === "keydown" &&
      !imeComposing &&
      (event.keyCode === 229 || event.isComposing)
    ) {
      return false;
    }
    if (event.isComposing || event.keyCode === 229) return true; // let xterm/IME own composition
    return null; // 조합과 무관 — 복사·붙여넣기 판단은 호출자 몫이다
  };

  return { bindComposition, onKeyEvent, resync: resyncSoon };
}
