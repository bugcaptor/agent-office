// src/renderer/settings/SettingsDialog.tsx
//
// 상시 설정 다이얼로그(BottomBar ⚙로 열림). FirstRunDialog와 달리 스토어
// 값을 직접 바인딩 — 토글 즉시 updateAppSettings로 저장된다(확인 버튼 없음).
import { useAppStore } from "../store/appStore";
import { SettingsForm } from "./SettingsForm";

export function SettingsDialog() {
  const modal = useAppStore((s) => s.modal);
  const closeModal = useAppStore((s) => s.closeModal);
  const appSettings = useAppStore((s) => s.appSettings);
  const updateAppSettings = useAppStore((s) => s.updateAppSettings);

  if (modal.kind !== "settings") return null;

  return (
    <div
      className="modal-backdrop"
      onMouseDown={(e) => {
        if (e.button === 0 && e.target === e.currentTarget) closeModal();
      }}
    >
      <div className="pixel-panel settings-dialog">
        <h2 className="pixel-title">설정</h2>
        <SettingsForm
          value={{
            summarizerEnabled: appSettings.summarizerEnabled,
            summaryProvider: appSettings.summaryProvider,
            observerEnabled: appSettings.observerEnabled,
          }}
          onChange={updateAppSettings}
        />
        <div className="settings-form">
          <label className="settings-item">
            <input
              type="checkbox"
              checked={appSettings.soundEnabled}
              onChange={(e) => updateAppSettings({ soundEnabled: e.target.checked })}
            />
            <span>
              <strong>사무실 사운드</strong>
              <small>
                에이전트가 일할 때 키보드 타이핑 소리와 알림·출퇴근 효과음을
                재생합니다.
              </small>
            </span>
          </label>
          <label className="settings-item">
            <span>
              <strong>볼륨</strong>
            </span>
            <input
              type="range"
              min={0}
              max={100}
              value={Math.round(appSettings.soundVolume * 100)}
              disabled={!appSettings.soundEnabled}
              onChange={(e) => updateAppSettings({ soundVolume: Number(e.target.value) / 100 })}
            />
          </label>
        </div>
        <div className="dialog-actions">
          <button className="pixel-btn" onClick={closeModal}>
            닫기
          </button>
        </div>
      </div>
    </div>
  );
}
