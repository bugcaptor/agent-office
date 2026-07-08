import "./layout/layout.css";

import { officeBus } from "./ipc/sessionBridge";
import { OfficeCanvas } from "./office/OfficeCanvas";
import type { AgentProfile as OfficeAgentProfile } from "./office/types";
import { AgentHoverCard } from "./portrait/AgentHoverCard";
import { ProfileDialog } from "./profile/ProfileDialog";
import { ConfirmDeleteDialog } from "./agent/ConfirmDeleteDialog";
import { ConfirmRestartDialog } from "./agent/ConfirmRestartDialog";
import { ConfirmQuitDialog } from "./agent/ConfirmQuitDialog";
import { SettingsDialog } from "./settings/SettingsDialog";
import { FirstRunDialog } from "./settings/FirstRunDialog";
import { useAppStore } from "./store/appStore";
import { useAgentList } from "./store/selectors";
import { THEMES } from "./theme/themes";
import { TaskLabelLayer } from "./labels/TaskLabelLayer";
import { TerminalOverlay } from "./terminal/TerminalOverlay";
import { UIChrome } from "./layout/UIChrome";

// Root component: the 4-layer z-stack.
//
//   Layer 0 (z:0)  OfficeCanvas    -- subsystem B, wired to the store-backed
//                                     `officeBus` and the agent list
//                                     (`useAgentList`, a `useShallow`
//                                     selector -- a stable reference across
//                                     renders so B's `syncAgents` effect
//                                     doesn't loop).
//   Layer 0.5 (z:5) TaskLabelLayer   -- 머리 위 작업 라벨(DOM, pointer-events:none)
//   Layer 1 (z:10) UIChrome        -- TopBar/SessionTimePanel/NotificationTicker/
//                                     BottomBar, pointer-events:none container.
//                                     SessionTimePanel is mounted inside
//                                     UIChrome's `.ticker-column`, above the
//                                     ticker (final-review fix; see
//                                     UIChrome.tsx).
//   Layer 2 (z:20) TerminalOverlay -- always mounted (keep-alive); closed =
//                                     display:none, never unmounted.
//   Layer 3 (z:30) ModalRoot       -- ProfileDialog/ConfirmDeleteDialog/
//                                     ConfirmRestartDialog/ConfirmQuitDialog/
//                                     SettingsDialog/FirstRunDialog, all
//                                     always mounted, each self-gated (`null`
//                                     render) on `modal.kind` except
//                                     FirstRunDialog which gates on
//                                     `settingsFirstRun`.
function App() {
  const agents = useAgentList();
  // The store's `AgentProfile` (src/shared/types.ts) is structurally
  // richer than office/types.ts's `AgentProfile` (id/name/role/seed + an
  // index signature -- a deliberately-decoupled contract) but lacks
  // that index signature itself, so TS's index-signature check needs an
  // explicit assertion at this boundary -- every field the office contract
  // reads is present.
  const officeProfiles = agents as unknown as readonly OfficeAgentProfile[];
  // Custom-sprite decode completion doesn't change any `AgentProfile` field,
  // so `syncAgents`'s profile-identity diff alone would miss it -- this
  // selector re-triggers B's resync effect whenever a sprite preview is
  // added/updated/removed (see `useOfficeScene`'s `resyncSignal` param).
  const spritePreviews = useAppStore((s) => s.spritePreviews);
  // 테마 -> Pixi 팔레트. `THEMES[..].pixi`는 모듈 상수라 참조가 안정적 —
  // 테마가 실제로 바뀔 때만 B의 setTheme 효과가 발화한다. DOM 쪽 토큰은
  // `applyTheme`(store.setTheme / main.tsx 부트)이 이미 처리한다.
  const themeId = useAppStore((s) => s.theme);
  const pixiPalette = THEMES[themeId].pixi;

  return (
    <div className="app-root">
      <OfficeCanvas
        bus={officeBus}
        profiles={officeProfiles}
        resyncSignal={spritePreviews}
        pixiPalette={pixiPalette}
      />
      <TaskLabelLayer bus={officeBus} />
      <UIChrome />
      <AgentHoverCard />
      <TerminalOverlay />
      <div className="modal-root">
        <ProfileDialog />
        <ConfirmDeleteDialog />
        <ConfirmRestartDialog />
        <ConfirmQuitDialog />
        <SettingsDialog />
        <FirstRunDialog />
      </div>
    </div>
  );
}

export default App;
