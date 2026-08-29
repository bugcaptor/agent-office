// src/renderer/settings/SummarySection.tsx
//
// 일반 탭의 요약기 설정 — provider별 실행 명령·경량/고급 모델 오버라이드,
// 어느 provider에서든 눌러 볼 수 있는 응답 테스트, 그리고 OpenRouter를
// 골랐을 때만 필요한 API 키 입력.
import { useCallback, useEffect, useState } from "react";
import { Trans, useTranslation } from "react-i18next";
import { useAppStore } from "../store/appStore";
import { tauriApi } from "../ipc/tauriApi";
import { backendErrorText } from "../shared/backendError";
import { ModelPicker } from "./ModelPicker";
import { keyStateLabel } from "./keyStatus";
import type { SummaryProvider, TtsStatus } from "@shared/types";

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

/** 요약기 provider별 기본 실행 명령(비우면 이 이름을 부른다) — 백엔드
 * `summarizer::resolve_command`가 쓰는 `SummaryProvider::as_str()`과 같아야
 * 한다. OpenRouter는 CLI가 아니라 HTTP라 명령이 없다(`null`). */
export const SUMMARY_DEFAULT_COMMANDS: Record<SummaryProvider, string | null> = {
  claude: "claude",
  codex: "codex",
  agy: "agy",
  gemini: "gemini",
  opencode: "opencode",
  openrouter: null,
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
export function SummaryModelSection() {
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
  const defaultCommand = SUMMARY_DEFAULT_COMMANDS[provider];
  const setField = (key: "light" | "heavy" | "command", value: string) =>
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
      {/* 실행 명령은 모델보다 앞이다 — 어떤 바이너리를 부르는지가 먼저 정해지고
          그 다음이 그 바이너리에게 줄 모델 id다. OpenRouter는 HTTP라 칸이 없다. */}
      {defaultCommand !== null && (
        <label className="settings-item settings-item-stacked">
          <span>
            <strong>
              {t("general.commandTitle", { provider: SUMMARY_PROVIDER_LABEL[provider] })}
            </strong>
            <small>
              <Trans
                t={t}
                i18nKey="general.commandHelp"
                values={{ command: defaultCommand }}
                components={{ code: <code /> }}
              />
            </small>
          </span>
          <input
            type="text"
            autoComplete="off"
            spellCheck={false}
            placeholder={defaultCommand}
            value={current.command}
            onChange={(e) => setField("command", e.target.value)}
          />
        </label>
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
          onChange={(v) => setField("light", v)}
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
          onChange={(v) => setField("heavy", v)}
        />
      </div>
      {isOpenrouter && <OpenrouterKeyTools />}
      <SummaryTestTools provider={provider} />
    </div>
  );
}

/** 요약 테스트가 실패했을 때 그대로 보여주면 뜻이 통하지 않는 코드들 → 번역 키.
 *  이 화면 전용 안내라 공통 매핑(`BACKEND_ERROR_KEY`)이 아니라 여기 둔다 —
 *  여기에도 공통에도 없는 코드는 원문을 보여준다(상류 오류는 종류가 열려 있다).
 *  키는 `settings` 네임스페이스라 접두사가 없다(`t`의 기본 ns).
 *
 *  `<provider>-not-found`는 provider마다 문자열이 달라(`claude-not-found`,
 *  `codex-not-found`, …) 여기 열거하지 않고 호출부가 그때그때 얹는다. */
type SummaryTestErrorCode = "summarizer-disabled" | "openrouter-key-missing";

const SUMMARY_TEST_ERROR_KEY: Record<SummaryTestErrorCode, string> = {
  "summarizer-disabled": "settings:general.errorSummarizerDisabled",
  "openrouter-key-missing": "settings:general.errorOpenrouterKeyMissing",
};

/**
 * 지금 고른 provider로 짧은 요약을 한 번 돌려 보는 버튼.
 *
 * provider를 가리지 않는다 — 실행 명령이나 모델 id를 고쳐 놓고 그게 실제로
 * 도는지 확인할 자리가 필요한 것은 OpenRouter만이 아니다. 전용 커맨드가
 * 아니라 `summarizeText`(라벨 목적)를 그대로 타므로, 여기서 성공하면 실제
 * 라벨 요약도 같은 명령·같은 경량 모델로 성공한다는 뜻이 된다.
 *
 * 실패는 저장해 둔 설정 그대로의 실패다. 흔한 것 둘은 문구로 바꿔 준다:
 * 요약 기능이 꺼져 있음(`summarizer-disabled`)과 실행 명령을 못 찾음
 * (`<provider>-not-found`). 나머지는 CLI stderr 원문이 가장 정보가 많다.
 */
function SummaryTestTools({ provider }: { provider: SummaryProvider }) {
  const { t } = useTranslation("settings");
  const [busy, setBusy] = useState(false);
  const [note, setNote] = useState<string | null>(null);

  const test = async () => {
    setBusy(true);
    setNote(null);
    try {
      // 표본은 짧아야 한다 — 구독·크레딧을 쓰는 실제 호출이다.
      const out = await tauriApi.summarizeText(
        provider,
        t("general.summaryTestInstruction"),
        t("general.summaryTestText"),
        "label",
      );
      setNote(t("general.summaryResult", { text: out }));
    } catch (err) {
      setNote(
        t("general.summaryFailed", {
          error: backendErrorText(err, {
            ...SUMMARY_TEST_ERROR_KEY,
            [`${provider}-not-found`]: "settings:general.errorCommandNotFound",
          }),
        })
      );
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="settings-item" style={{ flexDirection: "column", alignItems: "stretch", gap: 8 }}>
      <div style={{ display: "flex", gap: 8 }}>
        <button className="pixel-btn" disabled={busy} onClick={test}>
          {busy ? t("general.summaryTesting") : t("general.summaryTest")}
        </button>
      </div>
      {note && <div style={{ fontSize: 12, opacity: 0.85 }}>{note}</div>}
    </div>
  );
}

/**
 * OpenRouter 요약을 위한 키 입력.
 *
 * 키는 **소리·음성 탭과 같은 0600 저장소**를 그대로 쓴다(`ttsSetKeys`의 셋째
 * 칸). 요약 전용 키를 따로 두면 같은 키를 두 번 넣게 되고 어느 쪽이 실제로
 * 쓰이는지 알 수 없게 된다 — 백엔드도 키를 하나만 읽는다.
 *
 * 응답 테스트는 여기가 아니라 `SummaryTestTools`가 맡는다(모든 provider 공통).
 */
function OpenrouterKeyTools() {
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
      setNote(t("keys.saveFailed", { error: backendErrorText(err) }));
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
      setNote(t("keys.deleteFailed", { error: backendErrorText(err) }));
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
