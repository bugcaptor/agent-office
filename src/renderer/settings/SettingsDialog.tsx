// src/renderer/settings/SettingsDialog.tsx
//
// 상시 설정 다이얼로그(BottomBar ⚙로 열림). FirstRunDialog와 달리 스토어
// 값을 직접 바인딩 — 토글 즉시 updateAppSettings로 저장된다(확인 버튼 없음).
//
// 항목이 불어나 한 화면 스크롤로는 못 찾겠어서 탭 4개(일반/소리·음성/
// 시스템/제어)로 나눴다. 탭은 다이얼로그 로컬 상태이고 기억하지 않는다 —
// 열 때마다 첫 탭. 그래서 게이팅(SettingsDialog)과 본체(SettingsDialogBody)를
// 나눠, 닫으면 본체가 언마운트되며 탭 상태가 함께 사라지게 한다(이 컴포넌트는
// App에 상시 마운트돼 있어 useState만으로는 초기화되지 않는다).
import { useCallback, useEffect, useState } from "react";
import { useAppStore } from "../store/appStore";
import { tauriApi } from "../ipc/tauriApi";
import { SettingsForm } from "./SettingsForm";
import { WebRemoteSection } from "./WebRemoteSection";
import { OpenrouterModelDatalist } from "./openrouterModels";
import { previewVoice } from "../sound/soundManager";
import { THEMES, THEME_ORDER } from "../theme/themes";
import type { XtermThemeOverride } from "../terminal/theme";
import type {
  ControlStatus,
  ExternalEditorApp,
  ExternalTerminalApp,
  FileIndexBackend,
  SummaryProvider,
  TtsRewriteProvider,
  TtsStatus,
} from "@shared/types";

type SettingsTabId = "general" | "sound" | "system" | "control";

/** 화면 순서 = 이 배열 순서. 첫 항목이 열 때마다의 기본 탭이다. */
const SETTINGS_TABS: { id: SettingsTabId; label: string }[] = [
  { id: "general", label: "일반" },
  { id: "sound", label: "소리·음성" },
  { id: "system", label: "시스템" },
  { id: "control", label: "제어" },
];

export function SettingsDialog() {
  const modal = useAppStore((s) => s.modal);
  if (modal.kind !== "settings") return null;
  return <SettingsDialogBody />;
}

