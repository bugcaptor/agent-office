// src/renderer/layout/BottomBar.tsx
//
// Bottom bar (좌→우): "＋ New Agent"(ProfileDialog 생성 모드) · "🏠 출근"
// (퇴근한 에이전트 수 배지, 클릭 시 이름별 ContextMenu → clockInAgent) ·
// "전체 출퇴근"(ContextMenu 두 항목: "전체 출근"은 clockInAll을 즉시
// 호출 — 출근은 비파괴적이라 확인 없음, "전체 퇴근"은
// confirm-clock-out-all 모달을 연다 → clockOutAll; 각 항목은 대상 집합이
// 비면 disabled, 버튼 자체는 에이전트가 하나도 없을 때만 disabled) · 가운데
// 가동/대기 상태 요약 텍스트 · `UsageWidget` · "📊 기록"(분석/우수사원/
// 동료 대화를 묶은 ContextMenu — 세션 활동 분석(AnalyticsDialog), 이 달의
// 우수사원(AwardsDialog), 그리고 talkEnabled가 켜져 있을 때만 구분선 +
// 대화 로그 보기(talk-log 모달) + 대화 전체 중지(danger, talkEnabled:false
// 저장 — docs/agent-talk-design.md §5 킬스위치, 폴링도 이걸로 멈춘다).
// 열린 대화가 있으면 버튼 라벨에 " ·N" 배지. 폴링은 `talk/useTalkStatus.ts`
// 훅이 맡는다) · "⚙"(SettingsDialog, 선택적 에이전트 연동 2종) ·
// "ℹ"(AboutDialog, 앱 이름/버전/라이선스) · "🎨 {풍경}·{테마}"(현재 풍경·
// 테마 라벨을 그대로 보여주는 ContextMenu — "풍경"/"테마" 섹션 헤더로
// 나뉘고, 각 섹션에서 현재 선택 항목에 ✔) · 오른쪽 끝 mute 토글
// (store.muted를 뒤집는다; 토글 시 배지 리싱크는 여기가 아니라
// ipc/sessionBridge.ts의 installSessionBridge가 담당).
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { useAppStore } from "../store/appStore";
import {
  useAgentList,
  useClockedOutAgents,
  useClockedOutCount,
  usePendingCount,
  useRunningCount,
} from "../store/selectors";
import { THEMES, THEME_ORDER } from "../theme/themes";
import { SCENES, SCENE_ORDER } from "../office/scenes/scenes";
import { ContextMenu } from "../ui/ContextMenu";
import { clockInAgent, clockInAll } from "../agent/clockOut";
import { UsageWidget } from "../usage/UsageWidget";
import { useTalkStatus } from "../talk/useTalkStatus";

