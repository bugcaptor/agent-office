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
import type { WebRemoteStatus, ClientPermission } from "../ipc/webRemoteApi";
import type { WebRemoteBindPolicy } from "@shared/types";

const BIND_LABEL: Record<WebRemoteBindPolicy, string> = {
  tailnet: "Tailscale 망만 (권장)",
  all: "모든 네트워크 (평문 전송 주의)",
  loopback: "이 컴퓨터만 (사실상 비활성)",
};

export function WebRemoteSection() {
  const appSettings = useAppStore((s) => s.appSettings);
  const updateAppSettings = useAppStore((s) => s.updateAppSettings);
  const setWebRemotePending = useAppStore((s) => s.setWebRemotePending);

  const [host, setHost] = useState<WebRemoteStatus | null>(null);
  const [busy, setBusy] = useState(false);

  const refreshHost = useCallback(async () => {
    try {
      const status = await webRemoteApi.hostStatus();
      setHost(status);
      setWebRemotePending(status.pending ?? []);
    } catch {
      setHost(null);
    }
  }, [setWebRemotePending]);

  useEffect(() => {
    void refreshHost();
  }, [refreshHost, appSettings.webRemoteEnabled]);

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
            <div style={{ fontSize: 12, opacity: 0.85 }}>브라우저에서 이 주소로 접속하세요</div>
            <code style={{ fontSize: 13 }}>
              http://{host?.addressHint ?? "<이 컴퓨터 주소>"}:{host?.port ?? appSettings.webRemotePort}
              /web/
            </code>
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
