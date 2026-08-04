// src/renderer/settings/SettingsDialog.tsx
//
// 상시 설정 다이얼로그(BottomBar ⚙로 열림). FirstRunDialog와 달리 스토어
// 값을 직접 바인딩 — 토글 즉시 updateAppSettings로 저장된다(확인 버튼 없음).
import { useCallback, useEffect, useState } from "react";
import { useAppStore } from "../store/appStore";
import { tauriApi } from "../ipc/tauriApi";
import { SettingsForm } from "./SettingsForm";
import { PeerShareSection } from "./PeerShareSection";
import { previewVoice } from "../sound/soundManager";
import { THEMES, THEME_ORDER } from "../theme/themes";
import type { XtermThemeOverride } from "../terminal/theme";
import type {
  ControlStatus,
  ExternalEditorApp,
  ExternalTerminalApp,
  FileIndexBackend,
  TtsRewriteModel,
  TtsRewriteProvider,
  TtsStatus,
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
              checked={appSettings.typingSoundEnabled}
              onChange={(e) => updateAppSettings({ typingSoundEnabled: e.target.checked })}
            />
            <span>
              <strong>타건음</strong>
              <small>에이전트가 일할 때 나는 키보드 타이핑 소리입니다.</small>
            </span>
          </label>
          <label className="settings-item">
            <input
              type="checkbox"
              checked={appSettings.notifySoundEnabled}
              onChange={(e) => updateAppSettings({ notifySoundEnabled: e.target.checked })}
            />
            <span>
              <strong>알림음</strong>
              <small>알림이 왔을 때의 딩과 세션 시작·종료 효과음입니다.</small>
            </span>
          </label>
          <label className="settings-item">
            <span>
              <strong>볼륨</strong>
              <small>위 스위치들과 대사 읽어주기가 함께 씁니다.</small>
            </span>
            <input
              type="range"
              min={0}
              max={100}
              value={Math.round(appSettings.soundVolume * 100)}
              disabled={
                !appSettings.typingSoundEnabled &&
                !appSettings.notifySoundEnabled &&
                !appSettings.ttsEnabled
              }
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
            <input
              type="checkbox"
              checked={appSettings.sessionLogEnabled}
              onChange={(e) => updateAppSettings({ sessionLogEnabled: e.target.checked })}
            />
            <span>
              <strong>세션 로그 남기기</strong>
              <small>
                터미널에서 오간 내용(AI 대화·명령·출력)을 읽을 수 있는 텍스트로
                파일에 기록합니다. 캐릭터 탭 우클릭 "세션 로그 보기"에서 열람하고
                학습자료로 정리할 수 있습니다. 30일이 지나거나 전체 2GB를 넘으면
                오래된 것부터 자동 삭제됩니다.
              </small>
            </span>
          </label>
          <label className="settings-item">
            <input
              type="checkbox"
              checked={appSettings.mascotEnabled}
              onChange={(e) => updateAppSettings({ mascotEnabled: e.target.checked })}
            />
            <span>
              <strong>데스크톱 마스코트</strong>
              <small>
                지금 활동 중인 캐릭터를 앱 창과 별개의 작은 창으로 항상 위에
                띄웁니다. 알림이 오면 그 자리에서 알리고, 클릭하면 해당 캐릭터의
                터미널이 열립니다.
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
          <TerminalThemeItem />
        </div>
        <TtsSection />
        <ControlSection enabled={appSettings.cliEnabled} />
        <PeerShareSection />
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
 * 터미널 색상 선택. 다른 항목과 달리 AppSettings(Rust 영속)가 아니라
 * zustand + localStorage에 사는 값이라 `updateAppSettings`가 아닌 전용 액션에
 * 직접 바인딩한다(테마 자체와 같은 계층 — theme/applyTheme.ts 참고).
 */
function TerminalThemeItem() {
  const xtermTheme = useAppStore((s) => s.xtermTheme);
  const setXtermTheme = useAppStore((s) => s.setXtermTheme);

  return (
    <label className="settings-item">
      <span>
        <strong>터미널 색상</strong>
        <small>
          터미널 창 색만 별도 테마로 고정합니다. 기본값(테마 따름)이면 앱 테마를
          바꿀 때 터미널도 함께 바뀝니다.
        </small>
      </span>
      <select
        value={xtermTheme}
        onChange={(e) => setXtermTheme(e.target.value as XtermThemeOverride)}
      >
        <option value="auto">테마 따름 (기본)</option>
        {THEME_ORDER.map((id) => (
          <option key={id} value={id}>
            {THEMES[id].label}
          </option>
        ))}
      </select>
    </label>
  );
}

/** 리라이트 경로 라벨 → 사람이 읽는 이름. "자동"이 실제로 무엇을 고를지 알려준다. */
const REWRITE_VIA_LABEL: Record<TtsRewriteProvider, string> = {
  auto: "자동",
  api: "Anthropic API",
  "claude-cli": "claude CLI (구독)",
  none: "리라이트 없음 (원문 발화)",
};

/**
 * 확인 요청 대사 TTS 설정.
 *
 * 키는 스토어(appSettings)에 들어오지 않는다 — 백엔드가 0600 파일에만 보관하고
 * 여기에는 **존재 여부**(`TtsStatus`)만 내려온다. 그래서 입력 필드는 항상
 * 빈 채로 시작하고, 저장 버튼을 눌러야 백엔드로 넘어간다(빈 값 저장 = 삭제).
 */
function TtsSection() {
  const appSettings = useAppStore((s) => s.appSettings);
  const updateAppSettings = useAppStore((s) => s.updateAppSettings);
  // 미리듣기는 무음 모드에서도 울린다(방금 누른 버튼이 침묵하면 고장으로
  // 보인다). 대신 "실제 알림은 안 나온다"는 사실을 여기서 말해 준다 —
  // 무음인 줄 모르고 "왜 발화가 안 되지"로 헤매는 사고가 실제로 있었다.
  const muted = useAppStore((s) => s.muted);
  const [status, setStatus] = useState<TtsStatus | null>(null);
  const [elevenlabs, setElevenlabs] = useState("");
  const [anthropic, setAnthropic] = useState("");
  const [busy, setBusy] = useState(false);
  const [note, setNote] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    try {
      setStatus(await tauriApi.ttsKeyStatus());
    } catch {
      setStatus(null);
    }
  }, []);

  useEffect(() => {
    void refresh();
  }, [refresh, appSettings.ttsEnabled, appSettings.ttsRewriteProvider]);

  const saveKeys = async () => {
    setBusy(true);
    setNote(null);
    try {
      // 손대지 않은 필드는 undefined로 보내 기존 값을 보존한다.
      const next = await tauriApi.ttsSetKeys(
        elevenlabs === "" ? undefined : elevenlabs,
        anthropic === "" ? undefined : anthropic
      );
      setStatus(next);
      setElevenlabs("");
      setAnthropic("");
      setNote("키를 저장했습니다.");
    } catch (err) {
      setNote(`키 저장 실패: ${String(err)}`);
    } finally {
      setBusy(false);
    }
  };

  const preview = async () => {
    setBusy(true);
    setNote(null);
    try {
      const line = await previewVoice();
      setNote(line ? `발화: ${line}` : "발화할 수 없었습니다.");
    } catch (err) {
      setNote(`시청 실패: ${String(err)}`);
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="settings-form">
      <label className="settings-item">
        <input
          type="checkbox"
          checked={appSettings.ttsEnabled}
          onChange={(e) => updateAppSettings({ ttsEnabled: e.target.checked })}
        />
        <span>
          <strong>알림 대사 읽어주기 (TTS)</strong>
          <small>
            캐릭터가 확인을 기다리거나 작업을 마쳤을 때, 그 알림 문구를 캐릭터
            말투의 짧은 대사로 바꿔 목소리로 읽어줍니다(벨 알림은 제외).
            목소리는 캐릭터 종족에 맞춰 자동으로 정해지며, 캐릭터 편집에서 직접
            고를 수도 있습니다. ElevenLabs 음성 합성 크레딧을 소모하므로 기본 꺼짐.
          </small>
        </span>
      </label>

      {appSettings.ttsEnabled && (
        <>
          <label className="settings-item">
            <span>
              <strong>대사 리라이트</strong>
              <small>
                시스템 문구를 캐릭터 말투로 바꾸는 방법입니다. claude CLI는 API
                키 없이 쓸 수 있지만 <b>구독 사용량을 소모합니다</b>. "리라이트
                없음"은 원문 문구를 그대로 읽습니다.
              </small>
            </span>
            <select
              value={appSettings.ttsRewriteProvider}
              onChange={(e) =>
                updateAppSettings({
                  ttsRewriteProvider: e.target.value as TtsRewriteProvider,
                })
              }
            >
              <option value="auto">자동 (API 키 → claude CLI → 끄기)</option>
              <option value="api">Anthropic API 키</option>
              <option value="claude-cli">claude CLI (구독 사용량 소모)</option>
              <option value="none">리라이트 없음 (원문 발화)</option>
            </select>
          </label>

          <label className="settings-item">
            <span>
              <strong>리라이트 모델</strong>
              <small>한 줄 대사 변환이라 기본(Haiku)으로 충분합니다.</small>
            </span>
            <select
              value={appSettings.ttsRewriteModel}
              onChange={(e) =>
                updateAppSettings({
                  ttsRewriteModel: e.target.value as TtsRewriteModel,
                })
              }
            >
              <option value="claude-haiku-4-5">Haiku 4.5 (기본·빠름)</option>
              <option value="claude-sonnet-5">Sonnet 5</option>
              <option value="claude-opus-5">Opus 5</option>
            </select>
          </label>

          <div
            className="settings-item"
            style={{ flexDirection: "column", alignItems: "stretch", gap: 8 }}
          >
            <div style={{ fontSize: 12, opacity: 0.85 }}>
              {status
                ? `ElevenLabs 키 ${
                    status.elevenlabsSet
                      ? status.elevenlabsFromEnv
                        ? "있음(환경변수)"
                        : "있음"
                      : "없음"
                  } · Anthropic 키 ${
                    status.anthropicSet
                      ? status.anthropicFromEnv
                        ? "있음(환경변수)"
                        : "있음"
                      : "없음"
                  } · claude CLI ${status.claudeCliAvailable ? "있음" : "없음"} → 리라이트: ${
                    REWRITE_VIA_LABEL[status.effectiveRewriteVia]
                  }`
                : "상태 조회 중…"}
            </div>

            <label className="settings-item">
              <span>
                <strong>ElevenLabs API 키</strong>
                <small>
                  음성 합성에 필수입니다. 저장하면 앱에만 보관되고 화면에 다시
                  표시되지 않습니다. 비우고 저장하면 삭제됩니다
                  (<code>ELEVENLABS_API_KEY</code> 환경변수도 폴백으로 인정).
                </small>
              </span>
              <input
                type="password"
                autoComplete="off"
                placeholder={status?.elevenlabsSet ? "저장됨 (변경 시 입력)" : "xi-…"}
                value={elevenlabs}
                onChange={(e) => setElevenlabs(e.target.value)}
              />
            </label>

            <label className="settings-item">
              <span>
                <strong>Anthropic API 키 (선택)</strong>
                <small>
                  대사 리라이트에만 쓰입니다. 비어 있으면{" "}
                  <code>ANTHROPIC_API_KEY</code> 환경변수를 쓰고, 그것도 없으면
                  claude CLI로 넘어갑니다.
                </small>
              </span>
              <input
                type="password"
                autoComplete="off"
                placeholder={status?.anthropicSet ? "저장됨 (변경 시 입력)" : "sk-ant-…"}
                value={anthropic}
                onChange={(e) => setAnthropic(e.target.value)}
              />
            </label>

            <div style={{ display: "flex", gap: 8 }}>
              <button
                className="pixel-btn"
                disabled={busy || (elevenlabs === "" && anthropic === "")}
                onClick={saveKeys}
              >
                키 저장
              </button>
              <button className="pixel-btn" disabled={busy} onClick={preview}>
                시청 (미리듣기)
              </button>
            </div>
            {muted && (
              <div style={{ fontSize: 12, opacity: 0.85 }}>
                무음 모드가 켜져 있어 실제 알림은 발화되지 않습니다 (미리듣기는
                들립니다).
              </div>
            )}
            {note && <div style={{ fontSize: 12, opacity: 0.85 }}>{note}</div>}
          </div>
        </>
      )}
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
