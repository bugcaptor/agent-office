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
import { useAppStore } from "../store/appStore";
import { AgentTabStrip } from "./AgentTabStrip";
import { TerminalHost } from "./TerminalHost";

export function TerminalOverlay() {
  const isOpen = useAppStore((s) => s.activeTerminalAgentId !== null);
  const closeTerminal = useAppStore((s) => s.closeTerminal);

  return (
    <div
      className="terminal-overlay"
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
        <TerminalHost />
      </div>
    </div>
  );
}
