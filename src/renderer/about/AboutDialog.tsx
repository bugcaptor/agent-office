// src/renderer/about/AboutDialog.tsx
//
// 정보(About) 다이얼로그(BottomBar ℹ로 열림). SettingsDialog와 동일한
// 패턴 — 스토어의 modal.kind로만 열림/닫힘을 판단하는 단순 표시용 모달로,
// 편집 가능한 상태는 없다. 버전은 vite.config.ts/vitest.config.ts의
// `define`으로 주입된 `__APP_VERSION__`(package.json 기준)을 그대로 쓴다.
import { useTranslation } from "react-i18next";

import { useAppStore } from "../store/appStore";

export function AboutDialog() {
  const { t } = useTranslation("app");
  const modal = useAppStore((s) => s.modal);
  const closeModal = useAppStore((s) => s.closeModal);

  if (modal.kind !== "about") return null;

  return (
    <div
      className="modal-backdrop"
      onMouseDown={(e) => {
        if (e.button === 0 && e.target === e.currentTarget) closeModal();
      }}
    >
      <div className="pixel-panel about-dialog">
        <h2 className="pixel-title">Agent Office</h2>
        <p className="about-version">{t("about.version", { version: __APP_VERSION__ })}</p>
        <p className="about-desc">{t("about.desc")}</p>
        <p className="about-license">MIT License © 2026 bugcaptor</p>
        <div className="dialog-actions">
          <button className="pixel-btn" onClick={closeModal}>
            {t("about.close")}
          </button>
        </div>
      </div>
    </div>
  );
}
