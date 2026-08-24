// src/renderer/settings/SettingsForm.tsx
//
// 선택적 에이전트 연동 설정 — FirstRunDialog(첫 실행 동의)와
// SettingsDialog(상시 변경)가 공유한다. 폼은 상태를 소유하지 않는다:
// value/onChange 순수 제어 컴포넌트.
import { useTranslation } from "react-i18next";
import type { AppSettings } from "@shared/types";

export type SettingsFormValue = Pick<
  AppSettings,
  "summarizerEnabled" | "summaryProvider" | "diaryEnabled" | "observerEnabled"
>;

export function SettingsForm({
  value,
  onChange,
}: {
  value: SettingsFormValue;
  onChange: (patch: Partial<SettingsFormValue>) => void;
}) {
  const { t } = useTranslation("settings");

  return (
    <div className="settings-form">
      <label className="settings-item">
        <input
          type="checkbox"
          checked={value.summarizerEnabled}
          onChange={(e) => onChange({ summarizerEnabled: e.target.checked })}
        />
        <span>
          <strong>{t("general.summarizerTitle")}</strong>
          <small>{t("general.summarizerHelp")}</small>
        </span>
      </label>

      <fieldset aria-label={t("general.providerLegend")}>
        <legend>{t("general.providerLegend")}</legend>
        <label>
          <input
            type="radio"
            name="summary-provider"
            checked={value.summaryProvider === "claude"}
            onChange={() => onChange({ summaryProvider: "claude" })}
          />
          Claude
        </label>
        <label>
          <input
            type="radio"
            name="summary-provider"
            checked={value.summaryProvider === "codex"}
            onChange={() => onChange({ summaryProvider: "codex" })}
          />
          Codex
        </label>
        <label>
          <input
            type="radio"
            name="summary-provider"
            checked={value.summaryProvider === "agy"}
            onChange={() => onChange({ summaryProvider: "agy" })}
          />
          Antigravity (agy)
        </label>
        <label>
          <input
            type="radio"
            name="summary-provider"
            checked={value.summaryProvider === "gemini"}
            onChange={() => onChange({ summaryProvider: "gemini" })}
          />
          Gemini
        </label>
        <label>
          <input
            type="radio"
            name="summary-provider"
            checked={value.summaryProvider === "opencode"}
            onChange={() => onChange({ summaryProvider: "opencode" })}
          />
          opencode
        </label>
        <label>
          <input
            type="radio"
            name="summary-provider"
            checked={value.summaryProvider === "openrouter"}
            onChange={() => onChange({ summaryProvider: "openrouter" })}
          />
          {t("general.providerOpenrouter")}
        </label>
      </fieldset>

      <label className="settings-item">
        <input
          type="checkbox"
          checked={value.diaryEnabled}
          onChange={(e) => onChange({ diaryEnabled: e.target.checked })}
        />
        <span>
          <strong>{t("general.diaryTitle")}</strong>
          <small>{t("general.diaryHelp")}</small>
        </span>
      </label>

      <label className="settings-item">
        <input
          type="checkbox"
          checked={value.observerEnabled}
          onChange={(e) => onChange({ observerEnabled: e.target.checked })}
        />
        <span>
          <strong>{t("general.observerTitle")}</strong>
          <small>{t("general.observerHelp")}</small>
        </span>
      </label>
    </div>
  );
}