export function BottomBar() {
  const { t } = useTranslation("common");
  const openModal = useAppStore((s) => s.openModal);
  const muted = useAppStore((s) => s.muted);
  const toggleMuted = useAppStore((s) => s.toggleMuted);
  const theme = useAppStore((s) => s.theme);
  const setTheme = useAppStore((s) => s.setTheme);
  const scene = useAppStore((s) => s.scene);
  const setScene = useAppStore((s) => s.setScene);
  const talkEnabled = useAppStore((s) => s.appSettings.talkEnabled);
  const updateAppSettings = useAppStore((s) => s.updateAppSettings);
  const runningCount = useRunningCount();
  const pendingCount = usePendingCount();
  const onDutyCount = useAgentList().length;
  const clockedOutAgents = useClockedOutAgents();
  const clockedOutCount = useClockedOutCount();
  const { open: openTalkCount, queued: talkQueued } = useTalkStatus(talkEnabled);
  const [summonMenu, setSummonMenu] = useState<{ x: number; y: number } | null>(null);
  const [sceneThemeMenu, setSceneThemeMenu] = useState<{ x: number; y: number } | null>(null);
  const [clockAllMenu, setClockAllMenu] = useState<{ x: number; y: number } | null>(null);
  const [recordMenu, setRecordMenu] = useState<{ x: number; y: number } | null>(null);

  // "📊 기록" 버튼 툴팁 — 원래 TalkWidget이 title에 담던 열린/대기 대화
  // 문구를 여기로 옮긴다(talkEnabled가 꺼져 있으면 대화 정보는 뺀다).
  const recordTitle = talkEnabled
    ? talkQueued > 0
      ? t("bottomBar.recordsTitleQueued", { open: openTalkCount, queued: talkQueued })
      : t("bottomBar.recordsTitleTalk", { open: openTalkCount })
    : t("bottomBar.recordsTitle");

  return (
    <footer className="bottom-bar pixel-panel">
      <button
        type="button"
        className="pixel-btn primary new-agent-btn"
        onClick={() => openModal({ kind: "profile-create" })}
      >
        {t("bottomBar.newAgent")}
      </button>
      <button
        type="button"
        className="pixel-btn summon-btn"
        disabled={clockedOutCount === 0}
        onClick={(e) => {
          const rect = e.currentTarget.getBoundingClientRect();
          setSummonMenu({ x: rect.left, y: rect.top });
        }}
      >
        {t("bottomBar.clockIn", { n: clockedOutCount })}
      </button>
      <button
        type="button"
        className="pixel-btn clock-all-btn"
        aria-haspopup="menu"
        disabled={onDutyCount === 0 && clockedOutCount === 0}
        onClick={(e) => {
          const rect = e.currentTarget.getBoundingClientRect();
          setClockAllMenu({ x: rect.left, y: rect.top });
        }}
      >
        {t("bottomBar.clockAll")}
      </button>
      {clockAllMenu && (
        <ContextMenu
          x={clockAllMenu.x}
          y={clockAllMenu.y}
          onClose={() => setClockAllMenu(null)}
          items={[
            {
              icon: "🏠",
              label: t("bottomBar.clockInAll", { n: clockedOutCount }),
              disabled: clockedOutCount === 0,
              onSelect: () => clockInAll(),
            },
            {
              icon: "🌙",
              label: t("bottomBar.clockOutAll", { n: onDutyCount }),
              danger: true,
              disabled: onDutyCount === 0,
              onSelect: () => openModal({ kind: "confirm-clock-out-all" }),
            },
          ]}
        />
      )}
      {summonMenu && (
        <ContextMenu
          x={summonMenu.x}
          y={summonMenu.y}
          onClose={() => setSummonMenu(null)}
          items={clockedOutAgents.map((agent) => ({
            label: agent.name,
            onSelect: () => clockInAgent(agent.id),
          }))}
        />
      )}
      <span className="bottom-bar-status">
        {t("bottomBar.status", { running: runningCount, pending: pendingCount })}
      </span>
      <UsageWidget />
      <button
        type="button"
        className="pixel-btn records-btn"
        aria-label={t("bottomBar.recordsAria")}
        aria-haspopup="menu"
        title={recordTitle}
        onClick={(e) => {
          const rect = e.currentTarget.getBoundingClientRect();
          setRecordMenu({ x: rect.left, y: rect.top });
        }}
      >
        {t("bottomBar.records")}
        {talkEnabled && openTalkCount > 0 ? ` ·${openTalkCount}` : ""}
      </button>
      {recordMenu && (
        <ContextMenu
          x={recordMenu.x}
          y={recordMenu.y}
          onClose={() => setRecordMenu(null)}
          items={[
            {
              icon: "📊",
              label: t("bottomBar.analytics"),
              onSelect: () => openModal({ kind: "analytics" }),
            },
            {
              icon: "🏆",
              label: t("bottomBar.awards"),
              onSelect: () => openModal({ kind: "awards" }),
            },
            // talkEnabled가 꺼져 있으면 대화 항목 2개(+ 그 앞 구분선)를 아예
            // 넣지 않는다 — 예전 TalkWidget이 null을 렌더하던 것과 같은 의미.
            ...(talkEnabled
              ? [
                  { separator: true as const },
                  {
                    icon: "💬",
                    label: t("bottomBar.talkLog", { n: openTalkCount }),
                    onSelect: () => openModal({ kind: "talk-log" }),
                  },
                  {
                    icon: "⛔",
                    label: t("bottomBar.talkStop"),
                    danger: true,
                    onSelect: () => updateAppSettings({ talkEnabled: false }),
                  },
                ]
              : []),
          ]}
        />
      )}
      <button
        type="button"
        className="pixel-btn settings-btn"
        aria-label={t("bottomBar.settingsAria")}
        title={t("bottomBar.settingsTitle")}
        onClick={() => openModal({ kind: "settings" })}
      >
        ⚙
      </button>
      <button
        type="button"
        className="pixel-btn about-btn"
        aria-label={t("bottomBar.aboutAria")}
        title={t("bottomBar.aboutTitle")}
        onClick={() => openModal({ kind: "about" })}
      >
        ℹ
      </button>
      <button
        type="button"
        className="pixel-btn scene-theme-btn"
        aria-label={t("bottomBar.sceneThemeAria")}
        aria-haspopup="menu"
        title={t("bottomBar.sceneThemeTitle")}
        onClick={(e) => {
          const rect = e.currentTarget.getBoundingClientRect();
          setSceneThemeMenu({ x: rect.left, y: rect.top });
        }}
      >
        🎨 {t(SCENES[scene].labelKey)} · {t(THEMES[theme].labelKey)}
      </button>
      {sceneThemeMenu && (
        <ContextMenu
          x={sceneThemeMenu.x}
          y={sceneThemeMenu.y}
          onClose={() => setSceneThemeMenu(null)}
          items={[
            { header: t("bottomBar.sceneHeader") },
            ...SCENE_ORDER.map((id) => ({
              // 현재 항목만 체크 아이콘.
              icon: id === scene ? "✔" : undefined,
              label: t(SCENES[id].labelKey),
              onSelect: () => setScene(id),
            })),
            { header: t("bottomBar.themeHeader") },
            ...THEME_ORDER.map((id) => ({
              // 현재 테마는 체크로 표시(아이콘 슬롯 폭은 나머지 항목도 유지한다).
              icon: id === theme ? "✔" : undefined,
              label: t(THEMES[id].labelKey),
              onSelect: () => setTheme(id),
            })),
          ]}
        />
      )}
      <button
        type="button"
        className="pixel-btn mute-btn"
        aria-pressed={muted}
        aria-label={muted ? t("bottomBar.unmuteAria") : t("bottomBar.muteAria")}
        onClick={toggleMuted}
      >
        {muted ? "🔇" : "🔔"}
      </button>
    </footer>
  );
}
