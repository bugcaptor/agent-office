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
      ? `세션 활동 분석 · 이 달의 우수사원 · 열린 대화 ${openTalkCount}건 · 전달 대기 ${talkQueued}건 (상대가 한가해지면 전달)`
      : `세션 활동 분석 · 이 달의 우수사원 · 열린 대화 ${openTalkCount}건`
    : "세션 활동 분석 · 이 달의 우수사원";

  return (
    <footer className="bottom-bar pixel-panel">
      <button
        type="button"
        className="pixel-btn primary new-agent-btn"
        onClick={() => openModal({ kind: "profile-create" })}
      >
        ＋ New Agent
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
        🏠 출근 ({clockedOutCount})
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
        전체 출퇴근
      </button>
      {clockAllMenu && (
        <ContextMenu
          x={clockAllMenu.x}
          y={clockAllMenu.y}
          onClose={() => setClockAllMenu(null)}
          items={[
            {
              icon: "🏠",
              label: `전체 출근 (터미널 켜기, ${clockedOutCount}명)`,
              disabled: clockedOutCount === 0,
              onSelect: () => clockInAll(),
            },
            {
              icon: "🌙",
              label: `전체 퇴근 (${onDutyCount}명)`,
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
        {runningCount} running · {pendingCount} needs input
      </span>
      <UsageWidget />
      <button
        type="button"
        className="pixel-btn records-btn"
        aria-label="기록"
        aria-haspopup="menu"
        title={recordTitle}
        onClick={(e) => {
          const rect = e.currentTarget.getBoundingClientRect();
          setRecordMenu({ x: rect.left, y: rect.top });
        }}
      >
        📊 기록{talkEnabled && openTalkCount > 0 ? ` ·${openTalkCount}` : ""}
      </button>
      {recordMenu && (
        <ContextMenu
          x={recordMenu.x}
          y={recordMenu.y}
          onClose={() => setRecordMenu(null)}
          items={[
            {
              icon: "📊",
              label: "세션 활동 분석",
              onSelect: () => openModal({ kind: "analytics" }),
            },
            {
              icon: "🏆",
              label: "이 달의 우수사원",
              onSelect: () => openModal({ kind: "awards" }),
            },
            // talkEnabled가 꺼져 있으면 대화 항목 2개(+ 그 앞 구분선)를 아예
            // 넣지 않는다 — 예전 TalkWidget이 null을 렌더하던 것과 같은 의미.
            ...(talkEnabled
              ? [
                  { separator: true as const },
                  {
                    icon: "💬",
                    label: `대화 로그 보기 (${openTalkCount})`,
                    onSelect: () => openModal({ kind: "talk-log" }),
                  },
                  {
                    icon: "⛔",
                    label: "대화 전체 중지",
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
        aria-label="설정"
        title="설정 (선택적 에이전트 연동)"
        onClick={() => openModal({ kind: "settings" })}
      >
        ⚙
      </button>
      <button
        type="button"
        className="pixel-btn about-btn"
        aria-label="정보"
        title="Agent Office 정보"
        onClick={() => openModal({ kind: "about" })}
      >
        ℹ
      </button>
      <button
        type="button"
        className="pixel-btn scene-theme-btn"
        aria-label="풍경·테마 선택"
        aria-haspopup="menu"
        title="오피스 풍경·테마 목록에서 고르기"
        onClick={(e) => {
          const rect = e.currentTarget.getBoundingClientRect();
          setSceneThemeMenu({ x: rect.left, y: rect.top });
        }}
      >
        🎨 {SCENES[scene].label} · {THEMES[theme].label}
      </button>
      {sceneThemeMenu && (
        <ContextMenu
          x={sceneThemeMenu.x}
          y={sceneThemeMenu.y}
          onClose={() => setSceneThemeMenu(null)}
          items={[
            { header: "풍경" },
            ...SCENE_ORDER.map((id) => ({
              // 현재 항목만 체크 아이콘.
              icon: id === scene ? "✔" : undefined,
              label: SCENES[id].label,
              onSelect: () => setScene(id),
            })),
            { header: "테마" },
            ...THEME_ORDER.map((id) => ({
              // 현재 테마는 체크로 표시(아이콘 슬롯 폭은 나머지 항목도 유지한다).
              icon: id === theme ? "✔" : undefined,
              label: THEMES[id].label,
              onSelect: () => setTheme(id),
            })),
          ]}
        />
      )}
      <button
        type="button"
        className="pixel-btn mute-btn"
        aria-pressed={muted}
        aria-label={muted ? "Unmute notifications" : "Mute notifications"}
        onClick={toggleMuted}
      >
        {muted ? "🔇" : "🔔"}
      </button>
    </footer>
  );
}
