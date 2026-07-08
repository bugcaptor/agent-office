// src/renderer/layout/UIChrome.tsx
//
// Layer 1 of the app's z-stack: TopBar + notification
// ticker + BottomBar, positioned over the office canvas. This root container
// is `pointer-events: none` (see `layout.css`) so empty space passes clicks
// straight through to the office canvas underneath (Layer 0) — only the
// bars themselves and the ticker cards opt back in to `pointer-events: auto`.
//
// `SessionTimePanel` lives in the same `.ticker-column` as
// `NotificationTicker`, above it in document flow — final-review fix: it was
// originally a separate `position: fixed` overlay mounted in `App.tsx`,
// which visually overlapped both this column's ticker cards and the
// TopBar's stats. Sharing one column makes that overlap structurally
// impossible.
import { NotificationTicker } from "../notification/NotificationTicker";
import { SessionTimePanel } from "../timeline/SessionTimePanel";
import { BottomBar } from "./BottomBar";
import { TopBar } from "./TopBar";

export function UIChrome() {
  return (
    <div className="ui-chrome">
      <TopBar />
      <div className="ticker-column">
        <SessionTimePanel />
        <NotificationTicker />
      </div>
      <BottomBar />
    </div>
  );
}
