// src/renderer/terminal/TerminalOverlay.tsx
//
// Central overlay panel hosting the terminal tab strip + keep-alive host.
//
// CRITICAL invariant: "closed" is a `display:none` toggle on this root, NOT
// a conditional `{isOpen && ...}` render. `AgentTabStrip`/`TerminalHost` (and
// everything TerminalHost keeps mounted underneath — the xterm instances in
// TerminalRegistry) must never be torn down just because the overlay is
// closed; that is the whole point of the keep-alive design (README). Only
// `removeAgent` ever really destroys a terminal.
//
// Close paths (3): X button (AgentTabStrip), Cmd/Ctrl+W (AgentTabStrip), and
// backdrop mousedown (this file — mousedown directly on `.terminal-overlay`,
// outside the panel). Escape is deliberately NOT a close path anywhere —
// TUI apps under the terminal (vim etc.) need a real Escape keystroke; see
// AgentTabStrip's header.
//
// 뷰 모드(이슈 #69): filled에서는 패널이 오버레이 루트를 완전히 덮으므로
// backdrop mousedown 닫기 경로가 도달 불가 — 이 모드에서는 X 버튼/Cmd+W가
// 유일한 닫기 경로다(의도된 동작).
import { useAppStore } from "../store/appStore";
import { AgentTabStrip } from "./AgentTabStrip";
import { TerminalSummaryBar } from "./TerminalSummaryBar";
import { TerminalHost } from "./TerminalHost";

export function TerminalOverlay() {
  const isOpen = useAppStore((s) => s.activeTerminalAgentId !== null);
  const closeTerminal = useAppStore((s) => s.closeTerminal);
  // 뷰 모드(이슈 #69): 루트에 mode-* 클래스를 붙여 패널 크기/배경 딤을 CSS로만
  // 토글한다. 조건부 렌더가 아니라 클래스 변경이므로 keep-alive 불변식과 무관 —
  // 패널이 커지면 TerminalHost의 ResizeObserver가 자동 refit 한다.
  const viewMode = useAppStore((s) => s.terminalViewMode);

  return (
    <div
      className={`terminal-overlay mode-${viewMode}`}
      style={{ display: isOpen ? "flex" : "none" }}
      // mousedown, not click: a click's target is resolved from where the
      // mouseup lands, so dragging a text selection inside the terminal and
      // releasing the mouse over the backdrop would fire a click whose
      // target bubbles up as the backdrop itself, closing the overlay
      // unintentionally. mousedown fires at the press point, so only an
      // actual press on the backdrop (not the panel) triggers this. Also
      // guard on button === 0 (primary/left) so a right- or middle-click
      // on the backdrop (e.g. to open a context menu) doesn't close it.
      onMouseDown={(e) => {
        if (e.button === 0 && e.target === e.currentTarget) closeTerminal();
      }}
    >
      <div className="terminal-overlay-panel">
        <AgentTabStrip />
        {/* 활성 탭 요약 바(이슈 #44 T1). 탭 스트립과 호스트 사이에 상시 마운트
            — 표시는 오버레이 display 토글이 담당하므로 불변식과 무관하다. */}
        <TerminalSummaryBar />
        <TerminalHost />
      </div>
    </div>
  );
}
