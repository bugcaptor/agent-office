// src/renderer/settings/GeneralTab.tsx
//
// 설정 다이얼로그 "일반" 탭 — UI 언어, FirstRunDialog와 공유하는 연동 폼,
// 요약 모델 오버라이드.
import { useTranslation } from "react-i18next";
import { useAppStore } from "../store/appStore";
import { LANGUAGE_SYSTEM, applyLanguageSetting, availableLanguages } from "../i18n";
import { SettingsForm } from "./SettingsForm";
import { SummaryModelSection } from "./SummarySection";

/** 일반 — 요약 라벨·요약기·일기·관찰. FirstRunDialog와 공유하는 폼 그대로. */
export function GeneralTab() {
  const appSettings = useAppStore((s) => s.appSettings);
  const updateAppSettings = useAppStore((s) => s.updateAppSettings);

  return (
    <>
      <LanguageItem />
      <SettingsForm
        value={{
          summarizerEnabled: appSettings.summarizerEnabled,
          summaryProvider: appSettings.summaryProvider,
          diaryEnabled: appSettings.diaryEnabled,
          observerEnabled: appSettings.observerEnabled,
        }}
        onChange={updateAppSettings}
      />
      <SummaryModelSection />
    </>
  );
}

/**
 * UI 언어 선택. 항목은 번역 카탈로그에 실제로 있는 언어에서 도출되므로
 * (`availableLanguages()`), 언어를 추가할 때 이 컴포넌트는 고치지 않는다.
 *
 * 저장(`updateAppSettings`)과 적용(`applyLanguageSetting`)을 **둘 다** 부른다:
 * 저장은 다음 부팅용이고, 적용은 지금 화면을 즉시 바꾸기 위한 것이다(재시작
 * 불필요). 부팅 경로에서도 같은 함수를 부르므로 두 경로의 해석 규칙이 하나다.
 */
function LanguageItem() {
  const { t } = useTranslation("settings");
  const language = useAppStore((s) => s.appSettings.language);
  const updateAppSettings = useAppStore((s) => s.updateAppSettings);

  return (
    <label className="settings-item">
      <span>
        <strong>{t("language.title")}</strong>
        <small>{t("language.help")}</small>
      </span>
      <select
        value={language}
        onChange={(e) => {
          const next = e.target.value;
          updateAppSettings({ language: next });
          applyLanguageSetting(next);
        }}
      >
        <option value={LANGUAGE_SYSTEM}>{t("language.system")}</option>
        {availableLanguages().map((l) => (
          <option key={l.code} value={l.code}>
            {l.label}
          </option>
        ))}
      </select>
    </label>
  );
}
