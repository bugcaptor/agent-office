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
import { Trans, useTranslation } from "react-i18next";
import { useAppStore } from "../store/appStore";
import { LANGUAGE_SYSTEM, applyLanguageSetting, availableLanguages } from "../i18n";
import { tauriApi } from "../ipc/tauriApi";
import { SettingsForm } from "./SettingsForm";
import { WebRemoteSection } from "./WebRemoteSection";
import { TalkSection } from "./TalkSection";
import { ModelPicker } from "./ModelPicker";
import { previewVoice } from "../sound/soundManager";
import { THEMES, THEME_ORDER } from "../theme/themes";
import { XTERM_PALETTES, XTERM_PALETTE_ORDER } from "../terminal/palettes";
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

/** 화면 순서 = 이 배열 순서. 첫 항목이 열 때마다의 기본 탭이다.
 *  모듈 최상위라 `t()`를 부를 수 없어 라벨이 아니라 **키**를 담는다 —
 *  언어를 바꾸면 렌더 시점에 다시 번역된다. */
const SETTINGS_TABS: { id: SettingsTabId; labelKey: string }[] = [
  { id: "general", labelKey: "dialog.tabGeneral" },
  { id: "sound", labelKey: "dialog.tabSound" },
  { id: "system", labelKey: "dialog.tabSystem" },
  { id: "control", labelKey: "dialog.tabControl" },
];

export function SettingsDialog() {
  const modal = useAppStore((s) => s.modal);
  if (modal.kind !== "settings") return null;
  return <SettingsDialogBody />;
}

function SettingsDialogBody() {
  const { t } = useTranslation("settings");
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
        <h2 className="pixel-title">{t("dialog.title")}</h2>

        <div className="settings-tabs" role="tablist" aria-label={t("dialog.tabsAria")}>
          {SETTINGS_TABS.map((item) => (
            <button
              key={item.id}
              type="button"
              role="tab"
              id={`settings-tab-${item.id}`}
              aria-selected={tab === item.id}
              aria-controls={`settings-tabpanel-${item.id}`}
              className={tab === item.id ? "settings-tab settings-tab-active" : "settings-tab"}
              onClick={() => setTab(item.id)}
            >
              {t(item.labelKey)}
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
              <TalkSection />
            </>
          )}
        </div>

        <div className="dialog-actions">
          <button className="pixel-btn" onClick={closeModal}>
            {t("dialog.close")}
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
      <LanguageItem />
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

/**
 * UI 언어 선택. 항목은 번역 카탈로그에 실제로 있는 언어에서 도출되므로
 * (`availableLanguages()`), 언어를 추가할 때 이 컴포넌트는 고치지 않는다.
 *
 * 저장(`updateAppSettings`)과 적용(`applyLanguageSetting`)을 **둘 다** 부른다:
 * 저장은 다음 부팅용이고, 적용은 지금 화면을 즉시 바꾸기 위한 것이다(재시작
 * 불필요). 부팅 경로에서도 같은 함수를 부르므로 두 경로의 해석 규칙이 하나다.
 */
function LanguageItem() {
  const { t } = useTranslation("settings");
  const language = useAppStore((s) => s.appSettings.language);
  const updateAppSettings = useAppStore((s) => s.updateAppSettings);

  return (
    <label className="settings-item">
      <span>
        <strong>{t("language.title")}</strong>
        <small>{t("language.help")}</small>
      </span>
      <select
        value={language}
        onChange={(e) => {
          const next = e.target.value;
          updateAppSettings({ language: next });
          applyLanguageSetting(next);
        }}
      >
        <option value={LANGUAGE_SYSTEM}>{t("language.system")}</option>
        {availableLanguages().map((l) => (
          <option key={l.code} value={l.code}>
            {l.label}
          </option>
        ))}
      </select>
    </label>
  );
}

/** 요약기 provider별 기본 모델(비우면 이 값이 쓰인다) — 백엔드
 * `summarizer::SummaryPurpose`의 하드코딩 값과 같아야 한다. 안내 문구
 * (placeholder)에만 쓰이므로 어긋나도 동작에는 영향이 없다. */
export const SUMMARY_DEFAULT_MODELS: Record<SummaryProvider, { light: string; heavy: string }> = {
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

/** 서비스 이름은 고유명사라 번역하지 않는다 — 모듈 최상위 상수로 남겨도 된다. */
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
  const { t } = useTranslation("settings");
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
          <Trans t={t} i18nKey="general.openrouterNote" components={{ code: <code /> }} />
        </p>
      )}
      {isOpencode && (
        <p className="settings-note">
          <Trans t={t} i18nKey="general.opencodeNote" components={{ code: <code /> }} />
        </p>
      )}
      {/* 설명 아래 줄에 컨트롤을 둔다 — 나란히 두면 긴 설명이 폭을 다 먹어
          모델 id가 두세 글자만 보였다(kbm #2fc). */}
      <div className="settings-item settings-item-stacked">
        <span>
          <strong>{t("general.lightModelTitle", { provider: SUMMARY_PROVIDER_LABEL[provider] })}</strong>
          <small>
            <Trans
              t={t}
              i18nKey="general.lightModelHelp"
              values={{ model: defaults.light }}
              components={{ code: <code /> }}
            />
          </small>
        </span>
        <ModelPicker
          provider={provider}
          ariaLabel={t("general.lightModelTitle", {
            provider: SUMMARY_PROVIDER_LABEL[provider],
          })}
          placeholder={defaults.light}
          value={current.light}
          onChange={(v) => setModel("light", v)}
        />
      </div>
      <div className="settings-item settings-item-stacked">
        <span>
          <strong>{t("general.heavyModelTitle", { provider: SUMMARY_PROVIDER_LABEL[provider] })}</strong>
          <small>
            <Trans
              t={t}
              i18nKey="general.heavyModelHelp"
              values={{ model: defaults.heavy }}
              components={{ code: <code /> }}
            />
          </small>
        </span>
        <ModelPicker
          provider={provider}
          ariaLabel={t("general.heavyModelTitle", {
            provider: SUMMARY_PROVIDER_LABEL[provider],
          })}
          placeholder={defaults.heavy}
          value={current.heavy}
          onChange={(v) => setModel("heavy", v)}
        />
      </div>
      {isOpenrouter && <OpenrouterSummaryTools />}
    </div>
  );
}

