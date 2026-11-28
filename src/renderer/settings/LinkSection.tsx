// src/renderer/settings/LinkSection.tsx
//
// 설정 다이얼로그 "머신 연동" 탭. 끔·호스트·클라이언트 중 하나를 고른다(자동 발견 없음).
// 호스트: 페어링 코드를 띄우고(5분), 페어링된 기기 목록에서 기기별로 해제한다. 클라이언트가 붙는 곳은
// 웹 상태판 서버라 웹 상태판이 켜져 있어야 한다.
// 클라이언트: 호스트 주소와 호스트가 띄운 코드로 페어링하고, 호스트 연결 상태를 본다.
// 설정은 `link.json`, 기기·토큰은 `link-devices.json`·`link-client.json`(0600)에 따로 저장된다.
import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import type { LinkClientStatus, LinkDevice, LinkMode, LinkStatus } from "@shared/types";
import { tauriApi } from "../ipc/tauriApi";
import { linkErrorKey, setLinkStatus, useLinkStore } from "../link/linkStore";
import { setWebDashStatus, useWebDashStore } from "../webdash/webDashStore";
import { setFilesStatus, useFilesStore } from "../files/filesStore";
import "../webdash/webDash.css";

const MODES: LinkMode[] = ["off", "host", "client"];

function useNow(active: boolean): number {
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    if (!active) return;
    const id = setInterval(() => setNow(Date.now()), 1000);
    return () => clearInterval(id);
  }, [active]);
  return now;
}

function formatCode(code: string): string {
  return code.length === 8 ? `${code.slice(0, 4)}-${code.slice(4)}` : code;
}

function formatWhen(ms: number): string {
  return new Date(ms).toLocaleString();
}

export function LinkSection() {
  const { t } = useTranslation("settings");
  const status = useLinkStore((s) => s.status);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    void tauriApi.linkStatus().then(setLinkStatus).catch(() => {});
    void tauriApi.webDashStatus().then(setWebDashStatus).catch(() => {});
  }, []);

  if (!status) return null;

  const run = async (call: () => Promise<LinkStatus>) => {
    setBusy(true);
    setError(null);
    try {
      setLinkStatus(await call());
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="settings-form">
      <div className="settings-item settings-item-stacked" style={{ cursor: "default" }}>
        <span>
          <strong>{t("link.title")}</strong>
          <small>{t("link.help")}</small>
        </span>
        <div role="radiogroup" aria-label={t("link.mode")} style={{ display: "flex", gap: 12, flexWrap: "wrap" }}>
          {MODES.map((mode) => (
            <label key={mode} style={{ display: "flex", gap: 4, alignItems: "center" }}>
              <input
                type="radio"
                name="link-mode"
                checked={status.config.mode === mode}
                disabled={busy}
                onChange={() => void run(() => tauriApi.linkConfigure({ ...status.config, mode }))}
              />
              <span>{t(`link.modes.${mode}`)}</span>
            </label>
          ))}
        </div>
        <small>{t(`link.modeHelp.${status.config.mode}`)}</small>
      </div>
      {error && <LinkError code={error} />}
      {status.host && <HostPanel status={status} busy={busy} run={run} />}
      {status.client && <ClientPanel status={status} client={status.client} busy={busy} run={run} />}
      <FilesReceiveSection />
      <small style={{ opacity: 0.6 }}>{t("link.version", { protocol: status.protocol, app: status.appVersion })}</small>
    </div>
  );
}