function SettingsDialogBody() {
  const closeModal = useAppStore((s) => s.closeModal);
  const cliEnabled = useAppStore((s) => s.appSettings.cliEnabled);
  const [tab, setTab] = useState<SettingsTabId>(SETTINGS_TABS[0].id);

  return (
    <div
      className="modal-backdrop"
      onMouseDown={(e) => {
        if (e.button === 0 && e.target === e.currentTarget) closeModal();
      }}
    >
      <div className="pixel-panel settings-dialog">
        <h2 className="pixel-title">설정</h2>

        <div className="settings-tabs" role="tablist" aria-label="설정 분류">
          {SETTINGS_TABS.map((t) => (
            <button
              key={t.id}
              type="button"
              role="tab"
              id={`settings-tab-${t.id}`}
              aria-selected={tab === t.id}
              aria-controls={`settings-tabpanel-${t.id}`}
              className={tab === t.id ? "settings-tab settings-tab-active" : "settings-tab"}
              onClick={() => setTab(t.id)}
            >
              {t.label}
            </button>
          ))}
        </div>

        <div
          className="settings-tabpanel"
          role="tabpanel"
          id={`settings-tabpanel-${tab}`}
          aria-labelledby={`settings-tab-${tab}`}
          tabIndex={0}
        >
          {tab === "general" && <GeneralTab />}
          {tab === "sound" && <SoundTab />}
          {tab === "system" && <SystemTab />}
          {tab === "control" && (
            <>
              <ControlSection enabled={cliEnabled} />
              <WebRemoteSection />
            </>
          )}
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

/** 일반 — 요약 라벨·요약기·일기·관찰. FirstRunDialog와 공유하는 폼 그대로. */
function GeneralTab() {
  const appSettings = useAppStore((s) => s.appSettings);
  const updateAppSettings = useAppStore((s) => s.updateAppSettings);

  return (
    <>
      <SettingsForm
        value={{
          summarizerEnabled: appSettings.summarizerEnabled,
          summaryProvider: appSettings.summaryProvider,
          diaryEnabled: appSettings.diaryEnabled,
          observerEnabled: appSettings.observerEnabled,
        }}
        onChange={updateAppSettings}
      />
      <SummaryModelSection />
    </>
  );
}

/** 요약기 provider별 기본 모델(비우면 이 값이 쓰인다) — 백엔드
 * `summarizer::SummaryPurpose`의 하드코딩 값과 같아야 한다. 안내 문구
 * (placeholder)에만 쓰이므로 어긋나도 동작에는 영향이 없다. */
const SUMMARY_DEFAULT_MODELS: Record<SummaryProvider, { light: string; heavy: string }> = {
  claude: { light: "haiku", heavy: "sonnet" },
  codex: { light: "gpt-5.4-mini", heavy: "gpt-5.4" },
  agy: { light: "gemini-3.6-flash-low", heavy: "gemini-3.1-pro-low" },
  gemini: { light: "gemini-2.5-flash", heavy: "gemini-2.5-pro" },
  opencode: {
    light: "opencode-go/deepseek-v4-flash",
    heavy: "opencode-go/deepseek-v4-pro",
  },
  openrouter: { light: "openai/gpt-5.4-mini", heavy: "openai/gpt-5.4" },
};

const SUMMARY_PROVIDER_LABEL: Record<SummaryProvider, string> = {
  claude: "Claude",
  codex: "Codex",
  agy: "Antigravity (agy)",
  gemini: "Gemini",
  opencode: "opencode",
  openrouter: "OpenRouter",
};

/**
 * 요약 모델 오버라이드 — 지금 고른 요약기의 경량/고급 모델만 노출한다.
 * (SettingsForm은 FirstRunDialog와 공유하는 폼이라 손대지 않는다 — 첫 실행
 * 온보딩에서 모델 id까지 물을 이유가 없다.)
 *
 * 비우면 백엔드 기본값. 값은 그대로 해당 CLI의 `--model`로 실리므로 앱이
 * 목록을 강제하지 않는다 — 새 모델이 나올 때마다 앱을 고쳐야 하는 것보다,
 * 오타가 나면 그 요약이 실패해 원문 폴백으로 강등되는 편이 낫다.
 */
function SummaryModelSection() {
  const provider = useAppStore((s) => s.appSettings.summaryProvider);
  const summaryModels = useAppStore((s) => s.appSettings.summaryModels);
  const updateAppSettings = useAppStore((s) => s.updateAppSettings);
  const current = summaryModels[provider];
  const defaults = SUMMARY_DEFAULT_MODELS[provider];
  // OpenRouter만 CLI가 아니라 HTTP라 API 키가 따로 필요하다 — 키 입력과 연결
  // 테스트는 아래 OpenrouterSummaryTools가 맡는다(저장소는 소리·음성 탭과 공유).
  const isOpenrouter = provider === "openrouter";
  // opencode는 한 CLI가 여러 벤더를 묶는다 — 모델 id가 `provider/model`이고
  // 기본값은 opencode 자체 구독(opencode-go)을 가정한다. 다른 벤더를 쓰려면
  // 여기에 `opencode models`가 찍어 주는 id를 그대로 넣는다.
  const isOpencode = provider === "opencode";
  // 모델 id 추천은 TTS 쪽과 같은 목록을 쓴다(중복 정의하면 갈라진다).
  const modelListId = isOpenrouter ? "summary-openrouter-models" : undefined;

  const setModel = (key: "light" | "heavy", value: string) =>
    updateAppSettings({
      summaryModels: {
        ...summaryModels,
        [provider]: { ...current, [key]: value },
      },
    });

  return (
    <div className="settings-form">
      {isOpenrouter && (
        <p className="settings-note">
          OpenRouter 요약은 API 키(또는 환경변수 <code>OPENROUTER_API_KEY</code>)를
          씁니다. 키가 없으면 요약이 실패하고 원문이 그대로 표시됩니다.
        </p>
      )}
      {isOpencode && (
        <p className="settings-note">
          opencode 요약은 설치된 <code>opencode</code> CLI를 부릅니다. 모델 id는
          <code> provider/model</code> 표기이고(<code>opencode models</code>로 확인),
          기본값은 opencode 자체 구독(<code>opencode-go</code>)을 가정합니다.
        </p>
      )}
      <label className="settings-item">
        <span>
          <strong>{SUMMARY_PROVIDER_LABEL[provider]} 경량 모델</strong>
          <small>
            작업 라벨 요약과 캐릭터 일기에 쓰는 모델입니다. 비우면 기본값(
            <code>{defaults.light}</code>)을 씁니다.
          </small>
        </span>
        <input
          type="text"
          autoComplete="off"
          spellCheck={false}
          list={modelListId}
          placeholder={defaults.light}
          value={current.light}
          onChange={(e) => setModel("light", e.target.value)}
        />
      </label>
      <label className="settings-item">
        <span>
          <strong>{SUMMARY_PROVIDER_LABEL[provider]} 고급 모델</strong>
          <small>
            세션 로그 학습자료처럼 긴 글을 정리할 때 쓰는 모델입니다. 비우면
            기본값(<code>{defaults.heavy}</code>)을 씁니다.
          </small>
        </span>
        <input
          type="text"
          autoComplete="off"
          spellCheck={false}
          list={modelListId}
          placeholder={defaults.heavy}
          value={current.heavy}
          onChange={(e) => setModel("heavy", e.target.value)}
        />
      </label>
      {isOpenrouter && (
        <>
          <OpenrouterModelDatalist id="summary-openrouter-models" />
          <OpenrouterSummaryTools />
        </>
      )}
    </div>
  );
}

/** 요약 테스트가 실패했을 때 그대로 보여주면 뜻이 통하지 않는 코드들.
 *  나머지는 원문을 보여준다 — 상류 오류는 종류가 열려 있다. */
const SUMMARY_TEST_ERROR_LABEL: Record<string, string> = {
  "summarizer-disabled": "요약 기능이 꺼져 있습니다(일반 탭에서 켜세요)",
  "openrouter-key-missing": "OpenRouter API 키가 없습니다",
};

/** 요약 테스트에 쓰는 표본. 짧아야 한다 — 크레딧을 쓰는 실제 호출이다. */
const SUMMARY_TEST_INSTRUCTION = "다음 텍스트를 한 문장으로 요약하라.";
const SUMMARY_TEST_TEXT =
  "설정 화면에서 OpenRouter 연결을 확인하려고 보낸 시험 문장입니다. " +
  "요약이 돌아오면 키와 경량 모델 설정이 모두 올바른 것입니다.";

/**
 * OpenRouter 요약을 위한 키 입력과 연결 테스트.
 *
 * 키는 **소리·음성 탭과 같은 0600 저장소**를 그대로 쓴다(`ttsSetKeys`의 셋째
 * 칸). 요약 전용 키를 따로 두면 같은 키를 두 번 넣게 되고 어느 쪽이 실제로
 * 쓰이는지 알 수 없게 된다 — 백엔드도 키를 하나만 읽는다.
 *
 * 테스트는 전용 커맨드가 아니라 `summarizeText`(라벨 목적)를 그대로 탄다 —
 * 여기서 성공하면 실제 라벨 요약도 같은 키·같은 경량 모델로 성공한다는 뜻이
 * 돼야 하기 때문이다.
 */
function OpenrouterSummaryTools() {
  const [status, setStatus] = useState<TtsStatus | null>(null);
  const [apiKey, setApiKey] = useState("");
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
  }, [refresh]);

  const saveKey = async () => {
    setBusy(true);
    setNote(null);
    try {
      // 앞 두 칸은 undefined — 여기서는 OpenRouter 키만 건드린다.
      setStatus(await tauriApi.ttsSetKeys(undefined, undefined, apiKey));
      setApiKey("");
      setNote("키를 저장했습니다.");
    } catch (err) {
      setNote(`키 저장 실패: ${String(err)}`);
    } finally {
      setBusy(false);
    }
  };

  // 빈 문자열을 보내는 것이 백엔드의 삭제 신호다(undefined=보존). 입력창을
  // 비우고 저장하는 경로로는 여기에 절대 닿지 않으므로 전용 버튼이 필요하다.
  const deleteKey = async () => {
    setBusy(true);
    setNote(null);
    try {
      setStatus(await tauriApi.ttsSetKeys(undefined, undefined, ""));
      setNote("키를 삭제했습니다.");
    } catch (err) {
      setNote(`키 삭제 실패: ${String(err)}`);
    } finally {
      setBusy(false);
    }
  };

  const test = async () => {
    setBusy(true);
    setNote(null);
    try {
      const out = await tauriApi.summarizeText(
        "openrouter",
        SUMMARY_TEST_INSTRUCTION,
        SUMMARY_TEST_TEXT,
        "label",
      );
      setNote(`요약: ${out}`);
    } catch (err) {
      const code = String(err);
      setNote(`요약 실패: ${SUMMARY_TEST_ERROR_LABEL[code] ?? code}`);
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="settings-item" style={{ flexDirection: "column", alignItems: "stretch", gap: 8 }}>
      <div style={{ fontSize: 12, opacity: 0.85 }}>
        {status
          ? `OpenRouter 키 ${
              status.openrouterSet
                ? status.openrouterFromEnv
                  ? "있음(환경변수)"
                  : "있음"
                : "없음"
            }`
          : "상태 조회 중…"}
      </div>

      <label className="settings-item">
        <span>
          <strong>OpenRouter API 키</strong>
          <small>
            이 키는 <b>소리·음성</b> 탭의 OpenRouter 키와 같은 저장소를 씁니다
            (어느 쪽에서 넣어도 같습니다). 저장하면 앱에만 보관되고 화면에 다시
            표시되지 않습니다.
          </small>
        </span>
        <input
          type="password"
          autoComplete="off"
          placeholder={status?.openrouterSet ? "저장됨 (변경 시 입력)" : "sk-or-…"}
          value={apiKey}
          onChange={(e) => setApiKey(e.target.value)}
        />
      </label>

      <div style={{ display: "flex", gap: 8 }}>
        <button className="pixel-btn" disabled={busy || apiKey === ""} onClick={saveKey}>
          키 저장
        </button>
        <button className="pixel-btn" disabled={busy} onClick={test}>
          {busy ? "요약 테스트 중…" : "요약 테스트"}
        </button>
        {status?.openrouterSet && !status.openrouterFromEnv && (
          <button className="pixel-btn" disabled={busy} onClick={deleteKey}>
            키 삭제
          </button>
        )}
      </div>
      {note && <div style={{ fontSize: 12, opacity: 0.85 }}>{note}</div>}
    </div>
  );
}

/** 소리·음성 — 효과음/볼륨/알림 지연 + 대사 읽어주기(TTS). */
function SoundTab() {
  const appSettings = useAppStore((s) => s.appSettings);
  const updateAppSettings = useAppStore((s) => s.updateAppSettings);

  return (
    <>
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
      </div>
      <TtsSection />
    </>
  );
}

/** 시스템 — 앱 바깥(OS·저장소·외부 앱)에 닿는 설정과 터미널 색상. */
function SystemTab() {
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
  openrouter: "OpenRouter",
  "claude-cli": "claude CLI (구독)",
  none: "리라이트 없음 (원문 발화)",
};

/** 모델 입력 자유 텍스트의 추천 목록(datalist). 강제가 아니라 힌트다 — 새
 * 모델이 나와도 그냥 적어 넣으면 된다. (OpenRouter 쪽은 실시간 카탈로그와
 * 합쳐야 해서 openrouterModels.tsx가 따로 맡는다.) */
const ANTHROPIC_MODEL_PRESETS = ["claude-haiku-4-5", "claude-sonnet-5", "claude-opus-5"];

/**
 * 확인 요청 대사 TTS 설정.
 *
 * 키는 스토어(appSettings)에 들어오지 않는다 — 백엔드가 0600 파일에만 보관하고
 * 여기에는 **존재 여부**(`TtsStatus`)만 내려온다. 그래서 입력 필드는 항상
 * 빈 채로 시작하고, 저장 버튼을 눌러야 백엔드로 넘어간다. 입력을 비운 채
 * 저장하면 그 필드는 `undefined`로 넘어가 기존 값이 그대로 유지된다 — 삭제는
 * 이 값으로는 절대 닿을 수 없고, 저장된 키가 있을 때만 뜨는 전용 "키 삭제"
 * 버튼(빈 문자열 `""`을 보냄)으로만 한다.
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
  const [openrouter, setOpenrouter] = useState("");
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
        anthropic === "" ? undefined : anthropic,
        openrouter === "" ? undefined : openrouter
      );
      setStatus(next);
      setElevenlabs("");
      setAnthropic("");
      setOpenrouter("");
      setNote("키를 저장했습니다.");
    } catch (err) {
      setNote(`키 저장 실패: ${String(err)}`);
    } finally {
      setBusy(false);
    }
  };

  // 셋 중 지정한 한 칸만 빈 문자열(=삭제 신호)로 보내고 나머지 둘은
  // undefined(=보존)로 보낸다. 입력창을 비우고 저장하는 경로로는 빈 문자열에
  // 절대 닿지 않으므로(위 saveKeys 참고) 삭제 전용 버튼이 필요하다.
  const deleteKey = async (key: "elevenlabs" | "anthropic" | "openrouter") => {
    setBusy(true);
    setNote(null);
    try {
      const next = await tauriApi.ttsSetKeys(
        key === "elevenlabs" ? "" : undefined,
        key === "anthropic" ? "" : undefined,
        key === "openrouter" ? "" : undefined,
      );
      setStatus(next);
      setNote("키를 삭제했습니다.");
    } catch (err) {
      setNote(`키 삭제 실패: ${String(err)}`);
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
              <option value="openrouter">OpenRouter</option>
              <option value="claude-cli">claude CLI (구독 사용량 소모)</option>
              <option value="none">리라이트 없음 (원문 발화)</option>
            </select>
          </label>

          {/* 모델 입력은 공급자에 따라 하나만 보인다 — 지금 쓰이지 않는 쪽을
              같이 띄우면 어느 값이 실제로 쓰이는지 헷갈린다. "자동"과
              "claude CLI"는 Anthropic 모델 id 체계를 쓰므로 같은 칸이다. */}
          {appSettings.ttsRewriteProvider === "openrouter" ? (
            <label className="settings-item">
              <span>
                <strong>리라이트 모델 (OpenRouter)</strong>
                <small>
                  OpenRouter 모델 id를 <code>벤더/모델</code> 형식으로 적습니다.
                  목록에 없는 모델도 직접 입력할 수 있습니다.
                </small>
              </span>
              <input
                type="text"
                autoComplete="off"
                spellCheck={false}
                list="tts-openrouter-models"
                placeholder="openai/gpt-5.4-mini"
                value={appSettings.ttsRewriteModelOpenrouter}
                onChange={(e) =>
                  updateAppSettings({ ttsRewriteModelOpenrouter: e.target.value })
                }
              />
              <OpenrouterModelDatalist id="tts-openrouter-models" />
            </label>
          ) : (
            appSettings.ttsRewriteProvider !== "none" && (
              <label className="settings-item">
                <span>
                  <strong>리라이트 모델 (Anthropic)</strong>
                  <small>
                    한 줄 대사 변환이라 기본(Haiku)으로 충분합니다. 목록에 없는
                    모델도 직접 입력할 수 있습니다.
                  </small>
                </span>
                <input
                  type="text"
                  autoComplete="off"
                  spellCheck={false}
                  list="tts-anthropic-models"
                  placeholder="claude-haiku-4-5"
                  value={appSettings.ttsRewriteModelAnthropic}
                  onChange={(e) =>
                    updateAppSettings({ ttsRewriteModelAnthropic: e.target.value })
                  }
                />
                <datalist id="tts-anthropic-models">
                  {ANTHROPIC_MODEL_PRESETS.map((m) => (
                    <option key={m} value={m} />
                  ))}
                </datalist>
              </label>
            )
          )}

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
                  } · OpenRouter 키 ${
                    status.openrouterSet
                      ? status.openrouterFromEnv
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
                  표시되지 않습니다. 비워두고 저장하면 기존 키가 그대로
                  유지됩니다 — 삭제하려면 아래 "키 삭제" 버튼을 쓰세요
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
            {status?.elevenlabsSet && !status.elevenlabsFromEnv && (
              <div style={{ display: "flex", justifyContent: "flex-end" }}>
                <button className="pixel-btn" disabled={busy} onClick={() => deleteKey("elevenlabs")}>
                  ElevenLabs 키 삭제
                </button>
              </div>
            )}

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
            {status?.anthropicSet && !status.anthropicFromEnv && (
              <div style={{ display: "flex", justifyContent: "flex-end" }}>
                <button className="pixel-btn" disabled={busy} onClick={() => deleteKey("anthropic")}>
                  Anthropic 키 삭제
                </button>
              </div>
            )}

            <label className="settings-item">
              <span>
                <strong>OpenRouter API 키 (선택)</strong>
                <small>
                  위에서 공급자를 <b>OpenRouter</b>로 골랐을 때만 쓰입니다. 비어
                  있으면 <code>OPENROUTER_API_KEY</code> 환경변수를 씁니다.
                </small>
              </span>
              <input
                type="password"
                autoComplete="off"
                placeholder={status?.openrouterSet ? "저장됨 (변경 시 입력)" : "sk-or-…"}
                value={openrouter}
                onChange={(e) => setOpenrouter(e.target.value)}
              />
            </label>
            {status?.openrouterSet && !status.openrouterFromEnv && (
              <div style={{ display: "flex", justifyContent: "flex-end" }}>
                <button className="pixel-btn" disabled={busy} onClick={() => deleteKey("openrouter")}>
                  OpenRouter 키 삭제
                </button>
              </div>
            )}

            <div style={{ display: "flex", gap: 8 }}>
              <button
                className="pixel-btn"
                disabled={busy || (elevenlabs === "" && anthropic === "" && openrouter === "")}
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
