// src/renderer/shortcuts/accelerator.ts
//
// 사용자가 지정하는 단축키(에이전트별 터미널 열기, 알림 에이전트로 가기)의
// 문자열 형식·판정·표시. 앱 창 안에서만 쓰며 OS 전역 등록은 하지 않는다.
//
// 형식: `Ctrl+Alt+Shift+Meta+<KeyboardEvent.code>` — 수식키는 이 순서로 있는
// 것만 쓴다(예: `Meta+Alt+Digit1`, `Ctrl+Alt+KeyJ`). `key`가 아니라 `code`를
// 쓰는 이유: mac은 Option이 글자를 바꾸고(⌥1 = "¡"), 자판 배열마다 `key`가
// 달라져 같은 물리 키를 같은 단축키로 볼 수 없다.

/** 수식키 자체의 code. 이 키만 눌린 상태는 단축키가 아니다. */
const MODIFIER_CODES = new Set([
  "ControlLeft", "ControlRight", "AltLeft", "AltRight", "ShiftLeft", "ShiftRight",
  "MetaLeft", "MetaRight", "OSLeft", "OSRight", "CapsLock", "Fn", "FnLock", "AltGraph",
]);

interface KeyLike {
  code: string;
  key: string;
  ctrlKey: boolean;
  altKey: boolean;
  shiftKey: boolean;
  metaKey: boolean;
}

/** 키 이벤트 → 단축키 문자열. 수식키만 눌렸거나 code가 없으면 null. */
export function acceleratorFromEvent(e: KeyLike): string | null {
  if (!e.code || MODIFIER_CODES.has(e.code)) return null;
  const parts: string[] = [];
  if (e.ctrlKey) parts.push("Ctrl");
  if (e.altKey) parts.push("Alt");
  if (e.shiftKey) parts.push("Shift");
  if (e.metaKey) parts.push("Meta");
  parts.push(e.code);
  return parts.join("+");
}

interface Parsed {
  ctrl: boolean;
  alt: boolean;
  shift: boolean;
  meta: boolean;
  code: string;
}

function parse(acc: string): Parsed | null {
  const parts = acc.split("+");
  const code = parts.pop();
  if (!code) return null;
  const mods = new Set(parts);
  return {
    ctrl: mods.has("Ctrl"),
    alt: mods.has("Alt"),
    shift: mods.has("Shift"),
    meta: mods.has("Meta"),
    code,
  };
}

/** `Digit1`·`KeyJ`가 자판 그대로 찍는 글자(소문자). 그 밖의 code는 null. */
function plainChar(code: string): string | null {
  const digit = /^Digit(\d)$/.exec(code);
  if (digit) return digit[1];
  const letter = /^Key([A-Z])$/.exec(code);
  if (letter) return letter[1].toLowerCase();
  return null;
}

/**
 * 이 키 이벤트가 단축키와 같은가. Windows·Linux에서는 AltGr가 Ctrl+Alt로
 * 들어오므로, Ctrl+Alt+숫자/글자가 자판의 다른 글자(독일어 AltGr+7 = "{")를
 * 찍는 경우는 글자 입력으로 보고 단축키로 치지 않는다.
 */
export function matchesAccelerator(e: KeyLike, acc: string, isMac: boolean): boolean {
  if (!acc) return false;
  const pressed = acceleratorFromEvent(e);
  if (pressed !== acc) return false;
  if (!isMac && e.ctrlKey && e.altKey && !e.metaKey && !e.shiftKey) {
    const expected = plainChar(e.code);
    if (expected !== null && e.key.length === 1 && e.key.toLowerCase() !== expected) {
      return false;
    }
  }
  return true;
}

const MAC_SYMBOL = { ctrl: "⌃", alt: "⌥", shift: "⇧", meta: "⌘" } as const;

const KEY_LABEL: Record<string, string> = {
  ArrowLeft: "←",
  ArrowRight: "→",
  ArrowUp: "↑",
  ArrowDown: "↓",
  Backquote: "`",
  Minus: "-",
  Equal: "=",
  BracketLeft: "[",
  BracketRight: "]",
  Backslash: "\\",
  Semicolon: ";",
  Quote: "'",
  Comma: ",",
  Period: ".",
  Slash: "/",
  Escape: "Esc",
  Space: "Space",
  Enter: "Enter",
  Tab: "Tab",
  Backspace: "Backspace",
  PageUp: "PgUp",
  PageDown: "PgDn",
};

function keyLabel(code: string): string {
  const char = plainChar(code);
  if (char !== null) return char.toUpperCase();
  const numpad = /^Numpad(.+)$/.exec(code);
  if (numpad) return `Num${numpad[1]}`;
  return KEY_LABEL[code] ?? code;
}

