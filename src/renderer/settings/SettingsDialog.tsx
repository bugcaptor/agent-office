// src/renderer/settings/SettingsDialog.tsx
//
// 상시 설정 다이얼로그(BottomBar ⚙로 열림). FirstRunDialog와 달리 스토어
// 값을 직접 바인딩 — 토글 즉시 updateAppSettings로 저장된다(확인 버튼 없음).
import { useAppStore } from "../store/appStore";
import { SettingsForm } from "./SettingsForm";
import type { ExternalEditorApp, ExternalTerminalApp } from "@shared/types";

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
          <label className="settings-item">
            <span>
              <strong>질문 알림 지연 (초)</strong>
              <small>
                질문 알림을 이 시간만큼 보류하고, 그 사이 에이전트가 계속
                일하면(오토모드 자동 승인 등) 알림을 내지 않습니다. 0이면 즉시
                알림.
              </small>
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
          <label className="settings-item">
            <span>
              <strong>외부 터미널 앱</strong>
              <small>
                터미널 탭 우클릭 "OS 터미널로 열기"가 사용할 앱입니다. macOS
                전용 — 다른 OS에서는 무시됩니다.
              </small>
            </span>
            <select
              value={appSettings.externalTerminal}
              onChange={(e) =>
                updateAppSettings({
                  externalTerminal: e.target.value as ExternalTerminalApp,
                })
              }
            >
              <option value="terminal">Terminal (기본)</option>
              <option value="iterm">iTerm2</option>
            </select>
          </label>
          <label className="settings-item">
            <span>
              <strong>셸 출력 에디터</strong>
              <small>
                터미널 탭 우클릭 "셸 출력을 에디터로 보기"(단축키 Cmd/Ctrl+Shift+E)가
                .txt를 열 때 사용할 앱입니다.
              </small>
            </span>
            <select
              value={appSettings.externalEditor}
              onChange={(e) =>
                updateAppSettings({
                  externalEditor: e.target.value as ExternalEditorApp,
                })
              }
            >
              <option value="system">시스템 기본</option>
              <option value="vscode">VS Code</option>
            </select>
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
