// src/renderer/settings/SoundTab.tsx
//
// 설정 다이얼로그 "소리·음성" 탭 — 효과음/볼륨/알림 지연과 TTS 섹션.
import { useTranslation } from "react-i18next";
import { useAppStore } from "../store/appStore";
import { TtsSection } from "./TtsSection";

/** 소리·음성 — 효과음/볼륨/알림 지연 + 대사 읽어주기(TTS). */
export function SoundTab() {
  const { t } = useTranslation("settings");
  const appSettings = useAppStore((s) => s.appSettings);
  const updateAppSettings = useAppStore((s) => s.updateAppSettings);

  return (
    <>
      <div className="settings-form">
        <label className="settings-item">
          <input
            type="checkbox"
            checked={appSettings.typingSoundEnabled}
            onChange={(e) => updateAppSettings({ typingSoundEnabled: e.target.checked })}
          />
          <span>
            <strong>{t("sound.typingTitle")}</strong>
            <small>{t("sound.typingHelp")}</small>
          </span>
        </label>
        <label className="settings-item">
          <input
            type="checkbox"
            checked={appSettings.notifySoundEnabled}
            onChange={(e) => updateAppSettings({ notifySoundEnabled: e.target.checked })}
          />
          <span>
            <strong>{t("sound.notifyTitle")}</strong>
            <small>{t("sound.notifyHelp")}</small>
          </span>
        </label>
        <label className="settings-item">
          <span>
            <strong>{t("sound.volumeTitle")}</strong>
            <small>{t("sound.volumeHelp")}</small>
          </span>
          <input
            type="range"
            min={0}
            max={100}
            value={Math.round(appSettings.soundVolume * 100)}
            disabled={
              !appSettings.typingSoundEnabled &&
              !appSettings.notifySoundEnabled &&
              !appSettings.ttsEnabled
            }
            onChange={(e) => updateAppSettings({ soundVolume: Number(e.target.value) / 100 })}
          />
        </label>
        <label className="settings-item">
          <span>
            <strong>{t("sound.attentionHoldTitle")}</strong>
            <small>{t("sound.attentionHoldHelp")}</small>
          </span>
          <input
            type="number"
            min={0}
            max={60}
            value={Math.round(appSettings.attentionHoldMs / 1000)}
            onChange={(e) => {
              const secs = Math.max(0, Math.min(60, Math.round(Number(e.target.value) || 0)));
              updateAppSettings({ attentionHoldMs: secs * 1000 });
            }}
          />
        </label>
      </div>
      <TtsSection />
    </>
  );
}
