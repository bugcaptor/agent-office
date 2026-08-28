// src/renderer/settings/SystemTab.tsx
//
// 설정 다이얼로그 "시스템" 탭 — 앱 바깥(OS·저장소·외부 앱)에 닿는 설정과
// 터미널 색상.
import { useTranslation } from "react-i18next";
import { useAppStore } from "../store/appStore";
import { MascotLightsSection } from "./MascotLightsSection";
import { THEMES, THEME_ORDER } from "../theme/themes";
import { XTERM_PALETTES, XTERM_PALETTE_ORDER } from "../terminal/palettes";
import type { XtermThemeOverride } from "../terminal/theme";
import type {
  ExternalEditorApp,
  ExternalTerminalApp,
  FileIndexBackend,
} from "@shared/types";

/** 시스템 — 앱 바깥(OS·저장소·외부 앱)에 닿는 설정과 터미널 색상. */
export function SystemTab() {
  const { t } = useTranslation("settings");
  const appSettings = useAppStore((s) => s.appSettings);
  const updateAppSettings = useAppStore((s) => s.updateAppSettings);

  return (
    <div className="settings-form">
      <label className="settings-item">
        <input
          type="checkbox"
          checked={appSettings.gitStatusEnabled}
          onChange={(e) => updateAppSettings({ gitStatusEnabled: e.target.checked })}
        />
        <span>
          <strong>{t("system.gitStatusTitle")}</strong>
          <small>{t("system.gitStatusHelp")}</small>
        </span>
      </label>
      <label className="settings-item">
        <input
          type="checkbox"
          checked={appSettings.workdirShowIgnored}
          onChange={(e) => updateAppSettings({ workdirShowIgnored: e.target.checked })}
        />
        <span>
          <strong>{t("system.workdirShowIgnoredTitle")}</strong>
          <small>{t("system.workdirShowIgnoredHelp")}</small>
        </span>
      </label>
      <label className="settings-item">
        <input
          type="checkbox"
          checked={appSettings.keepAwakeEnabled}
          onChange={(e) => updateAppSettings({ keepAwakeEnabled: e.target.checked })}
        />
        <span>
          <strong>{t("system.keepAwakeTitle")}</strong>
          <small>{t("system.keepAwakeHelp")}</small>
        </span>
      </label>
      <label className="settings-item">
        <input
          type="checkbox"
          checked={appSettings.sessionLogEnabled}
          onChange={(e) => updateAppSettings({ sessionLogEnabled: e.target.checked })}
        />
        <span>
          <strong>{t("system.sessionLogTitle")}</strong>
          <small>{t("system.sessionLogHelp")}</small>
        </span>
      </label>
      <label className="settings-item">
        <input
          type="checkbox"
          checked={appSettings.mascotEnabled}
          onChange={(e) => updateAppSettings({ mascotEnabled: e.target.checked })}
        />
        <span>
          <strong>{t("system.mascotTitle")}</strong>
          <small>{t("system.mascotHelp")}</small>
        </span>
      </label>
      <MascotLightsSection />
      <label className="settings-item">
        <input
          type="checkbox"
          checked={appSettings.usageFloatEnabled}
          onChange={(e) => updateAppSettings({ usageFloatEnabled: e.target.checked })}
        />
        <span>
          <strong>{t("system.usageFloatTitle")}</strong>
          <small>{t("system.usageFloatHelp")}</small>
        </span>
      </label>
      <label className="settings-item">
        <input
          type="checkbox"
          checked={appSettings.sessionCostEnabled}
          onChange={(e) => updateAppSettings({ sessionCostEnabled: e.target.checked })}
        />
        <span>
          <strong>{t("system.sessionCostTitle")}</strong>
          <small>{t("system.sessionCostHelp")}</small>
        </span>
      </label>
      <label className="settings-item">
        <span>
          <strong>{t("system.externalTerminalTitle")}</strong>
          <small>{t("system.externalTerminalHelp")}</small>
        </span>
        <select
          value={appSettings.externalTerminal}
          onChange={(e) =>
            updateAppSettings({
              externalTerminal: e.target.value as ExternalTerminalApp,
            })
          }
        >
          <option value="terminal">{t("system.terminalDefault")}</option>
          <option value="iterm">iTerm2</option>
        </select>
      </label>
      <label className="settings-item">
        <span>
          <strong>{t("system.externalEditorTitle")}</strong>
          <small>{t("system.externalEditorHelp")}</small>
        </span>
        <select
          value={appSettings.externalEditor}
          onChange={(e) =>
            updateAppSettings({
              externalEditor: e.target.value as ExternalEditorApp,
            })
          }
        >
          <option value="system">{t("system.editorSystem")}</option>
          <option value="vscode">VS Code</option>
        </select>
      </label>
      <label className="settings-item">
        <span>
          <strong>{t("system.fileIndexTitle")}</strong>
          <small>{t("system.fileIndexHelp")}</small>
        </span>
        <select
          value={appSettings.fileIndexBackend}
          onChange={(e) =>
            updateAppSettings({
              fileIndexBackend: e.target.value as FileIndexBackend,
            })
          }
        >
          <option value="walker">{t("system.fileIndexWalker")}</option>
          <option value="everything">Everything (es.exe)</option>
        </select>
      </label>
      <TerminalThemeItem />
    </div>
  );
}

/**
 * 터미널 색상 선택. 다른 항목과 달리 AppSettings(Rust 영속)가 아니라
 * zustand + localStorage에 사는 값이라 `updateAppSettings`가 아닌 전용 액션에
 * 직접 바인딩한다(테마 자체와 같은 계층 — theme/applyTheme.ts 참고).
 */
function TerminalThemeItem() {
  const { t } = useTranslation("settings");
  const xtermTheme = useAppStore((s) => s.xtermTheme);
  const setXtermTheme = useAppStore((s) => s.setXtermTheme);

  return (
    <label className="settings-item">
      <span>
        <strong>{t("system.terminalThemeTitle")}</strong>
        <small>{t("system.terminalThemeHelp")}</small>
      </span>
      <select
        value={xtermTheme}
        onChange={(e) => setXtermTheme(e.target.value as XtermThemeOverride)}
      >
        <option value="auto">{t("system.terminalThemeAuto")}</option>
        <optgroup label={t("system.terminalThemeAppGroup")}>
          {THEME_ORDER.map((id) => (
            <option key={id} value={id}>
              {t(THEMES[id].labelKey)}
            </option>
          ))}
        </optgroup>
        {/* 앱 테마와 짝이 없는, 터미널에만 적용되는 팔레트(terminal/palettes.ts). */}
        <optgroup label={t("system.terminalThemeOwnGroup")}>
          {XTERM_PALETTE_ORDER.map((id) => (
            <option key={id} value={id}>
              {t(XTERM_PALETTES[id].labelKey)}
            </option>
          ))}
        </optgroup>
      </select>
    </label>
  );
}
