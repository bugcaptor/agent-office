import "./layout/layout.css";

import { officeBus } from "./ipc/sessionBridge";
import { OfficeCanvas } from "./office/OfficeCanvas";
import type { AgentProfile as OfficeAgentProfile } from "./office/types";
import { AgentHoverCard } from "./portrait/AgentHoverCard";
import { DeskAssignMenu } from "./desk/DeskAssignMenu";
import { ProfileDialog } from "./profile/ProfileDialog";
import { ConfirmDeleteDialog } from "./agent/ConfirmDeleteDialog";
import { ConfirmRestartDialog } from "./agent/ConfirmRestartDialog";
import { ConfirmResumeDialog } from "./agent/ConfirmResumeDialog";
import { ConfirmTerminateDialog } from "./agent/ConfirmTerminateDialog";
import { ConfirmClockOutDialog } from "./agent/ConfirmClockOutDialog";
import { ConfirmQuitDialog } from "./agent/ConfirmQuitDialog";
import { SettingsDialog } from "./settings/SettingsDialog";
import { FirstRunDialog } from "./settings/FirstRunDialog";
import { AnalyticsDialog } from "./analytics/AnalyticsDialog";
import { UsageDialog } from "./usage/UsageDialog";
import { AboutDialog } from "./about/AboutDialog";
import { useAppStore } from "./store/appStore";
import { useAgentList, useLightsOff } from "./store/selectors";
import { THEMES } from "./theme/themes";
import { TaskLabelLayer } from "./labels/TaskLabelLayer";
import { TerminalOverlay } from "./terminal/TerminalOverlay";
import { MarkdownPalette } from "./markdown/MarkdownPalette";
import { MarkdownEditorOverlay } from "./markdown/MarkdownEditorOverlay";
import { UIChrome } from "./layout/UIChrome";

// Root component: the 4-layer z-stack.
//
//   Layer 0 (z:0)  OfficeCanvas    -- subsystem B, wired to the store-backed
//                                     `officeBus` and the agent list
//                                     (`useAgentList`, a `useShallow`
//                                     selector -- a stable reference across
//                                     renders so B's `syncAgents` effect
//                                     doesn't loop).
//   Layer 0.4 (z:3) .office-lights-off -- 전원 퇴근 시(에이전트가 하나 이상
//                                     있고 전부 clockedOut) 켜지는 소등
//                                     오버레이(`useLightsOff`). 캔버스 위,
//                                     라벨/UI 아래이며 클릭은 통과시킨다.
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
//                                     ConfirmRestartDialog/ConfirmClockOutDialog/
//                                     ConfirmQuitDialog/SettingsDialog/
//                                     FirstRunDialog/AnalyticsDialog/
//                                     UsageDialog/AboutDialog, all always
//                                     mounted, each self-gated (`null`
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
  // 에이전트가 하나 이상 있으나 전원 퇴근했을 때만 true(빈 새 사무실은 제외).
  const lightsOff = useLightsOff();

  return (
    <div className="app-root">
      <OfficeCanvas
        bus={officeBus}
        profiles={officeProfiles}
        resyncSignal={spritePreviews}
        pixiPalette={pixiPalette}
      />
      {lightsOff && (
        <div className="office-lights-off" aria-hidden="true">
          <span className="office-lights-off-label">모두 퇴근했습니다</span>
        </div>
      )}
      <TaskLabelLayer bus={officeBus} />
      <UIChrome />
      <AgentHoverCard />
      <DeskAssignMenu />
      <TerminalOverlay />
      {/* 마크다운 문서 탐색·편집(이슈 #10). 항상 마운트, 각자 store 상태로
          self-gate(null 렌더). z-index로 터미널 오버레이 위에 뜬다(markdown.css).
          터미널 keep-alive와 무관 — 터미널 DOM은 건드리지 않는다. */}
      <MarkdownPalette />
      <MarkdownEditorOverlay />
      <div className="modal-root">
        <ProfileDialog />
        <ConfirmDeleteDialog />
        <ConfirmRestartDialog />
        <ConfirmResumeDialog />
        <ConfirmTerminateDialog />
        <ConfirmClockOutDialog />
        <ConfirmQuitDialog />
        <SettingsDialog />
        <FirstRunDialog />
        <AnalyticsDialog />
        <UsageDialog />
        <AboutDialog />
      </div>
    </div>
  );
}

export default App;
