// src/renderer/settings/WebRemoteSection.tsx
//
// 웹 원격(docs/web-remote-design.md) 설정 UI. 앱은 호스트 역할만 한다 —
// 폰이나 다른 컴퓨터의 **브라우저**가 이 앱에 붙어 상태를 보고 터미널에
// 개입한다. 앱↔앱 접속은 범위 밖이라 뷰어 UI가 없다.
//
// 보안 흐름은 CLI 제어와 같은 2단계 옵트인이다: 토글로 서버를 켜도 페어링
// 승인 전에는 모든 요청이 401이고, 브라우저에 열리는 것은 allowlist에 등재된
// 명령뿐이다(설정 변경·봇 조작은 열리지 않는다).

import { useCallback, useEffect, useState } from "react";
import { Trans, useTranslation } from "react-i18next";
import { useAppStore } from "../store/appStore";
import { webRemoteApi } from "../ipc/webRemoteApi";
import { backendErrorText } from "../shared/backendError";
import type {
  WebRemoteStatus,
  ClientPermission,
  TailscaleServeStatus,
} from "../ipc/webRemoteApi";
import type { WebRemoteBindPolicy } from "@shared/types";

/** 드롭다운 순서와 각 항목의 번역 키. 모듈 최상위라 `t()`를 부를 수 없어
 *  값이 아니라 **키**를 담는다 — 언어를 바꾸면 렌더 시점에 다시 번역된다. */
const BIND_ORDER: WebRemoteBindPolicy[] = ["tailnet", "all", "loopback"];
const BIND_LABEL_KEY: Record<WebRemoteBindPolicy, string> = {
  tailnet: "webRemote.bindTailnet",
  all: "webRemote.bindAll",
  loopback: "webRemote.bindLoopback",
};

/**
 * 접속주소 복사 버튼. 폰으로 옮겨 적기 번거로운 주소(특히 100.x.y.z와
 * MagicDNS 이름)를 그대로 집어 가라고 붙인다. 성공 피드백은 2초 뒤 원복 —
 * 별도 토스트 인프라 없이 버튼 라벨만 바꾼다.
 */
function CopyButton({ text, label }: { text: string; label: string }) {
  const { t } = useTranslation("settings");
  const [copied, setCopied] = useState(false);

  useEffect(() => {
    if (!copied) return;
    const timer = window.setTimeout(() => setCopied(false), 2000);
    return () => window.clearTimeout(timer);
  }, [copied]);

  return (
    <button
      className="pixel-btn"
      aria-label={label}
      onClick={() => {
        void (async () => {
          try {
            await navigator.clipboard.writeText(text);
            setCopied(true);
          } catch (err) {
            console.warn("WebRemoteSection: clipboard write failed", err);
          }
        })();
      }}
    >
      {copied ? t("webRemote.copied") : t("webRemote.copy")}
    </button>
  );
}

