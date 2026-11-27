// src/renderer/settings/ShellAssistSection.tsx
//
// 설정 다이얼로그 "셸 캐릭터" 탭. 말 → 명령 제안에 쓸 로컬 LLM(OpenAI 호환 엔드포인트)을
// 순서대로 넣고, 모두 실패했을 때 클라우드로 넘어갈지(끌 수 있음)와 어느 provider를 쓸지 고른다.
// 값은 앱 설정과 따로 `shell-assist.json`(0600)에 저장된다 — 저장 버튼으로 한 번에 쓴다.
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import type { ShellAssistConfig, ShellAssistEndpoint, SummaryProvider } from "@shared/types";
import { tauriApi } from "../ipc/tauriApi";
import { SUMMARY_DEFAULT_MODELS, SUMMARY_PROVIDER_LABEL } from "./SummarySection";

const PROVIDERS = Object.keys(SUMMARY_PROVIDER_LABEL) as SummaryProvider[];
const MAX_ENDPOINTS = 8;

function emptyEndpoint(): ShellAssistEndpoint {
  return { label: "", baseUrl: "", model: "", apiKey: "", enabled: true };
}

type Probe = { busy: boolean; models?: string[]; error?: string };

export function ShellAssistSection() {
  const { t } = useTranslation("settings");
  const [config, setConfig] = useState<ShellAssistConfig | null>(null);
  const [dirty, setDirty] = useState(false);
  const [saving, setSaving] = useState(false);
  const [message, setMessage] = useState<string | null>(null);
  const [probes, setProbes] = useState<Record<number, Probe>>({});

  useEffect(() => {
    void tauriApi.shellAssistGetConfig().then(setConfig).catch((e) => setMessage(String(e)));
  }, []);

  if (!config) return message ? <p className="settings-note">{message}</p> : null;

  const patch = (next: Partial<ShellAssistConfig>) => {
    setConfig({ ...config, ...next });
    setDirty(true);
    setMessage(null);
  };
  const setEndpoint = (i: number, next: Partial<ShellAssistEndpoint>) =>
    patch({ endpoints: config.endpoints.map((e, j) => (j === i ? { ...e, ...next } : e)) });
  const move = (i: number, d: -1 | 1) => {
    const list = [...config.endpoints];
    const j = i + d;
    if (j < 0 || j >= list.length) return;
    [list[i], list[j]] = [list[j], list[i]];
    patch({ endpoints: list });
    setProbes({});
  };
  const remove = (i: number) => {
    patch({ endpoints: config.endpoints.filter((_, j) => j !== i) });
    setProbes({});
  };
  const probe = async (i: number) => {
    setProbes((p) => ({ ...p, [i]: { busy: true } }));
    try {
      const models = await tauriApi.shellAssistListModels(config.endpoints[i]);
      setProbes((p) => ({ ...p, [i]: { busy: false, models } }));
    } catch (e) {
      setProbes((p) => ({ ...p, [i]: { busy: false, error: String(e) } }));
    }
  };
  const save = async () => {
    setSaving(true);
    try {
      setConfig(await tauriApi.shellAssistSetConfig(config));
      setDirty(false);
      setMessage(t("shellAssist.saved"));
    } catch (e) {
      setMessage(String(e));
    } finally {
      setSaving(false);
    }
  };

  return (
    <div className="settings-form">
      <p className="settings-note">{t("shellAssist.intro")}</p>
      <p className="settings-note">{t("shellAssist.examples")}</p>
      {config.endpoints.map((ep, i) => {
        const p = probes[i];
        const listId = `shell-assist-models-${i}`;
        return (
          <fieldset key={i} className="shell-assist-endpoint">
            <legend>{t("shellAssist.endpointN", { n: i + 1 })}</legend>
            <label className="settings-item">
              <input type="checkbox" checked={ep.enabled} onChange={(e) => setEndpoint(i, { enabled: e.target.checked })} />
              <span><strong>{t("shellAssist.enabled")}</strong></span>
            </label>
            <label className="settings-item settings-item-stacked">
              <span><strong>{t("shellAssist.label")}</strong></span>
              <input type="text" value={ep.label} placeholder={t("shellAssist.labelPlaceholder")} onChange={(e) => setEndpoint(i, { label: e.target.value })} />
            </label>
            <label className="settings-item settings-item-stacked">
              <span><strong>{t("shellAssist.baseUrl")}</strong><small>{t("shellAssist.baseUrlHelp")}</small></span>
              <input type="text" spellCheck={false} autoComplete="off" value={ep.baseUrl} placeholder="http://localhost:11434" onChange={(e) => setEndpoint(i, { baseUrl: e.target.value })} />
            </label>
            <label className="settings-item settings-item-stacked">
              <span><strong>{t("shellAssist.model")}</strong><small>{t("shellAssist.modelHelp")}</small></span>
              <input type="text" spellCheck={false} autoComplete="off" list={listId} value={ep.model} placeholder="qwen2.5-coder:14b" onChange={(e) => setEndpoint(i, { model: e.target.value })} />
              {p?.models && (
                <datalist id={listId}>
                  {p.models.map((m) => <option key={m} value={m} />)}
                </datalist>
              )}
            </label>
            <label className="settings-item settings-item-stacked">
              <span><strong>{t("shellAssist.apiKey")}</strong><small>{t("shellAssist.apiKeyHelp")}</small></span>
              <input type="password" autoComplete="off" value={ep.apiKey} onChange={(e) => setEndpoint(i, { apiKey: e.target.value })} />
            </label>
            <div className="shell-assist-actions">
              <button type="button" className="pixel-btn" disabled={!ep.baseUrl.trim() || p?.busy} onClick={() => void probe(i)}>
                {p?.busy ? t("shellAssist.probing") : t("shellAssist.probe")}
              </button>
              <button type="button" className="pixel-btn" disabled={i === 0} onClick={() => move(i, -1)} aria-label={t("shellAssist.up")}>↑</button>
              <button type="button" className="pixel-btn" disabled={i === config.endpoints.length - 1} onClick={() => move(i, 1)} aria-label={t("shellAssist.down")}>↓</button>
              <button type="button" className="pixel-btn" onClick={() => remove(i)}>{t("shellAssist.remove")}</button>
            </div>
            {p?.models && (
              <p className="settings-note">
                {p.models.length ? t("shellAssist.probeOk", { count: p.models.length, models: p.models.slice(0, 6).join(", ") }) : t("shellAssist.probeEmpty")}
                {ep.model.trim() && p.models.length > 0 && !p.models.includes(ep.model.trim()) && ` ${t("shellAssist.modelMissing")}`}
              </p>
            )}
            {p?.error && <p className="settings-note shell-assist-error">{t("shellAssist.probeFailed", { error: p.error })}</p>}
          </fieldset>
        );
      })}
      {config.endpoints.length < MAX_ENDPOINTS && (
        <div>
          <button type="button" className="pixel-btn" onClick={() => patch({ endpoints: [...config.endpoints, emptyEndpoint()] })}>
            {t("shellAssist.add")}
          </button>
        </div>
      )}
      <label className="settings-item">
        <input type="checkbox" checked={config.explain} onChange={(e) => patch({ explain: e.target.checked })} />
        <span><strong>{t("shellAssist.explain")}</strong><small>{t("shellAssist.explainHelp")}</small></span>
      </label>
      <label className="settings-item">
        <input type="checkbox" checked={config.fallbackEnabled} onChange={(e) => patch({ fallbackEnabled: e.target.checked })} />
        <span><strong>{t("shellAssist.fallback")}</strong><small>{t("shellAssist.fallbackHelp")}</small></span>
      </label>
      {config.fallbackEnabled && (
        <>
          <label className="settings-item">
            <span><strong>{t("shellAssist.fallbackProvider")}</strong></span>
            <select value={config.fallbackProvider} onChange={(e) => patch({ fallbackProvider: e.target.value as SummaryProvider })}>
              {PROVIDERS.map((p) => <option key={p} value={p}>{SUMMARY_PROVIDER_LABEL[p]}</option>)}
            </select>
          </label>
          <label className="settings-item settings-item-stacked">
            <span><strong>{t("shellAssist.fallbackModel")}</strong><small>{t("shellAssist.fallbackModelHelp")}</small></span>
            <input type="text" spellCheck={false} autoComplete="off" value={config.fallbackModel} placeholder={SUMMARY_DEFAULT_MODELS[config.fallbackProvider].light} onChange={(e) => patch({ fallbackModel: e.target.value })} />
          </label>
          {config.fallbackProvider === "openrouter" && <p className="settings-note">{t("shellAssist.openrouterKey")}</p>}
        </>
      )}
      <p className="settings-note">{t("shellAssist.profileHint")}</p>
      <div className="shell-assist-actions">
        <button type="button" className="pixel-btn" disabled={!dirty || saving} onClick={() => void save()}>
          {saving ? t("shellAssist.saving") : t("shellAssist.save")}
        </button>
        {message && <span className="settings-note">{message}</span>}
      </div>
    </div>
  );
}
