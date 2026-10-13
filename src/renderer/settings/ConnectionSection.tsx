// Optional entry points that let this desktop app initiate connections to
// software outside Agent Office.
import { useTranslation } from "react-i18next";
import { useAppStore } from "../store/appStore";

export function ConnectionSection() {
  const { t } = useTranslation("settings");
  const settings = useAppStore((s) => s.appSettings);
  const updateAppSettings = useAppStore((s) => s.updateAppSettings);

  return <div className="settings-form">
    <label className="settings-item">
      <input
        type="checkbox"
        checked={settings.ideConnectionEnabled}
        onChange={(event) => updateAppSettings({ ideConnectionEnabled: event.target.checked })}
      />
      <span>
        <strong>{t("connections.ideTitle")}</strong>
        <small>{t("connections.ideHelp")}</small>
      </span>
    </label>
  </div>;
}
