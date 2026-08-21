import "./layout/layout.css";

import { useEffect, useMemo } from "react";
import { officeBus } from "./ipc/sessionBridge";
import { OfficeCanvas } from "./office/OfficeCanvas";
import type { AgentProfile as OfficeAgentProfile } from "./office/types";
import { AgentHoverCard } from "./portrait/AgentHoverCard";
import { DeskAssignMenu } from "./desk/DeskAssignMenu";
import { ProfileDialog } from "./profile/ProfileDialog";
import {
  ConfirmDeleteDialog,
  ConfirmRestartDialog,
  ConfirmResumeDialog,
  ConfirmTerminateDialog,
  ConfirmBotStartDialog,
  ConfirmClockOutDialog,
} from "./agent/confirmDialogs";
import { ConfirmQuitDialog } from "./agent/ConfirmQuitDialog";
import { SettingsDialog } from "./settings/SettingsDialog";
import { FirstRunDialog } from "./settings/FirstRunDialog";
import { PairingRequestDialog } from "./settings/PairingRequestDialog";
import { AnalyticsDialog } from "./analytics/AnalyticsDialog";
import { UsageDialog } from "./usage/UsageDialog";
import { AboutDialog } from "./about/AboutDialog";
import { useAppStore } from "./store/appStore";
import { useAgentList, useLightsOff } from "./store/selectors";
import { THEMES } from "./theme/themes";
import { SCENES } from "./office/scenes/scenes";
import { applyTerminalBg } from "./theme/applyTheme";
import { resolveXtermTheme } from "./terminal/theme";
import { terminalRegistry } from "./terminal/TerminalRegistry";
import { TaskLabelLayer } from "./labels/TaskLabelLayer";
import { TerminalOverlay } from "./terminal/TerminalOverlay";
import { MarkdownPalette } from "./markdown/MarkdownPalette";
import { MarkdownEditorOverlay } from "./markdown/MarkdownEditorOverlay";
import { WorkdirPalette } from "./workdir/WorkdirPalette";
import { DiaryDialog } from "./diary/DiaryDialog";
import { SessionLogDialog } from "./sessionlog/SessionLogDialog";
import { MemoArchiveDialog } from "./memo/MemoArchiveDialog";
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
  // 미니미 커스텀 픽셀아트도 같은 이유로 재동기화 신호에 포함한다. 두 맵을
  // useMemo로 한 객체에 묶어, 둘 중 하나가 실제로 바뀔 때만 새 참조가 되게 한다
  // (매 렌더 새 객체 리터럴을 넘기면 매번 diff-sync가 돈다).
  const minimiPreviews = useAppStore((s) => s.minimiPreviews);
  const appearanceSignal = useMemo(
    () => ({ spritePreviews, minimiPreviews }),
    [spritePreviews, minimiPreviews],
  );
  // 테마 -> Pixi 씬. `THEMES[..]`는 모듈 상수라 참조가 안정적 — 테마가 실제로
  // 바뀔 때만 B의 setTheme 효과가 발화한다. DOM 쪽 토큰은
  // `applyTheme`(store.setTheme / main.tsx 부트)이 이미 처리한다.
  const themeId = useAppStore((s) => s.theme);
  const theme = THEMES[themeId];
  // 풍경(scene)은 테마와 직교한 축이다 — 같은 이유로 레지스트리 상수를 그대로
  // 넘겨 참조 동일성이 바뀔 때만 B의 setScene 효과가 발화하게 한다.
  const sceneId = useAppStore((s) => s.scene);
  const scene = SCENES[sceneId];
  // 테마 -> 터미널(xterm) 팔레트. Pixi 배선과 같은 모양이되, 소비처가 React
  // 트리 밖(keep-alive 레지스트리)이라 효과로 밀어 넣는다. `xtermTheme`이
  // "auto"가 아니면 앱 테마와 무관하게 그 테마로 고정된다.
  const xtermOverride = useAppStore((s) => s.xtermTheme);
  useEffect(() => {
    terminalRegistry.setTheme(resolveXtermTheme(themeId, xtermOverride));
    applyTerminalBg(themeId, xtermOverride);
  }, [themeId, xtermOverride]);
  // 에이전트가 하나 이상 있으나 전원 퇴근했을 때만 true(빈 새 사무실은 제외).
  const lightsOff = useLightsOff();

  return (
    <div className="app-root">
      <OfficeCanvas
        bus={officeBus}
        profiles={officeProfiles}
        resyncSignal={appearanceSignal}
        theme={theme}
        scene={scene}
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
      {/* 작업 폴더 보기(이슈 #11). markdown 오버레이와 같은 층위·관례로 상시 마운트. */}
      <WorkdirPalette />
      {/* 캐릭터 일기(이슈 #56). 같은 층위·관례로 상시 마운트, store로 self-gate. */}
      <DiaryDialog />
      {/* 세션 로그 보기(docs/session-log-design.md). 같은 층위·관례로 상시 마운트. */}
      <SessionLogDialog />
      {/* 포스트잇 메모 아카이브(이슈 #79). 위젯 자체는 터미널 오버레이 패널 안에
          있고, 아카이브 열람만 이 층에 뜬다(터미널을 덮어야 하므로). */}
      <MemoArchiveDialog />
      {/* pipboy 전용 CRT 연출(스캔라인 + 옅은 비네트). 순수 CSS 1장, 클릭
          통과, 최상단. 다른 테마에서는 아예 마운트되지 않는다. */}
      {themeId === "pipboy" && <div className="crt-overlay" aria-hidden="true" />}
      <div className="modal-root">
        <ProfileDialog />
        <ConfirmDeleteDialog />
        <ConfirmRestartDialog />
        <ConfirmResumeDialog />
        <ConfirmTerminateDialog />
        <ConfirmBotStartDialog />
        <ConfirmClockOutDialog />
        <ConfirmQuitDialog />
        <SettingsDialog />
        <FirstRunDialog />
        {/* 페어링 승인(#7k/#7m). 설정을 열지 않아도 코드가 보여야 하므로
            여기 모달 층에 상시 마운트하고 webRemotePending으로 self-gate한다. */}
        <PairingRequestDialog />
        <AnalyticsDialog />
        <UsageDialog />
        <AboutDialog />
      </div>
    </div>
  );
}

export default App;