/** 요약 테스트가 실패했을 때 그대로 보여주면 뜻이 통하지 않는 코드들 → 번역 키.
 *  여기 없는 코드는 원문을 보여준다 — 상류 오류는 종류가 열려 있다. */
const SUMMARY_TEST_ERROR_KEY: Record<string, string> = {
  "summarizer-disabled": "general.errorSummarizerDisabled",
  "openrouter-key-missing": "general.errorOpenrouterKeyMissing",
};

/** "있음 / 있음(환경변수) / 없음" — 키 상태 한 조각. 요약 탭과 소리·음성 탭이
 *  같은 저장소를 보므로 문구도 하나로 공유한다. */
function keyStateLabel(
  t: (key: string) => string,
  set: boolean,
  fromEnv: boolean
): string {
  if (!set) return t("keys.absent");
  return fromEnv ? t("keys.presentEnv") : t("keys.present");
}

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
  const { t } = useTranslation("settings");
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
      setNote(t("keys.savedNote"));
    } catch (err) {
      setNote(t("keys.saveFailed", { error: String(err) }));
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
      setNote(t("keys.deletedNote"));
    } catch (err) {
      setNote(t("keys.deleteFailed", { error: String(err) }));
    } finally {
      setBusy(false);
    }
  };

  const test = async () => {
    setBusy(true);
    setNote(null);
    try {
      // 표본은 짧아야 한다 — 크레딧을 쓰는 실제 호출이다.
      const out = await tauriApi.summarizeText(
        "openrouter",
        t("general.summaryTestInstruction"),
        t("general.summaryTestText"),
        "label",
      );
      setNote(t("general.summaryResult", { text: out }));
    } catch (err) {
      const code = String(err);
      const key = SUMMARY_TEST_ERROR_KEY[code];
      setNote(t("general.summaryFailed", { error: key ? t(key) : code }));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="settings-item" style={{ flexDirection: "column", alignItems: "stretch", gap: 8 }}>
      <div style={{ fontSize: 12, opacity: 0.85 }}>
        {status
          ? t("general.openrouterKeyState", {
              state: keyStateLabel(t, status.openrouterSet, status.openrouterFromEnv),
            })
          : t("keys.statusLoading")}
      </div>

      <label className="settings-item">
        <span>
          <strong>{t("general.openrouterKeyTitle")}</strong>
          <small>
            <Trans t={t} i18nKey="general.openrouterKeyHelp" components={{ b: <b /> }} />
          </small>
        </span>
        <input
          type="password"
          autoComplete="off"
          placeholder={status?.openrouterSet ? t("keys.savedPlaceholder") : "sk-or-…"}
          value={apiKey}
          onChange={(e) => setApiKey(e.target.value)}
        />
      </label>

      <div style={{ display: "flex", gap: 8 }}>
        <button className="pixel-btn" disabled={busy || apiKey === ""} onClick={saveKey}>
          {t("keys.save")}
        </button>
        <button className="pixel-btn" disabled={busy} onClick={test}>
          {busy ? t("general.summaryTesting") : t("general.summaryTest")}
        </button>
        {status?.openrouterSet && !status.openrouterFromEnv && (
          <button className="pixel-btn" disabled={busy} onClick={deleteKey}>
            {t("keys.delete")}
          </button>
        )}
      </div>
      {note && <div style={{ fontSize: 12, opacity: 0.85 }}>{note}</div>}
    </div>
  );
}

