// src/renderer/settings/SettingsDialog.tsx
//
// 상시 설정 다이얼로그(BottomBar ⚙로 열림). FirstRunDialog와 달리 스토어
// 값을 직접 바인딩 — 토글 즉시 updateAppSettings로 저장된다(확인 버튼 없음).
import { useCallback, useEffect, useState } from "react";
import { useAppStore } from "../store/appStore";
import { tauriApi } from "../ipc/tauriApi";
import { SettingsForm } from "./SettingsForm";
import type {
  ControlStatus,
  ExternalEditorApp,
  ExternalTerminalApp,
  FileIndexBackend,
} from "@shared/types";

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
            diaryEnabled: appSettings.diaryEnabled,
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
            <input
              type="checkbox"
              checked={appSettings.gitStatusEnabled}
              onChange={(e) => updateAppSettings({ gitStatusEnabled: e.target.checked })}
            />
            <span>
              <strong>작업 폴더 git 상태 표시</strong>
              <small>
                "작업 폴더 보기"에서 파일별 git 변경 상태(수정·추가·삭제 등)를
                조회해 뱃지로 보여줍니다. 거대 저장소에서 느리면 끄세요.
              </small>
            </span>
          </label>
          <label className="settings-item">
            <input
              type="checkbox"
              checked={appSettings.keepAwakeEnabled}
              onChange={(e) => updateAppSettings({ keepAwakeEnabled: e.target.checked })}
            />
            <span>
              <strong>작업 중 시스템 잠자기 방지</strong>
              <small>
                캐릭터가 작업하는 동안 컴퓨터가 자동으로 잠들지 않게 합니다. 화면은
                꺼질 수 있으며, 뚜껑을 닫거나 수동으로 재우는 것은 막지 않습니다.
                (macOS·Windows)
              </small>
            </span>
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
          <label className="settings-item">
            <span>
              <strong>파일 목록 백엔드</strong>
              <small>
                Everything(es.exe)은 Windows 전용·문서(md) 팔레트 한정, 실패
                시 자동으로 기본 스캐너를 사용합니다.
              </small>
            </span>
            <select
              value={appSettings.fileIndexBackend}
              onChange={(e) =>
                updateAppSettings({
                  fileIndexBackend: e.target.value as FileIndexBackend,
                })
              }
            >
              <option value="walker">기본 스캐너 (walker)</option>
              <option value="everything">Everything (es.exe)</option>
            </select>
          </label>
        </div>
        <ControlSection enabled={appSettings.cliEnabled} />
        <div className="dialog-actions">
          <button className="pixel-btn" onClick={closeModal}>
            닫기
          </button>
        </div>
      </div>
    </div>
  );
}

/**
 * CLI 제어(이슈 #55) 설정 — 2단계 옵트인. 1단계: "CLI 제어 활성화" 토글로
 * 로컬 control 서버를 켠다(control-port 기록). 2단계: "승인"으로 토큰을
 * 발급해야만 실제로 명령이 실행된다. 승인 전에는 서버가 떠 있어도 모든 요청
 * 401. 승인은 지속되며 "승인 취소"로 토큰을 폐기할 수 있다.
 */
function ControlSection({ enabled }: { enabled: boolean }) {
  const updateAppSettings = useAppStore((s) => s.updateAppSettings);
  const [status, setStatus] = useState<ControlStatus | null>(null);
  const [busy, setBusy] = useState(false);

  const refresh = useCallback(async () => {
    try {
      setStatus(await tauriApi.controlStatus());
    } catch {
      setStatus(null);
    }
  }, []);

  useEffect(() => {
    void refresh();
  }, [refresh, enabled]);

  const approve = async () => {
    setBusy(true);
    try {
      await tauriApi.controlApprove();
      await refresh();
    } finally {
      setBusy(false);
    }
  };

  const revoke = async () => {
    setBusy(true);
    try {
      await tauriApi.controlRevoke();
      await refresh();
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="settings-form">
      <label className="settings-item">
        <input
          type="checkbox"
          checked={enabled}
          onChange={(e) => updateAppSettings({ cliEnabled: e.target.checked })}
        />
        <span>
          <strong>CLI 제어 (외부 조종)</strong>
          <small>
            <code>agent-office ctl …</code> 또는 스크립트가 이 앱을 조종하도록
            로컬(127.0.0.1) 제어 서버를 엽니다. 켜도 아래에서 <b>명시적으로
            승인</b>해야 명령이 실행됩니다(2단계). 보안 표면이므로 기본 꺼짐.
          </small>
        </span>
      </label>

      {enabled && (
        <div className="settings-item" style={{ flexDirection: "column", alignItems: "stretch", gap: 8 }}>
          <div style={{ fontSize: 12, opacity: 0.85 }}>
            상태:{" "}
            {status
              ? `서버 ${status.running ? `실행 중(포트 ${status.port ?? "?"})` : "정지"} · ${
                  status.approved ? "승인됨" : "미승인"
                }`
              : "조회 중…"}
          </div>

          {status && !status.approved && (
            <button className="pixel-btn" disabled={busy} onClick={approve}>
              CLI 제어 승인 (토큰 발급)
            </button>
          )}
          {status && status.approved && (
            <>
              <button className="pixel-btn" disabled={busy} onClick={revoke}>
                승인 취소 (토큰 폐기)
              </button>
              <div style={{ fontSize: 12, opacity: 0.85 }}>
                <div style={{ marginBottom: 4 }}>
                  세션 터미널 안에서는 바로 사용할 수 있습니다:
                </div>
                <code style={{ display: "block", whiteSpace: "pre-wrap" }}>
                  agent-office ctl status{"\n"}
                  agent-office ctl list{"\n"}
                  agent-office ctl send &lt;agentId&gt; "npm test" --enter
                </code>
                <div style={{ marginTop: 6, opacity: 0.7 }}>
                  외부 스크립트는 app_data 자동발견을 씁니다:{" "}
                  <code>{status.appDataDir}</code>
                </div>
              </div>
            </>
          )}
        </div>
      )}
    </div>
  );
}
