// src/renderer/settings/WebDashSection.tsx
//
// 설정 다이얼로그 "제어" 탭의 웹 상태판 섹션. 켜기·네트워크·포트·이미지 보이기를 고르고 접속 주소(토큰 포함)를
// 복사한다. 설정은 앱 설정과 따로 `web-dash.json`에 저장된다.
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import type { WebDashBind, WebDashConfig } from "@shared/types";
import { tauriApi } from "../ipc/tauriApi";
import { setWebDashStatus, useWebDashStore } from "../webdash/webDashStore";
import "../webdash/webDash.css";

const BINDS: WebDashBind[] = ["tailnet", "lan", "both"];

export function WebDashSection() {
  const { t } = useTranslation("settings");
  const status = useWebDashStore((s) => s.status);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [port, setPort] = useState("");
  const [copied, setCopied] = useState<string | null>(null);

  useEffect(() => {
    void tauriApi.webDashStatus().then(setWebDashStatus).catch(() => {});
  }, []);
  useEffect(() => {
    if (status) setPort(String(status.config.port));
  }, [status?.config.port]);

  if (!status) return null;
  const config = status.config;

  const apply = async (patch: Partial<WebDashConfig>) => {
    setBusy(true);
    setError(null);
    try {
      setWebDashStatus(await tauriApi.webDashConfigure({ ...config, ...patch }));
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setBusy(false);
    }
  };
  const rotate = async () => {
    setBusy(true);
    try {
      setWebDashStatus(await tauriApi.webDashRotateToken());
    } finally {
      setBusy(false);
    }
  };
  const copy = (url: string) => {
    void navigator.clipboard.writeText(url).then(() => {
      setCopied(url);
      setTimeout(() => setCopied((c) => (c === url ? null : c)), 1500);
    });
  };
  const commitPort = () => {
    const n = Number(port);
    if (Number.isInteger(n) && n >= 1024 && n <= 65535 && n !== config.port) void apply({ port: n });
    else setPort(String(config.port));
  };
  const problem = error ?? status.error;

  return (
    <div className="settings-form">
      <label className="settings-item">
        <input type="checkbox" checked={config.enabled} disabled={busy} onChange={(e) => void apply({ enabled: e.target.checked })} />
        <span>
          <strong>{t("webDash.title")}</strong>
          <small>{t("webDash.help")}</small>
        </span>
      </label>
      {config.enabled && (
        <div className="settings-item" style={{ flexDirection: "column", alignItems: "stretch", gap: 8 }}>
          <label style={{ display: "flex", gap: 8, alignItems: "center" }}>
            <span style={{ minWidth: 80 }}>{t("webDash.bind")}</span>
            <select value={config.bind} disabled={busy} onChange={(e) => void apply({ bind: e.target.value as WebDashBind })}>
              {BINDS.map((b) => (
                <option key={b} value={b}>{t(`webDash.binds.${b}`)}</option>
              ))}
            </select>
          </label>
          <label style={{ display: "flex", gap: 8, alignItems: "center" }}>
            <span style={{ minWidth: 80 }}>{t("webDash.port")}</span>
            <input
              type="number"
              min={1024}
              max={65535}
              value={port}
              disabled={busy}
              style={{ width: 100 }}
              onChange={(e) => setPort(e.target.value)}
              onBlur={commitPort}
              onKeyDown={(e) => { if (e.key === "Enter") commitPort(); }}
            />
          </label>
          <label style={{ display: "flex", gap: 8, alignItems: "flex-start" }}>
            <input type="checkbox" checked={config.images !== false} disabled={busy} onChange={(e) => void apply({ images: e.target.checked })} />
            <span>
              <span>{t("webDash.images")}</span>
              <small style={{ display: "block", opacity: 0.7 }}>{t("webDash.imagesHelp")}</small>
            </span>
          </label>
          <div style={{ fontSize: 12, opacity: 0.85 }}>
            {status.running ? t("webDash.running", { count: status.clients }) : t("webDash.stopped")}
          </div>
          {problem && <div style={{ fontSize: 12, color: "var(--accent-warn)" }}>{t(`webDash.errors.${problem.split(":")[0]}`, { defaultValue: problem })}</div>}
          {status.httpsError && (
            <div style={{ fontSize: 12, opacity: 0.85 }}>
              {t(`webDash.errors.${status.httpsError.split(":")[0]}`, { defaultValue: status.httpsError })}
            </div>
          )}
          {status.urls.length > 0 && (
            <div className="web-dash-urls">
              <div style={{ fontSize: 12, opacity: 0.85 }}>{t("webDash.urlsIntro")}</div>
              {status.urls.map((url) => (
                <div key={url} className="web-dash-url">
                  <code title={url}>{url.replace(/#t=.*/, "")}</code>
                  <button className="pixel-btn" onClick={() => copy(url)}>
                    {copied === url ? t("webDash.copied") : t("webDash.copy")}
                  </button>
                </div>
              ))}
            </div>
          )}
          <button className="pixel-btn" disabled={busy} onClick={() => void rotate()}>
            {t("webDash.rotate")}
          </button>
          <small style={{ opacity: 0.7 }}>{t("webDash.rotateHelp")}</small>
        </div>
      )}
    </div>
  );
}