export function WebRemoteSection() {
  const { t } = useTranslation("settings");
  const appSettings = useAppStore((s) => s.appSettings);
  const updateAppSettings = useAppStore((s) => s.updateAppSettings);
  const setWebRemotePending = useAppStore((s) => s.setWebRemotePending);

  const [host, setHost] = useState<WebRemoteStatus | null>(null);
  const [busy, setBusy] = useState(false);
  // tailscale serve 상태는 **tailscaled가 정본**이라 앱 설정에 두지 않고
  // 화면을 열 때마다 조회한다(앱을 꺼도 매핑은 남는다).
  const [serve, setServe] = useState<TailscaleServeStatus | null>(null);
  const [serveBusy, setServeBusy] = useState(false);
  const [serveError, setServeError] = useState<string | null>(null);

  const refreshHost = useCallback(async () => {
    try {
      const status = await webRemoteApi.hostStatus();
      setHost(status);
      setWebRemotePending(status.pending ?? []);
    } catch {
      setHost(null);
    }
  }, [setWebRemotePending]);

  const refreshServe = useCallback(async () => {
    try {
      setServe(await webRemoteApi.serveStatus());
    } catch (err) {
      console.warn("WebRemoteSection: serveStatus failed", err);
      setServe(null);
    }
  }, []);

  useEffect(() => {
    void refreshHost();
    // updateAppSettings 저장은 fire-and-forget이라 바인드/포트 변경의 재바인드가
    // 끝나기 전에 이 effect가 먼저 돌 수 있다 — 잠시 뒤 한 번 더 조회해 새
    // 주소를 붙잡는다.
    const timer = window.setTimeout(() => void refreshHost(), 1200);
    return () => window.clearTimeout(timer);
  }, [
    refreshHost,
    appSettings.webRemoteEnabled,
    appSettings.webRemoteBind,
    appSettings.webRemotePort,
  ]);

  // HTTPS는 tailnet에 실제로 열려 있을 때만 의미가 있다 — 루프백 폴백이나
  // 전 네트워크 바인드에서는 serve 업스트림을 잡을 근거가 없다.
  const serveVisible = appSettings.webRemoteBind === "tailnet" && host?.tailnetFound === true;

  useEffect(() => {
    if (!serveVisible) return;
    void refreshServe();
  }, [serveVisible, refreshServe]);

  const httpUrl = host?.addressHint
    ? `http://${host.addressHint}:${host.port ?? appSettings.webRemotePort}/web/`
    : null;

  const runServe = (action: () => Promise<void>) => {
    setServeBusy(true);
    setServeError(null);
    void action()
      .catch((err: unknown) => setServeError(backendErrorText(err)))
      .then(refreshServe)
      .finally(() => setServeBusy(false));
  };

  return (
    <div className="settings-form">
      <label className="settings-item">
        <input
          type="checkbox"
          checked={appSettings.webRemoteEnabled}
          onChange={(e) => updateAppSettings({ webRemoteEnabled: e.target.checked })}
        />
        <span>
          <strong>{t("webRemote.title")}</strong>
          <small>
            <Trans t={t} i18nKey="webRemote.help" components={{ b: <b /> }} />
          </small>
        </span>
      </label>

      {appSettings.webRemoteEnabled && (
        <>
          <label className="settings-item">
            <span>
              <strong>{t("webRemote.bindTitle")}</strong>
              <small>{t("webRemote.bindHelp")}</small>
            </span>
            <select
              value={appSettings.webRemoteBind}
              onChange={(e) =>
                updateAppSettings({ webRemoteBind: e.target.value as WebRemoteBindPolicy })
              }
            >
              {BIND_ORDER.map((k) => (
                <option key={k} value={k}>
                  {t(BIND_LABEL_KEY[k])}
                </option>
              ))}
            </select>
          </label>

          <div
            className="settings-item"
            style={{ flexDirection: "column", alignItems: "stretch", gap: 6 }}
          >
            <div style={{ fontSize: 12, opacity: 0.85 }}>
              {t("webRemote.statusLabel")}{" "}
              {host
                ? host.running
                  ? t("webRemote.listening", { name: host.hostName })
                  : t("webRemote.stopped")
                : t("webRemote.loading")}
            </div>
            {host && appSettings.webRemoteBind === "tailnet" && !host.tailnetFound && (
              <div style={{ fontSize: 12, color: "var(--accent-warn)" }}>
                <Trans
                  t={t}
                  i18nKey="webRemote.tailnetMissing"
                  components={{ b: <b />, code: <code /> }}
                />
              </div>
            )}
            <div style={{ fontSize: 12, opacity: 0.85 }}>{t("webRemote.urlIntro")}</div>
            <div style={{ display: "flex", gap: 6, alignItems: "center", flexWrap: "wrap" }}>
              <code style={{ fontSize: 13, flex: 1 }}>
                {httpUrl ?? (
                  <>
                    http://&lt;{t("webRemote.hostPlaceholder")}&gt;:
                    {host?.port ?? appSettings.webRemotePort}/web/
                  </>
                )}
              </code>
              {httpUrl && <CopyButton text={httpUrl} label={t("webRemote.copyUrlAria")} />}
            </div>
            <div style={{ fontSize: 12, opacity: 0.7 }}>
              <Trans t={t} i18nKey="webRemote.pairHint" components={{ b: <b /> }} />
            </div>
            {!host?.running && (
              <div style={{ fontSize: 12, opacity: 0.7 }}>{t("webRemote.notRunning")}</div>
            )}

            {serveVisible && (
              <div
                style={{
                  display: "flex",
                  flexDirection: "column",
                  gap: 6,
                  marginTop: 8,
                  paddingTop: 8,
                  borderTop: "1px solid var(--border, rgba(128,128,128,0.3))",
                }}
              >
                <div style={{ fontSize: 12, opacity: 0.85 }}>
                  <strong>HTTPS (tailscale serve)</strong>
                </div>
                {serve === null && (
                  <div style={{ fontSize: 12, opacity: 0.7 }}>{t("webRemote.loading")}</div>
                )}
                {serve && !serve.cliFound && (
                  <div style={{ fontSize: 12, opacity: 0.7 }}>
                    <Trans t={t} i18nKey="webRemote.serveNoCli" components={{ b: <b /> }} />
                  </div>
                )}
                {serve?.cliFound && serve.registered && (
                  <>
                    <div style={{ fontSize: 12, opacity: 0.85 }}>
                      {t("webRemote.serveOnIntro")}
                    </div>
                    <div
                      style={{ display: "flex", gap: 6, alignItems: "center", flexWrap: "wrap" }}
                    >
                      <code style={{ fontSize: 13, flex: 1 }}>{serve.httpsUrl}</code>
                      {serve.httpsUrl && (
                        <CopyButton text={serve.httpsUrl} label={t("webRemote.copyHttpsAria")} />
                      )}
                    </div>
                    <div style={{ fontSize: 12, opacity: 0.7 }}>
                      <Trans t={t} i18nKey="webRemote.serveOnNote" components={{ b: <b /> }} />
                    </div>
                    <div>
                      <button
                        className="pixel-btn"
                        disabled={serveBusy}
                        onClick={() => runServe(() => webRemoteApi.serveDisable())}
                      >
                        {t("webRemote.serveOff")}
                      </button>
                    </div>
                  </>
                )}
                {serve?.cliFound && !serve.registered && (
                  <>
                    <div style={{ fontSize: 12, opacity: 0.7 }}>
                      <Trans
                        t={t}
                        i18nKey="webRemote.serveOffIntro"
                        values={{ port: serve.httpsPort }}
                        components={{ code: <code /> }}
                      />
                    </div>
                    {serve.conflict && (
                      <>
                        <div style={{ fontSize: 12, color: "var(--accent-warn)" }}>
                          {t("webRemote.serveConflict", {
                            port: serve.httpsPort,
                            upstream: serve.upstream,
                          })}
                        </div>
                        {/* 설계의 "최후에는 복사 가능한 명령으로 폴백". 앱 자신이
                            예전에 등록해 둔 낡은 매핑(포트·tailnet IP가 바뀐 경우)도
                            여기로 풀린다 — 버튼으로 대행하면 남의 서비스를 한 번의
                            클릭으로 날릴 수 있어서 사람 손에 맡긴다. */}
                        <div
                          style={{ display: "flex", gap: 6, alignItems: "center", flexWrap: "wrap" }}
                        >
                          <code style={{ fontSize: 12, flex: 1 }}>
                            tailscale serve --https={serve.httpsPort} off
                          </code>
                          <CopyButton
                            text={`tailscale serve --https=${serve.httpsPort} off`}
                            label={t("webRemote.copyCleanupAria")}
                          />
                        </div>
                      </>
                    )}
                    <div>
                      <button
                        className="pixel-btn"
                        disabled={serveBusy || serve.conflict || !host?.running}
                        onClick={() => runServe(() => webRemoteApi.serveEnable())}
                      >
                        {t("webRemote.serveOn")}
                      </button>
                    </div>
                  </>
                )}
                {serveError && (
                  <div style={{ fontSize: 12, color: "var(--accent-warn)" }}>{serveError}</div>
                )}
              </div>
            )}

            {host && host.clients.length > 0 && (
              <>
                <div style={{ fontSize: 12, opacity: 0.85, marginTop: 6 }}>
                  {t("webRemote.clients")}
                </div>
                {host.clients.map((p) => (
                  <div
                    key={p.clientId}
                    style={{ display: "flex", gap: 6, alignItems: "center", flexWrap: "wrap" }}
                  >
                    <span style={{ flex: 1 }}>{p.name}</span>
                    <select
                      value={p.permission}
                      disabled={busy}
                      onChange={(e) => {
                        setBusy(true);
                        void webRemoteApi
                          .setClientPermission(p.clientId, e.target.value as ClientPermission)
                          .then(refreshHost)
                          .finally(() => setBusy(false));
                      }}
                    >
                      <option value="input">{t("webRemote.permInput")}</option>
                      <option value="readOnly">{t("webRemote.permReadOnly")}</option>
                    </select>
                    <button
                      className="pixel-btn"
                      disabled={busy}
                      onClick={() => {
                        setBusy(true);
                        void webRemoteApi
                          .revokeClient(p.clientId)
                          .then(refreshHost)
                          .finally(() => setBusy(false));
                      }}
                    >
                      {t("webRemote.revoke")}
                    </button>
                  </div>
                ))}
              </>
            )}
          </div>
        </>
      )}
    </div>
  );
}
