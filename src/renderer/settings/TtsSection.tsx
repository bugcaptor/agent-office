// src/renderer/settings/TtsSection.tsx
//
// 확인 요청 대사 읽어주기(TTS) 설정 — 리라이트 경로·모델과 API 키 3종.
// 소리·음성 탭(SoundTab)이 통째로 끼워 넣는다.
import { useCallback, useEffect, useState } from "react";
import { Trans, useTranslation } from "react-i18next";
import { useAppStore } from "../store/appStore";
import { tauriApi } from "../ipc/tauriApi";
import { backendErrorText } from "../shared/backendError";
import { ModelPicker } from "./ModelPicker";
import { previewVoice } from "../sound/soundManager";
import { keyStateLabel } from "./keyStatus";
import type { TtsRewriteProvider, TtsStatus } from "@shared/types";

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
export function TtsSection() {
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
      setNote(t("keys.saveFailed", { error: backendErrorText(err) }));
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
      setNote(t("keys.deleteFailed", { error: backendErrorText(err) }));
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
      setNote(t("tts.previewFailed", { error: backendErrorText(err) }));
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
