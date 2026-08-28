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
import { PostItWidget } from "../memo/PostItWidget";
import { UsageFloat } from "../usage/UsageFloat";

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
        {/* 포스트잇 메모(이슈 #79). 패널 우상단 absolute — 위 형제들과 달리
            조건부 렌더여도 무해하다(keep-alive 불변식은 AgentTabStrip/
            TerminalHost와 그 아래 xterm에만 걸린 것이고, 이 위젯의 본문
            진실은 디스크다). 패널이 positioning context가 되도록
            layout.css의 .terminal-overlay-panel에 position:relative가 있다. */}
        <PostItWidget />
        {/* filled 뷰 모드 사용량 플로팅(이슈 #69). PostItWidget과 같은 이유로
            조건부 렌더여도 무해하다 — keep-alive 불변식은 AgentTabStrip/
            TerminalHost(그 아래 xterm)에만 걸려 있고, 이 위젯은 그 트리와
            무관한 형제이며 진실은 스토어(s.usage)에 있다. 표시 여부는
            컴포넌트 내부(뷰 모드 + 오버레이 열림 + 표시할 provider 존재)가 스스로
            판단한다. */}
        <UsageFloat />
      </div>
    </div>
  );
}
