// @vitest-environment jsdom
//
// src/renderer/terminal/__tests__/imeBridge.test.ts
//
// 조합(IME) 배선의 단위 테스트. 이 파일이 존재할 수 있게 된 것이
// `imeBridge.ts` 분리의 목적이었다 — 이 로직이 `TerminalRegistry.ensure()`
// 안에 있던 동안에는 앱(WKWebView)을 띄우지 않고는 한 줄도 확인할 수 없었고,
// Safari로 재면 다른 결과가 나오는 영역이라 브라우저 실험도 근거가 못 됐다.
//
// 여기서 검증하는 것은 **결정 로직**이다: 어떤 이벤트 조합이 오면 무엇을 PTY로
// 보내고 무엇을 상쇄하는가. WKWebView가 실제로 어떤 이벤트를 쏘는지는 여전히
// 앱 안에서 재야 하고(그 실측 결과는 imeBridge.ts 주석에 박혀 있다), 이 테스트는
// 그 실측을 코드로 옮긴 부분이 맞게 굴러가는지를 본다.
import { describe, expect, it } from "vitest";
import type { Terminal } from "@xterm/xterm";

import { createImeBridge } from "../imeBridge";

/** 브리지가 실제로 만지는 xterm 표면은 `onData`와 `textarea` 둘뿐이다. */
function fakeTerm(textarea: HTMLTextAreaElement) {
  let onData: ((data: string) => void) | undefined;
  return {
    term: { textarea, onData: (cb: (d: string) => void) => (onData = cb) } as unknown as Terminal,
    /** xterm이 스스로 결론 내 보내는 청크(keypress·제어키·paste)를 흉내 낸다. */
    emitData: (data: string) => onData?.(data),
  };
}

interface Harness {
  container: HTMLDivElement;
  textarea: HTMLTextAreaElement;
  sent: string[];
  emitData: (data: string) => void;
  bridge: ReturnType<typeof createImeBridge>;
  clock: { t: number };
}

function harness(opts: { isMac: boolean; blocked?: boolean }): Harness {
  const container = document.createElement("div");
  const textarea = document.createElement("textarea");
  container.appendChild(textarea);
  document.body.appendChild(container);

  const { term, emitData } = fakeTerm(textarea);
  const sent: string[] = [];
  const clock = { t: 1000 };
  const bridge = createImeBridge({
    term,
    container,
    inputBlocked: () => opts.blocked ?? false,
    send: (d) => sent.push(d),
    isMac: opts.isMac,
    now: () => clock.t,
  });
  bridge.bindComposition();
  return { container, textarea, sent, emitData, bridge, clock };
}

/** 숨은 textarea의 값이 바뀌고 `input`이 뜨는 흐름(WKWebView의 한글 조합).
 *  반환값은 "브리지가 이 이벤트의 전파를 끊었는가" — 끊었다면 xterm의
 *  `_inputEvent`는 이 이벤트를 아예 보지 못한다. */
function typeInto(h: Harness, value: string): { stopped: boolean } {
  h.textarea.value = value;
  const ev = new Event("input", { bubbles: true });
  let stopped = false;
  const real = ev.stopPropagation.bind(ev);
  ev.stopPropagation = () => {
    stopped = true;
    real();
  };
  h.textarea.dispatchEvent(ev);
  return { stopped };
}

function keydown(keyCode: number, isComposing = false): KeyboardEvent {
  return { type: "keydown", keyCode, isComposing } as unknown as KeyboardEvent;
}

describe("imeBridge · 조합 키 판정(onKeyEvent)", () => {
  it("맥에서 조합 키(keyCode 229)는 xterm에게 감춘다", () => {
    // 이 false 하나가 `CompositionHelper._handleAnyTextareaChanges`(비동기로
    // 제멋대로 쓰는 세 번째 writer)를 막는다.
    const h = harness({ isMac: true });
    expect(h.bridge.onKeyEvent(keydown(229))).toBe(false);
  });

  it("맥이 아니면 조합은 xterm+IME 몫이라 그대로 넘긴다", () => {
    const h = harness({ isMac: false });
    expect(h.bridge.onKeyEvent(keydown(229))).toBe(true);
  });

  it("진짜 composition 이벤트가 오는 IME라면 맥에서도 비켜 준다", () => {
    // 일본어/중국어처럼 조합을 제대로 알려 주는 IME. 여기서 막으면 조합 중
    // Enter가 통째로 증발한다.
    const h = harness({ isMac: true });
    h.textarea.dispatchEvent(new Event("compositionstart"));
    expect(h.bridge.onKeyEvent(keydown(229))).toBe(true);
  });

  it("조합과 무관한 키는 null — 복사·붙여넣기 판단을 호출자에게 넘긴다", () => {
    const h = harness({ isMac: true });
    expect(h.bridge.onKeyEvent(keydown(67))).toBeNull();
  });
});

