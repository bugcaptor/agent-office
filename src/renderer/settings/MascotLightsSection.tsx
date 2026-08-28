// src/renderer/settings/MascotLightsSection.tsx
//
// 마스코트 신호등(docs/mascot-lights-design.md) 설정 UI. 모드(끄기/에이전트별/
// 프로젝트별) 선택, 세로 배열 체크박스, 프로젝트 모드일 때만 보이는 폴더 목록
// 편집기(추가/제거)로 구성된다.
//
// `mascotEnabled`가 신호등의 상위 게이트다(설계 §2 결정 6) — 마스코트 자체가
// 꺼져 있으면 셋 다 비활성화한다(값은 유지, 조작만 막는다).

import { useTranslation } from "react-i18next";
import { useAppStore } from "../store/appStore";
import { tauriApi } from "../ipc/tauriApi";
import type { MascotLightsMode } from "@shared/types";

export function MascotLightsSection() {
  const { t } = useTranslation("settings");
  const appSettings = useAppStore((s) => s.appSettings);
  const updateAppSettings = useAppStore((s) => s.updateAppSettings);
  const disabled = !appSettings.mascotEnabled;
  const projects = appSettings.mascotLightsProjects;

  const onAddProject = async () => {
    try {
      const picked = await tauriApi.pickDirectory(undefined);
      if (picked && !projects.includes(picked)) {
        updateAppSettings({ mascotLightsProjects: [...projects, picked] });
      }
    } catch (err) {
      console.warn("MascotLightsSection: pickDirectory failed", err);
    }
  };

  const onRemoveProject = (index: number) => {
    updateAppSettings({ mascotLightsProjects: projects.filter((_, i) => i !== index) });
  };

  return (
    <div className="settings-form">
      <label className="settings-item">
        <span>
          <strong>{t("system.mascotLightsTitle")}</strong>
          <small>{t("system.mascotLightsHelp")}</small>
        </span>
        <select
          disabled={disabled}
          value={appSettings.mascotLightsMode}
          onChange={(e) =>
            updateAppSettings({ mascotLightsMode: e.target.value as MascotLightsMode })
          }
        >
          <option value="off">{t("system.mascotLightsModeOff")}</option>
          <option value="agents">{t("system.mascotLightsModeAgents")}</option>
          <option value="projects">{t("system.mascotLightsModeProjects")}</option>
        </select>
      </label>

      <label className="settings-item">
        <input
          type="checkbox"
          disabled={disabled}
          checked={appSettings.mascotLightsVertical}
          onChange={(e) => updateAppSettings({ mascotLightsVertical: e.target.checked })}
        />
        <span>
          <strong>{t("system.mascotLightsVerticalTitle")}</strong>
        </span>
      </label>

      {appSettings.mascotLightsMode === "projects" && (
        <div className="settings-item-stacked mascot-lights-projects">
          <span>
            <strong>{t("system.mascotLightsProjectsTitle")}</strong>
          </span>
          {projects.length === 0 ? (
            <p className="settings-note">{t("system.mascotLightsProjectsEmpty")}</p>
          ) : (
            <ul className="mascot-lights-project-list">
              {projects.map((project, index) => (
                <li key={project} className="mascot-lights-project-row">
                  <span className="mascot-lights-project-path">{project}</span>
                  <button
                    type="button"
                    className="pixel-btn"
                    disabled={disabled}
                    onClick={() => onRemoveProject(index)}
                  >
                    {t("system.mascotLightsProjectsRemove")}
                  </button>
                </li>
              ))}
            </ul>
          )}
          <button
            type="button"
            className="pixel-btn"
            disabled={disabled}
            onClick={() => void onAddProject()}
          >
            {t("system.mascotLightsProjectsAdd")}
          </button>
        </div>
      )}
    </div>
  );
}
