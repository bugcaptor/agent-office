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
import { useAppStore } from "../store/appStore";
import { webRemoteApi } from "../ipc/webRemoteApi";
import type {
  WebRemoteStatus,
  ClientPermission,
  TailscaleServeStatus,
} from "../ipc/webRemoteApi";
import type { WebRemoteBindPolicy } from "@shared/types";

const BIND_LABEL: Record<WebRemoteBindPolicy, string> = {
  tailnet: "Tailscale 망만 (권장)",
  all: "모든 네트워크 (평문 전송 주의)",
  loopback: "이 컴퓨터만 (사실상 비활성)",
};

/**
 * 접속주소 복사 버튼. 폰으로 옮겨 적기 번거로운 주소(특히 100.x.y.z와
 * MagicDNS 이름)를 그대로 집어 가라고 붙인다. 성공 피드백은 2초 뒤 원복 —
 * 별도 토스트 인프라 없이 버튼 라벨만 바꾼다.
 */
function CopyButton({ text, label }: { text: string; label: string }) {
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
      {copied ? "복사됨" : "복사"}
    </button>
  );
}

export function WebRemoteSection() {
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
  }, [refreshHost, appSettings.webRemoteEnabled]);

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
      .catch((err: unknown) => setServeError(String(err)))
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
          <strong>웹 원격 (브라우저로 접속해서 작업)</strong>
          <small>
            폰이나 다른 컴퓨터의 <b>브라우저</b>로 접속해 상태를 보고 터미널을
            조작합니다. 세션은 <b>이 컴퓨터에서 계속 돌고</b> 출력/입력만
            중계됩니다. 켜도 <b>페어링을 승인</b>해야 붙을 수 있고, 브라우저에는
            <b>정해진 명령만</b> 열립니다(설정 변경·봇 조작은 열리지 않습니다).
            네트워크 표면이므로 기본 꺼짐.
          </small>
        </span>
      </label>

      {appSettings.webRemoteEnabled && (
        <>
          <label className="settings-item">
            <span>
              <strong>허용 네트워크</strong>
              <small>
                Tailscale 망만 허용하면 tailnet(WireGuard)이 암호화를 맡습니다.
                모든 네트워크를 허용하면 같은 LAN에 평문으로 흐릅니다.
              </small>
            </span>
            <select
              value={appSettings.webRemoteBind}
              onChange={(e) =>
                updateAppSettings({ webRemoteBind: e.target.value as WebRemoteBindPolicy })
              }
            >
              {(Object.keys(BIND_LABEL) as WebRemoteBindPolicy[]).map((k) => (
                <option key={k} value={k}>
                  {BIND_LABEL[k]}
                </option>
              ))}
            </select>
          </label>

          <div
            className="settings-item"
            style={{ flexDirection: "column", alignItems: "stretch", gap: 6 }}
          >
            <div style={{ fontSize: 12, opacity: 0.85 }}>
              상태: {host ? (host.running ? `수신 중 (이름: ${host.hostName})` : "정지") : "조회 중…"}
            </div>
            {host && appSettings.webRemoteBind === "tailnet" && !host.tailnetFound && (
              <div style={{ fontSize: 12, color: "var(--accent-warn)" }}>
                <b>Tailscale이 감지되지 않았습니다</b> — 이 컴퓨터에서만 접속할 수
                있게 <code>127.0.0.1</code>에만 열었습니다. Tailscale을 켜고 앱을
                다시 시작하거나, 허용 네트워크를 바꾸세요.
              </div>
            )}
            <div style={{ fontSize: 12, opacity: 0.85 }}>브라우저에서 이 주소로 접속하세요</div>
            <div style={{ display: "flex", gap: 6, alignItems: "center", flexWrap: "wrap" }}>
              <code style={{ fontSize: 13, flex: 1 }}>
                {httpUrl ?? (
                  <>
                    http://&lt;이 컴퓨터 주소&gt;:{host?.port ?? appSettings.webRemotePort}/web/
                  </>
                )}
              </code>
              {httpUrl && <CopyButton text={httpUrl} label="접속 주소 복사" />}
            </div>
            <div style={{ fontSize: 12, opacity: 0.7 }}>
              브라우저에서 <b>연결 요청</b>을 누르면 이 앱에 <b>6자리 코드와 승인
              창</b>이 뜹니다. 그 코드를 브라우저에 입력하세요(설정 창을 닫아 두어도
              뜹니다).
            </div>
            {!host?.running && (
              <div style={{ fontSize: 12, opacity: 0.7 }}>
                서버가 아직 뜨지 않았습니다 — 잠시 후 이 화면을 다시 열어 보세요.
              </div>
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
                  <div style={{ fontSize: 12, opacity: 0.7 }}>조회 중…</div>
                )}
                {serve && !serve.cliFound && (
                  <div style={{ fontSize: 12, opacity: 0.7 }}>
                    Tailscale 명령줄 도구를 찾지 못했습니다 — HTTPS 없이 위 http
                    주소로 접속하세요(tailnet 자체가 암호화합니다). Tailscale 앱의
                    <b> Install CLI</b>를 실행하면 여기서 켤 수 있습니다.
                  </div>
                )}
                {serve?.cliFound && serve.registered && (
                  <>
                    <div style={{ fontSize: 12, opacity: 0.85 }}>
                      HTTPS로 열려 있습니다. 이 주소로 접속하세요
                    </div>
                    <div
                      style={{ display: "flex", gap: 6, alignItems: "center", flexWrap: "wrap" }}
                    >
                      <code style={{ fontSize: 13, flex: 1 }}>{serve.httpsUrl}</code>
                      {serve.httpsUrl && (
                        <CopyButton text={serve.httpsUrl} label="HTTPS 주소 복사" />
                      )}
                    </div>
                    <div style={{ fontSize: 12, opacity: 0.7 }}>
                      이 매핑은 <b>Tailscale이 기억</b>합니다 — 앱을 꺼도, 컴퓨터를
                      다시 켜도 남아 있습니다(앱이 꺼져 있으면 접속만 실패합니다).
                      첫 접속은 인증서 발급으로 몇 초 걸릴 수 있습니다.
                    </div>
                    <div>
                      <button
                        className="pixel-btn"
                        disabled={serveBusy}
                        onClick={() => runServe(() => webRemoteApi.serveDisable())}
                      >
                        HTTPS 끄기
                      </button>
                    </div>
                  </>
                )}
                {serve?.cliFound && !serve.registered && (
                  <>
                    <div style={{ fontSize: 12, opacity: 0.7 }}>
                      Tailscale에 HTTPS 프록시(포트 {serve.httpsPort})를 등록해
                      <code> https://</code> 주소로 접속할 수 있습니다. 브라우저
                      경고 없이 붙고, 쿠키도 <code>Secure</code>로 발급됩니다.
                    </div>
                    {serve.conflict && (
                      <>
                        <div style={{ fontSize: 12, color: "var(--accent-warn)" }}>
                          포트 {serve.httpsPort}을 이미 다른 곳이 쓰고 있습니다
                          ({serve.upstream}). 앱은 남의 설정을 덮어쓰지 않습니다 —
                          그 매핑이 필요 없다면 아래 명령으로 직접 지우세요.
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
                            label="정리 명령 복사"
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
                        HTTPS 켜기
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
                <div style={{ fontSize: 12, opacity: 0.85, marginTop: 6 }}>승인된 브라우저</div>
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
                      <option value="input">입력 허용</option>
                      <option value="readOnly">읽기 전용</option>
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
                      승인 취소
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