/** 소리·음성 — 효과음/볼륨/알림 지연 + 대사 읽어주기(TTS). */
function SoundTab() {
  const { t } = useTranslation("settings");
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
            <strong>{t("sound.typingTitle")}</strong>
            <small>{t("sound.typingHelp")}</small>
          </span>
        </label>
        <label className="settings-item">
          <input
            type="checkbox"
            checked={appSettings.notifySoundEnabled}
            onChange={(e) => updateAppSettings({ notifySoundEnabled: e.target.checked })}
          />
          <span>
            <strong>{t("sound.notifyTitle")}</strong>
            <small>{t("sound.notifyHelp")}</small>
          </span>
        </label>
        <label className="settings-item">
          <span>
            <strong>{t("sound.volumeTitle")}</strong>
            <small>{t("sound.volumeHelp")}</small>
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
            <strong>{t("sound.attentionHoldTitle")}</strong>
            <small>{t("sound.attentionHoldHelp")}</small>
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
              {THEMES[id].label}
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

/** 리라이트 경로 라벨 → 사람이 읽는 이름의 **번역 키**. "자동"이 실제로 무엇을
 *  고를지 알려준다. 모듈 최상위라 값이 아니라 키를 담는다(렌더 시점에 번역). */
const REWRITE_VIA_LABEL_KEY: Record<TtsRewriteProvider, string> = {
  auto: "tts.viaAuto",
  api: "tts.viaApi",
  openrouter: "tts.viaOpenrouter",
  "claude-cli": "tts.viaClaudeCli",
  none: "tts.viaNone",
};

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
  const { t } = useTranslation("settings");
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
      setNote(t("keys.savedNote"));
    } catch (err) {
      setNote(t("keys.saveFailed", { error: String(err) }));
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
      setNote(t("keys.deletedNote"));
    } catch (err) {
      setNote(t("keys.deleteFailed", { error: String(err) }));
    } finally {
      setBusy(false);
    }
  };

  const preview = async () => {
    setBusy(true);
    setNote(null);
    try {
      const line = await previewVoice();
      setNote(line ? t("tts.previewSpoken", { line }) : t("tts.previewNone"));
    } catch (err) {
      setNote(t("tts.previewFailed", { error: String(err) }));
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
          <strong>{t("tts.title")}</strong>
          <small>{t("tts.help")}</small>
        </span>
      </label>

      {appSettings.ttsEnabled && (
        <>
          <label className="settings-item">
            <span>
              <strong>{t("tts.rewriteTitle")}</strong>
              <small>
                <Trans t={t} i18nKey="tts.rewriteHelp" components={{ b: <b /> }} />
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
              <option value="auto">{t("tts.rewriteAuto")}</option>
              <option value="api">{t("tts.rewriteApi")}</option>
              <option value="openrouter">OpenRouter</option>
              <option value="claude-cli">{t("tts.rewriteClaudeCli")}</option>
              <option value="none">{t("tts.rewriteNone")}</option>
            </select>
          </label>

          {/* 모델 입력은 공급자에 따라 하나만 보인다 — 지금 쓰이지 않는 쪽을
              같이 띄우면 어느 값이 실제로 쓰이는지 헷갈린다. "자동"과
              "claude CLI"는 Anthropic 모델 id 체계를 쓰므로 같은 칸이다. */}
          {appSettings.ttsRewriteProvider === "openrouter" ? (
            <div className="settings-item settings-item-stacked">
              <span>
                <strong>{t("tts.modelOpenrouterTitle")}</strong>
                <small>
                  <Trans
                    t={t}
                    i18nKey="tts.modelOpenrouterHelp"
                    components={{ code: <code /> }}
                  />
                </small>
              </span>
              <ModelPicker
                provider="openrouter"
                ariaLabel={t("tts.modelOpenrouterTitle")}
                placeholder="openai/gpt-5.4-mini"
                value={appSettings.ttsRewriteModelOpenrouter}
                onChange={(v) => updateAppSettings({ ttsRewriteModelOpenrouter: v })}
              />
            </div>
          ) : (
            appSettings.ttsRewriteProvider !== "none" && (
              <div className="settings-item settings-item-stacked">
                <span>
                  <strong>{t("tts.modelAnthropicTitle")}</strong>
                  <small>{t("tts.modelAnthropicHelp")}</small>
                </span>
                <ModelPicker
                  provider="anthropic"
                  ariaLabel={t("tts.modelAnthropicTitle")}
                  placeholder="claude-haiku-4-5"
                  value={appSettings.ttsRewriteModelAnthropic}
                  onChange={(v) => updateAppSettings({ ttsRewriteModelAnthropic: v })}
                />
              </div>
            )
          )}

          <div
            className="settings-item"
            style={{ flexDirection: "column", alignItems: "stretch", gap: 8 }}
          >
            <div style={{ fontSize: 12, opacity: 0.85 }}>
              {status
                ? t("tts.keyStatus", {
                    elevenlabs: keyStateLabel(
                      t,
                      status.elevenlabsSet,
                      status.elevenlabsFromEnv
                    ),
                    anthropic: keyStateLabel(t, status.anthropicSet, status.anthropicFromEnv),
                    openrouter: keyStateLabel(
                      t,
                      status.openrouterSet,
                      status.openrouterFromEnv
                    ),
                    claudeCli: status.claudeCliAvailable
                      ? t("keys.present")
                      : t("keys.absent"),
                    rewrite: t(REWRITE_VIA_LABEL_KEY[status.effectiveRewriteVia]),
                  })
                : t("keys.statusLoading")}
            </div>

            <label className="settings-item">
              <span>
                <strong>{t("tts.elevenlabsKeyTitle")}</strong>
                <small>
                  <Trans t={t} i18nKey="tts.elevenlabsKeyHelp" components={{ code: <code /> }} />
                </small>
              </span>
              <input
                type="password"
                autoComplete="off"
                placeholder={status?.elevenlabsSet ? t("keys.savedPlaceholder") : "xi-…"}
                value={elevenlabs}
                onChange={(e) => setElevenlabs(e.target.value)}
              />
            </label>
            {status?.elevenlabsSet && !status.elevenlabsFromEnv && (
              <div style={{ display: "flex", justifyContent: "flex-end" }}>
                <button className="pixel-btn" disabled={busy} onClick={() => deleteKey("elevenlabs")}>
                  {t("tts.deleteElevenlabs")}
                </button>
              </div>
            )}

            <label className="settings-item">
              <span>
                <strong>{t("tts.anthropicKeyTitle")}</strong>
                <small>
                  <Trans t={t} i18nKey="tts.anthropicKeyHelp" components={{ code: <code /> }} />
                </small>
              </span>
              <input
                type="password"
                autoComplete="off"
                placeholder={status?.anthropicSet ? t("keys.savedPlaceholder") : "sk-ant-…"}
                value={anthropic}
                onChange={(e) => setAnthropic(e.target.value)}
              />
            </label>
            {status?.anthropicSet && !status.anthropicFromEnv && (
              <div style={{ display: "flex", justifyContent: "flex-end" }}>
                <button className="pixel-btn" disabled={busy} onClick={() => deleteKey("anthropic")}>
                  {t("tts.deleteAnthropic")}
                </button>
              </div>
            )}

            <label className="settings-item">
              <span>
                <strong>{t("tts.openrouterKeyTitle")}</strong>
                <small>
                  <Trans
                    t={t}
                    i18nKey="tts.openrouterKeyHelp"
                    components={{ b: <b />, code: <code /> }}
                  />
                </small>
              </span>
              <input
                type="password"
                autoComplete="off"
                placeholder={status?.openrouterSet ? t("keys.savedPlaceholder") : "sk-or-…"}
                value={openrouter}
                onChange={(e) => setOpenrouter(e.target.value)}
              />
            </label>
            {status?.openrouterSet && !status.openrouterFromEnv && (
              <div style={{ display: "flex", justifyContent: "flex-end" }}>
                <button className="pixel-btn" disabled={busy} onClick={() => deleteKey("openrouter")}>
                  {t("tts.deleteOpenrouter")}
                </button>
              </div>
            )}

            <div style={{ display: "flex", gap: 8 }}>
              <button
                className="pixel-btn"
                disabled={busy || (elevenlabs === "" && anthropic === "" && openrouter === "")}
                onClick={saveKeys}
              >
                {t("keys.save")}
              </button>
              <button className="pixel-btn" disabled={busy} onClick={preview}>
                {t("tts.preview")}
              </button>
            </div>
            {muted && (
              <div style={{ fontSize: 12, opacity: 0.85 }}>{t("tts.mutedNote")}</div>
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
  const { t } = useTranslation("settings");
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
          <strong>{t("control.title")}</strong>
          <small>
            <Trans t={t} i18nKey="control.help" components={{ b: <b />, code: <code /> }} />
          </small>
        </span>
      </label>

      {enabled && (
        <div className="settings-item" style={{ flexDirection: "column", alignItems: "stretch", gap: 8 }}>
          <div style={{ fontSize: 12, opacity: 0.85 }}>
            {t("control.statusLabel")}{" "}
            {status
              ? t("control.statusLine", {
                  server: status.running
                    ? t("control.serverRunning", { port: status.port ?? "?" })
                    : t("control.serverStopped"),
                  approval: status.approved ? t("control.approved") : t("control.unapproved"),
                })
              : t("control.loading")}
          </div>

          {status && !status.approved && (
            <button className="pixel-btn" disabled={busy} onClick={approve}>
              {t("control.approve")}
            </button>
          )}
          {status && status.approved && (
            <>
              <button className="pixel-btn" disabled={busy} onClick={revoke}>
                {t("control.revoke")}
              </button>
              <div style={{ fontSize: 12, opacity: 0.85 }}>
                <div style={{ marginBottom: 4 }}>{t("control.usageIntro")}</div>
                <code style={{ display: "block", whiteSpace: "pre-wrap" }}>
                  agent-office ctl status{"\n"}
                  agent-office ctl list{"\n"}
                  agent-office ctl send &lt;agentId&gt; "npm test" --enter
                </code>
                <div style={{ marginTop: 6, opacity: 0.7 }}>
                  {t("control.appDataNote")} <code>{status.appDataDir}</code>
                </div>
              </div>
            </>
          )}
        </div>
      )}
    </div>
  );
}
