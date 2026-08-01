// src/renderer/shared/useEscapeToClose.ts
//
// 오버레이/다이얼로그 공통 "Esc로 닫기" 훅. MemoArchiveDialog / SessionLogDialog /
// DiaryDialog가 글자 하나 다르지 않은 같은 useEffect를 각자 갖고 있던 것을 뽑았다.
//
// 핵심은 **캡처 단계**(`addEventListener(..., true)`)로 듣고 `stopPropagation`
// 하는 것 — 그래야 Esc가 터미널(xterm)이나 전역 단축키로 새지 않는다. 열려
// 있을 때만 리스너를 걸고, 닫히거나 언마운트되면 같은 캡처 플래그로 정확히
// 떼어낸다.
import { useEffect } from "react";

/**
 * `open`인 동안 window에서 Esc를 캡처해 `onClose`를 부른다(전파는 멈춤).
 *
 * @param open 오버레이가 떠 있는지. false면 리스너를 걸지 않는다.
 * @param onClose Esc 때 호출할 닫기 함수. 참조가 바뀌면 리스너를 다시 건다.
 */
export function useEscapeToClose(open: boolean, onClose: () => void): void {
  useEffect(() => {
    if (!open) return;
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") {
        e.stopPropagation();
        onClose();
      }
    };
    window.addEventListener("keydown", onKey, true);
    return () => window.removeEventListener("keydown", onKey, true);
  }, [open, onClose]);
}
