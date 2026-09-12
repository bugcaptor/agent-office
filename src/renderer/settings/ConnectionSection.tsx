// Optional entry points that connect this app to software outside Agent Office.
// They deliberately live apart from WebRemoteSection: that switch publishes
// this office to a browser, while these switches allow this desktop app to
// initiate connections.
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
    <label className="settings-item">
      <input
        type="checkbox"
        checked={settings.remoteServerConnectionEnabled}
        onChange={(event) => updateAppSettings({ remoteServerConnectionEnabled: event.target.checked })}
      />
      <span>
        <strong>{t("connections.remoteTitle")}</strong>
        <small>{t("connections.remoteHelp")}</small>
      </span>
    </label>
    <p className="settings-item" style={{ margin: 0, fontSize: 12, opacity: 0.75 }}>
      {t("connections.webRemoteNote")}
    </p>
  </div>;
}