/** 파일 받기 옵트인과 받은 파일 폴더(`files.json`). 연동이 꺼져 있어도 이 머신 캐릭터끼리 쓰므로 늘 보인다. */
function FilesReceiveSection() {
  const { t } = useTranslation("files");
  const status = useFilesStore((s) => s.status);
  const [error, setError] = useState<string | null>(null);
  useEffect(() => {
    void tauriApi.filesStatus().then(setFilesStatus).catch(() => {});
  }, []);
  if (!status) return null;
  const save = async (config: typeof status.config) => {
    setError(null);
    try {
      setFilesStatus(await tauriApi.filesConfigure(config));
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    }
  };
  const pick = async () => {
    const dir = await tauriApi.pickDirectory(status.receiveDir).catch(() => null);
    if (dir) await save({ ...status.config, receiveDir: dir });
  };
  return (
    <div className="settings-item settings-item-stacked" style={{ cursor: "default", marginTop: 8, paddingTop: 8, borderTop: "1px dashed var(--border-dark)" }}>
      <label style={{ display: "flex", gap: 8, alignItems: "flex-start" }}>
        <input
          type="checkbox"
          checked={status.config.receiveEnabled}
          onChange={(e) => void save({ ...status.config, receiveEnabled: e.target.checked })}
        />
        <span>
          <strong>{t("settings.title")}</strong>
          <small>{t("settings.help")}</small>
        </span>
      </label>
      <small>{t("settings.dir")}</small>
      <div style={{ display: "flex", gap: 8, alignItems: "center", flexWrap: "wrap" }}>
        <code style={{ userSelect: "text", wordBreak: "break-all" }}>{status.receiveDir}</code>
        {!status.config.receiveDir && <small>({t("settings.default")})</small>}
      </div>
      <div style={{ display: "flex", gap: 8, flexWrap: "wrap" }}>
        <button className="pixel-btn" onClick={() => void pick()}>{t("settings.change")}</button>
        {status.config.receiveDir && (
          <button className="pixel-btn" onClick={() => void save({ ...status.config, receiveDir: undefined })}>{t("settings.reset")}</button>
        )}
        <button className="pixel-btn" onClick={() => void tauriApi.filesReveal().catch(() => {})}>{t("settings.open")}</button>
      </div>
      {error && <div className="link-error" role="alert">{t(`errors.${error}`, { defaultValue: error, detail: "" })}</div>}
    </div>
  );
}

function LinkError({ code }: { code: string }) {
  const { t } = useTranslation("settings");
  const { key, detail } = linkErrorKey(code);
  return (
    <div className="link-error" role="alert">
      {t(`link.errors.${key}`, { defaultValue: code, detail, code: detail })}
      {detail && key !== "link-http" && <small style={{ display: "block", opacity: 0.7 }}>{detail}</small>}
    </div>
  );
}

type Run = (call: () => Promise<LinkStatus>) => Promise<void>;