/** 화면 표시. mac은 기호(⌃⌥⇧⌘), 그 밖은 `Ctrl+Alt+Shift+Win+키`. */
export function formatAccelerator(acc: string, isMac: boolean): string {
  const p = parse(acc);
  if (!p) return acc;
  const key = keyLabel(p.code);
  if (isMac) {
    return (
      (p.ctrl ? MAC_SYMBOL.ctrl : "") +
      (p.alt ? MAC_SYMBOL.alt : "") +
      (p.shift ? MAC_SYMBOL.shift : "") +
      (p.meta ? MAC_SYMBOL.meta : "") +
      key
    );
  }
  const mods: string[] = [];
  if (p.ctrl) mods.push("Ctrl");
  if (p.alt) mods.push("Alt");
  if (p.shift) mods.push("Shift");
  if (p.meta) mods.push("Win");
  return [...mods, key].join("+");
}

/** 추천 수식키: mac ⌘⌥, 그 밖 Ctrl+Alt. TUI·셸이 거의 쓰지 않는 조합이다. */
function recommendedPrefix(isMac: boolean): string {
  return isMac ? "Alt+Meta" : "Ctrl+Alt";
}

/** 에이전트 목록에서 index(0부터)번째 에이전트의 추천 단축키. 1~9번째만 있다. */
export function recommendedAgentShortcut(index: number, isMac: boolean): string | null {
  if (index < 0 || index > 8) return null;
  return `${recommendedPrefix(isMac)}+Digit${index + 1}`;
}

/** 알림 에이전트로 가기 추천 단축키(J = Jump). */
export function recommendedNotifyShortcut(isMac: boolean): string {
  return `${recommendedPrefix(isMac)}+KeyJ`;
}

export type AcceleratorProblem = "needsModifier" | "terminalKey" | "reserved";

const FUNCTION_KEY = /^F\d{1,2}$/;

/** 앱이 이미 쓰는 키와 OS가 가져가는 키. `acceleratorFromEvent` 형식. */
function buildReservedSet(isMac: boolean): ReadonlySet<string> {
  const set = new Set<string>();
  // 터미널 탭 키(AgentTabStrip·TerminalRegistry)는 두 OS 모두 Ctrl로도 받는다.
  for (let d = 1; d <= 9; d++) set.add(`Ctrl+Digit${d}`);
  for (const k of ["Ctrl+KeyW", "Ctrl+Shift+KeyE", "Ctrl+KeyC", "Ctrl+KeyV", "Ctrl+Shift+KeyC", "Ctrl+Shift+KeyV"]) {
    set.add(k);
  }
  if (isMac) {
    for (const k of ["W", "Q", "H", "M", "C", "V", "X", "Z", "A", "N", "T"]) set.add(`Meta+Key${k}`);
    for (const k of ["Tab", "Space", "Backquote", "Comma"]) set.add(`Meta+${k}`);
    for (let d = 1; d <= 9; d++) set.add(`Meta+Digit${d}`);
    set.add("Shift+Meta+KeyE");
    set.add("Ctrl+Meta+KeyF");
    for (const k of ["KeyH", "KeyM", "KeyD", "KeyI", "Escape"]) set.add(`Alt+Meta+${k}`);
  } else {
    set.add("Ctrl+Alt+Delete");
    set.add("Alt+F4");
  }
  return set;
}

const MAC_RESERVED = buildReservedSet(true);
const OTHER_RESERVED = buildReservedSet(false);

/**
 * 단축키로 쓸 수 없는 이유. 쓸 수 있으면 null.
 * - needsModifier: Ctrl·Alt·⌘ 중 하나도 없다.
 * - terminalKey: 셸·TUI가 받는 키다(Ctrl+글자는 제어 문자, Alt+글자는 Meta 키).
 *   ⌘ 또는 Ctrl+Alt가 들어가거나 F키면 통과.
 * - reserved: 앱(탭 번호·닫기·복사 등)이나 OS가 이미 쓴다. Windows·Linux의
 *   Win/Super 조합도 OS가 가져가므로 여기에 든다.
 */
export function acceleratorProblem(acc: string, isMac: boolean): AcceleratorProblem | null {
  const p = parse(acc);
  if (!p) return "needsModifier";
  if (!p.ctrl && !p.alt && !p.meta) return "needsModifier";
  if ((isMac ? MAC_RESERVED : OTHER_RESERVED).has(acc)) return "reserved";
  if (!isMac && p.meta) return "reserved";
  if (!p.meta && !(p.ctrl && p.alt) && !FUNCTION_KEY.test(p.code)) return "terminalKey";
  return null;
}
