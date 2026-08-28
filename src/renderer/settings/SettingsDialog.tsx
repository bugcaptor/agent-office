// src/renderer/settings/SettingsDialog.tsx
//
// 상시 설정 다이얼로그(BottomBar ⚙로 열림). FirstRunDialog와 달리 스토어
// 값을 직접 바인딩 — 토글 즉시 updateAppSettings로 저장된다(확인 버튼 없음).
//
// 항목이 불어나 한 화면 스크롤로는 못 찾겠어서 탭 4개(일반/소리·음성/
// 시스템/제어)로 나눴다. 탭은 다이얼로그 로컬 상태이고 기억하지 않는다 —
// 열 때마다 첫 탭. 그래서 게이팅(SettingsDialog)과 본체(SettingsDialogBody)를
// 나눠, 닫으면 본체가 언마운트되며 탭 상태가 함께 사라지게 한다(이 컴포넌트는
// App에 상시 마운트돼 있어 useState만으로는 초기화되지 않는다).
//
// 탭 본체는 각각 별도 파일이다(GeneralTab / SoundTab / SystemTab /
// ControlSection) — 이 파일은 게이팅과 탭 전환만 맡는다.
import { useState } from "react";
import { useTranslation } from "react-i18next";
import { useAppStore } from "../store/appStore";
import { GeneralTab } from "./GeneralTab";
import { SoundTab } from "./SoundTab";
import { SystemTab } from "./SystemTab";
import { ControlSection } from "./ControlSection";
import { WebRemoteSection } from "./WebRemoteSection";
import { TalkSection } from "./TalkSection";

type SettingsTabId = "general" | "sound" | "system" | "control";

/** 화면 순서 = 이 배열 순서. 첫 항목이 열 때마다의 기본 탭이다.
 *  모듈 최상위라 `t()`를 부를 수 없어 라벨이 아니라 **키**를 담는다 —
 *  언어를 바꾸면 렌더 시점에 다시 번역된다. */
const SETTINGS_TABS: { id: SettingsTabId; labelKey: string }[] = [
  { id: "general", labelKey: "dialog.tabGeneral" },
  { id: "sound", labelKey: "dialog.tabSound" },
  { id: "system", labelKey: "dialog.tabSystem" },
  { id: "control", labelKey: "dialog.tabControl" },
];

export function SettingsDialog() {
  const modal = useAppStore((s) => s.modal);
  if (modal.kind !== "settings") return null;
  return <SettingsDialogBody />;
}

function SettingsDialogBody() {
  const { t } = useTranslation("settings");
  const closeModal = useAppStore((s) => s.closeModal);
  const cliEnabled = useAppStore((s) => s.appSettings.cliEnabled);
  const [tab, setTab] = useState<SettingsTabId>(SETTINGS_TABS[0].id);

  return (
    <div
      className="modal-backdrop"
      onMouseDown={(e) => {
        if (e.button === 0 && e.target === e.currentTarget) closeModal();
      }}
    >
      <div className="pixel-panel settings-dialog">
        <h2 className="pixel-title">{t("dialog.title")}</h2>

        <div className="settings-tabs" role="tablist" aria-label={t("dialog.tabsAria")}>
          {SETTINGS_TABS.map((item) => (
            <button
              key={item.id}
              type="button"
              role="tab"
              id={`settings-tab-${item.id}`}
              aria-selected={tab === item.id}
              aria-controls={`settings-tabpanel-${item.id}`}
              className={tab === item.id ? "settings-tab settings-tab-active" : "settings-tab"}
              onClick={() => setTab(item.id)}
            >
              {t(item.labelKey)}
            </button>
          ))}
        </div>

        <div
          className="settings-tabpanel"
          role="tabpanel"
          id={`settings-tabpanel-${tab}`}
          aria-labelledby={`settings-tab-${tab}`}
          tabIndex={0}
        >
          {tab === "general" && <GeneralTab />}
          {tab === "sound" && <SoundTab />}
          {tab === "system" && <SystemTab />}
          {tab === "control" && (
            <>
              <ControlSection enabled={cliEnabled} />
              <WebRemoteSection />
              <TalkSection />
            </>
          )}
        </div>

        <div className="dialog-actions">
          <button className="pixel-btn" onClick={closeModal}>
            {t("dialog.close")}
          </button>
        </div>
      </div>
    </div>
  );
}