describe("imeBridge · macOS 미러(우리가 유일한 writer)", () => {
  it("textarea 값의 차이만큼 DEL+새 꼬리를 한 번에 보낸다", () => {
    const h = harness({ isMac: true });
    h.bridge.onKeyEvent(keydown(229));
    typeInto(h, "ㄱ");
    expect(h.sent).toEqual(["ㄱ"]);

    // 조합이 굴러간다: "ㄱ" → "가"는 한 글자를 지우고 새로 쓴다.
    h.bridge.onKeyEvent(keydown(229));
    typeInto(h, "가");
    expect(h.sent).toEqual(["ㄱ", "\x7f가"]);
  });

  it("소유한 input은 전파를 끊어 xterm `_inputEvent`가 못 보게 한다", () => {
    const h = harness({ isMac: true });
    h.bridge.onKeyEvent(keydown(229));
    expect(typeInto(h, "ㄱ").stopped).toBe(true);
  });

  it("서로게이트 페어를 반쪽 내지 않는다(코드포인트 단위 비교)", () => {
    const h = harness({ isMac: true });
    typeInto(h, "😀");
    expect(h.sent).toEqual(["😀"]);
    typeInto(h, "");
    // 이모지는 UTF-16으로 두 칸이지만 사람이 지운 것은 한 글자다.
    expect(h.sent).toEqual(["😀", "\x7f"]);
  });

  it("줄 끝 NBSP를 평범한 공백으로 정규화한다", () => {
    // WKWebView는 textarea 줄 끝 공백을 U+00A0로 채운다. 정규화가 없으면
    // 접두사 비교가 어긋나 공백이 두 칸 들어간다(실측 버그).
    const h = harness({ isMac: true });
    typeInto(h, "가\u00a0");
    expect(h.sent).toEqual(["가 "]);
  });

  it("맥이 아니면 미러가 아예 손을 대지 않는다", () => {
    const h = harness({ isMac: false });
    expect(typeInto(h, "a").stopped).toBe(false);
    expect(h.sent).toEqual([]);
  });

  it("봇이 몰고 있는 탭은 사람 입력을 PTY로 보내지 않는다", () => {
    const h = harness({ isMac: true, blocked: true });
    typeInto(h, "ㄱ");
    expect(h.sent).toEqual([]);
  });
});

describe("imeBridge · 키별 원장(중복 발신 상쇄)", () => {
  it("미러가 보낸 청크를 xterm이 또 보내려 하면 정확히 1회 상쇄한다", () => {
    // 스페이스가 두 칸 들어가던 버그: input(미러)이 keypress(xterm)보다 먼저 온다.
    const h = harness({ isMac: true });
    h.bridge.onKeyEvent(keydown(32));
    typeInto(h, " ");
    expect(h.sent).toEqual([" "]);
    h.emitData(" "); // xterm의 keypress 결론 — 상쇄된다
    expect(h.sent).toEqual([" "]);
  });

  it("xterm이 먼저 보낸 청크를 미러가 또 보내려 해도 상쇄한다(순서 무관)", () => {
    const h = harness({ isMac: true });
    h.bridge.onKeyEvent(keydown(65));
    h.emitData("a");
    expect(h.sent).toEqual(["a"]);
    typeInto(h, "a"); // 같은 키의 뒤늦은 input
    expect(h.sent).toEqual(["a"]);
  });

  it("상쇄는 같은 문자열 1회뿐 — 다른 청크는 반드시 살아남는다", () => {
    // 실측 트레이스 `keypress "," → input "ㄹ"`. 순서 전제로 막으면 글자가 죽는다.
    const h = harness({ isMac: true });
    h.bridge.onKeyEvent(keydown(229));
    h.emitData(",");
    typeInto(h, "ㄹ");
    expect(h.sent).toEqual([",", "ㄹ"]);
  });

  it("같은 글자를 연달아 쳐도 두 번째가 먹히지 않는다(keydown마다 원장이 빈다)", () => {
    const h = harness({ isMac: true });
    h.bridge.onKeyEvent(keydown(32));
    h.emitData(" ");
    h.bridge.onKeyEvent(keydown(32)); // 새 키 — 원장 초기화
    h.emitData(" ");
    expect(h.sent).toEqual([" ", " "]);
  });
});

describe("imeBridge · Windows 이중 입력 가드", () => {
  it("compositionend 직후 연달아 온 같은 청크의 둘째를 버린다", () => {
    // WebView2에서 compositionend 경로 위로 음절이 한 번 더 나가는 버그
    // ("여러번" → "여여러러번번").
    const h = harness({ isMac: false });
    h.textarea.dispatchEvent(new Event("compositionend"));
    h.emitData("번");
    h.clock.t += 5;
    h.emitData("번");
    expect(h.sent).toEqual(["번"]);
  });

  it("세 번째 같은 청크는 다시 살린다(진짜 연타를 먹지 않는다)", () => {
    const h = harness({ isMac: false });
    h.textarea.dispatchEvent(new Event("compositionend"));
    h.emitData("번");
    h.clock.t += 5;
    h.emitData("번"); // 버려짐
    h.clock.t += 5;
    h.emitData("번");
    expect(h.sent).toEqual(["번", "번"]);
  });

  it("커밋에서 멀리 떨어진 반복은 진짜 입력이라 통과시킨다", () => {
    const h = harness({ isMac: false });
    h.textarea.dispatchEvent(new Event("compositionend"));
    h.emitData("번");
    h.clock.t += 200; // 커밋 창(80ms) 밖
    h.emitData("번");
    expect(h.sent).toEqual(["번", "번"]);
  });

  it("맥에서는 이 가드가 아예 걸리지 않는다", () => {
    // 맥에는 composition 이벤트가 안 오므로 걸릴 일이 없고, 걸린다면 진짜
    // 입력을 먹는 쪽이다.
    const h = harness({ isMac: true });
    h.textarea.dispatchEvent(new Event("compositionend"));
    h.emitData("번");
    h.clock.t += 5;
    h.emitData("번");
    expect(h.sent).toEqual(["번", "번"]);
  });
});