function HostPanel({ status, busy, run }: { status: LinkStatus; busy: boolean; run: Run }) {
  const { t } = useTranslation("settings");
  const webDash = useWebDashStore((s) => s.status);
  const host = status.host!;
  const now = useNow(!!host.pairing);
  const remaining = host.pairing ? Math.max(0, Math.round((host.pairing.expiresAt - now) / 1000)) : 0;
  const addresses = (webDash?.running ? webDash.urls : [])
    .map((url) => url.replace(/\/?#t=.*$/, ""))
    .filter((url) => !/\/\/127\./.test(url));

  return (
    <div className="settings-item settings-item-stacked" style={{ cursor: "default" }}>
      {!webDash?.running && <div className="link-error">{t("link.host.needsWebDash")}</div>}
      {addresses.length > 0 && (
        <div className="web-dash-urls">
          <small>{t("link.host.addresses")}</small>
          {addresses.map((url) => (
            <div key={url} className="web-dash-url">
              <code style={{ userSelect: "text" }}>{url}</code>
            </div>
          ))}
        </div>
      )}
      <div className="link-pairing">
        {host.pairing && remaining > 0 ? (
          <>
            <small>{t("link.host.codeIntro")}</small>
            <code className="link-code" aria-label={t("link.host.code")}>{formatCode(host.pairing.code)}</code>
            <small>{t("link.host.codeRemaining", { minutes: Math.floor(remaining / 60), seconds: String(remaining % 60).padStart(2, "0") })}</small>
            <button className="pixel-btn" disabled={busy} onClick={() => void run(() => tauriApi.linkCancelPairing())}>
              {t("link.host.cancelCode")}
            </button>
          </>
        ) : (
          <button className="pixel-btn" disabled={busy} onClick={() => void run(() => tauriApi.linkStartPairing())}>
            {t("link.host.newCode")}
          </button>
        )}
      </div>
      <strong style={{ marginTop: 8 }}>{t("link.host.devices")}</strong>
      {host.devices.length === 0 ? (
        <small>{t("link.host.noDevices")}</small>
      ) : (
        <ul className="link-devices">
          {host.devices.map((d) => (
            <DeviceRow key={d.id} device={d} busy={busy} onRevoke={() => void run(() => tauriApi.linkRevokeDevice(d.id))} />
          ))}
        </ul>
      )}
    </div>
  );
}

function DeviceRow({ device, busy, onRevoke }: { device: LinkDevice; busy: boolean; onRevoke: () => void }) {
  const { t } = useTranslation("settings");
  const [confirming, setConfirming] = useState(false);
  const state = device.connected
    ? t("link.host.connected", { address: device.address ?? "" })
    : device.lastSeenAt
      ? t("link.host.lastSeen", { when: formatWhen(device.lastSeenAt) })
      : t("link.host.neverSeen");
  return (
    <li className="link-device">
      <span className={device.connected ? "link-dot link-dot-on" : "link-dot"} aria-hidden />
      <span style={{ flex: 1, minWidth: 0 }}>
        <span>{device.name || device.id}</span>
        <small style={{ display: "block" }}>
          {state}
          {device.appVersion ? ` · v${device.appVersion}` : ""}
        </small>
      </span>
      {confirming ? (
        <>
          <button className="pixel-btn" disabled={busy} onClick={onRevoke}>{t("link.host.revokeConfirm")}</button>
          <button className="pixel-btn" disabled={busy} onClick={() => setConfirming(false)}>{t("link.cancel")}</button>
        </>
      ) : (
        <button className="pixel-btn" disabled={busy} onClick={() => setConfirming(true)}>{t("link.host.revoke")}</button>
      )}
    </li>
  );
}

function ClientPanel({ status, client, busy, run }: { status: LinkStatus; client: LinkClientStatus; busy: boolean; run: Run }) {
  const { t } = useTranslation("settings");
  const [pairing, setPairing] = useState(!client.paired);
  const [address, setAddress] = useState(status.config.hostAddress);
  const [code, setCode] = useState("");
  const now = useNow(client.state === "waiting");
  useEffect(() => {
    if (!client.paired) setPairing(true);
  }, [client.paired]);

  const pair = async () => {
    await run(async () => {
      const next = await tauriApi.linkPair(address, code);
      setPairing(false);
      setCode("");
      return next;
    });
  };
  const mismatch = client.error === "link-protocol-mismatch" && client.hostProtocol !== undefined;
  const retryIn = client.retryAt ? Math.max(0, Math.round((client.retryAt - now) / 1000)) : 0;

  return (
    <div className="settings-item settings-item-stacked" style={{ cursor: "default" }}>
      {client.paired && (
        <div className="link-client-state">
          <span className={client.state === "connected" ? "link-dot link-dot-on" : "link-dot"} aria-hidden />
          <span style={{ flex: 1, minWidth: 0 }}>
            <strong>{client.hostName ?? client.address}</strong>
            <small style={{ display: "block" }}>
              {t(`link.client.states.${client.state}`, { seconds: retryIn })}
              {client.address ? ` · ${client.address}` : ""}
              {client.pending > 0 ? ` · ${t("link.client.pending", { count: client.pending })}` : ""}
            </small>
          </span>
        </div>
      )}
      {client.paired && mismatch && (
        <div className="link-error">
          {t(client.hostProtocol! > status.protocol ? "link.client.updateThis" : "link.client.updateHost", {
            host: client.hostAppVersion ?? "?",
            mine: status.appVersion,
          })}
        </div>
      )}
      {client.paired && client.error && !mismatch && client.state !== "connected" && <LinkError code={client.error} />}
      {client.pinnedFingerprint && (
        <small>
          {t("link.client.pinned")}
          <code style={{ display: "block", overflowWrap: "anywhere", userSelect: "text" }}>{client.pinnedFingerprint}</code>
        </small>
      )}
      {pairing ? (
        <div className="link-pair-form">
          <label className="settings-item settings-item-stacked">
            <span>{t("link.client.address")}</span>
            <input
              type="text"
              value={address}
              placeholder="https://192.168.0.5:7788"
              disabled={busy}
              onChange={(e) => setAddress(e.target.value)}
            />
            <small>{t("link.client.addressHelp")}</small>
          </label>
          <label className="settings-item settings-item-stacked">
            <span>{t("link.client.code")}</span>
            <input
              type="text"
              value={code}
              placeholder="ABCD-EFGH"
              autoCapitalize="characters"
              spellCheck={false}
              disabled={busy}
              onChange={(e) => setCode(e.target.value)}
              onKeyDown={(e) => { if (e.key === "Enter" && address.trim() && code.trim()) void pair(); }}
            />
          </label>
          <div className="shell-assist-actions">
            <button className="pixel-btn" disabled={busy || !address.trim() || !code.trim()} onClick={() => void pair()}>
              {busy ? t("link.client.pairing") : t("link.client.pair")}
            </button>
            {client.paired && (
              <button className="pixel-btn" disabled={busy} onClick={() => setPairing(false)}>{t("link.cancel")}</button>
            )}
          </div>
        </div>
      ) : (
        <div className="shell-assist-actions">
          <button className="pixel-btn" disabled={busy} onClick={() => setPairing(true)}>{t("link.client.repair")}</button>
          <button className="pixel-btn" disabled={busy} onClick={() => void run(() => tauriApi.linkForgetHost())}>{t("link.client.forget")}</button>
        </div>
      )}
      {!pairing && <small>{t("link.client.forgetHelp")}</small>}
    </div>
  );
}
