// src/web/KeyBar.tsx
//
// 소프트 키보드 보조바. 에이전트 CLI는 Esc와 방향키가 필수인데 모바일 키보드에
// 없다 — 이게 없으면 "보기"는 되고 "작업"은 안 된다(#7m §H). 가치의 절반이라
// Phase 1에 포함한다.
//
// Ctrl은 **래치**다: 한 번 누르면 다음 키 한 번에만 적용되고 해제된다.
// (폰에서 두 손가락 동시 입력은 비현실적이다.)

import { useState } from "react";

interface Props {
  onKey: (data: string) => void;
}

/** Ctrl+<문자> → 제어 문자(예: c → \x03). */
function ctrlOf(ch: string): string | null {
  const upper = ch.toUpperCase();
  const code = upper.charCodeAt(0);
  if (code >= 64 && code <= 95) return String.fromCharCode(code - 64);
  if (upper === " ") return "\x00";
  return null;
}

const KEYS: Array<{ label: string; data: string }> = [
  { label: "Esc", data: "\x1b" },
  { label: "Tab", data: "\t" },
  { label: "←", data: "\x1b[D" },
  { label: "↓", data: "\x1b[B" },
  { label: "↑", data: "\x1b[A" },
  { label: "→", data: "\x1b[C" },
  { label: "⏎", data: "\r" },
  { label: "⌫", data: "\x7f" },
];

/** 자주 쓰는 Ctrl 조합 — 래치를 거치지 않는 직행 버튼. */
const CTRL_SHORTCUTS: Array<{ label: string; ch: string }> = [
  { label: "^C", ch: "c" },
  { label: "^D", ch: "d" },
  { label: "^Z", ch: "z" },
];

export function KeyBar({ onKey }: Props) {
  const [ctrlLatched, setCtrlLatched] = useState(false);
  const [text, setText] = useState("");

  const press = (data: string) => {
    if (ctrlLatched && data.length === 1) {
      const c = ctrlOf(data);
      setCtrlLatched(false);
      if (c) {
        onKey(c);
        return;
      }
    }
    onKey(data);
  };

  return (
    <div className="keybar">
      {/* xterm의 숨은 textarea는 모바일 IME(한글 조합·자동완성)와 상성이 나쁘다.
          별도 입력칸에서 조합을 끝낸 뒤 통째로 보내면 그 문제를 피한다. */}
      <form
        className="keybar-input"
        onSubmit={(e) => {
          e.preventDefault();
          if (text) onKey(text);
          onKey("\r");
          setText("");
        }}
      >
        <input
          type="text"
          value={text}
          placeholder="입력 후 전송"
          autoCapitalize="off"
          autoCorrect="off"
          spellCheck={false}
          onChange={(e) => setText(e.target.value)}
        />
        <button className="btn small" type="submit">
          전송
        </button>
      </form>
      <div className="keybar-row">
        <button
          className={`key ${ctrlLatched ? "latched" : ""}`}
          onClick={() => setCtrlLatched((v) => !v)}
        >
          Ctrl
        </button>
        {CTRL_SHORTCUTS.map((k) => (
          <button key={k.label} className="key" onClick={() => onKey(ctrlOf(k.ch) ?? "")}>
            {k.label}
          </button>
        ))}
        {KEYS.map((k) => (
          <button key={k.label} className="key" onClick={() => press(k.data)}>
            {k.label}
          </button>
        ))}
      </div>
    </div>
  );
}
