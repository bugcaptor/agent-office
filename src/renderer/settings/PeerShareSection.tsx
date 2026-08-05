// src/renderer/settings/PeerShareSection.tsx
//
// 피어 세션 공유(#7k, docs/peer-session-share-design.md) 설정 UI.
// 한 화면에 두 역할이 있다 — 위쪽은 **호스트**(내 캐릭터를 남에게 보여주기),
// 아래쪽은 **뷰어**(남의 캐릭터를 내 사무실에 세우기).
//
// 보안 흐름은 CLI 제어와 같은 2단계 옵트인이다: 토글로 서버를 켜도 페어링
// 승인 전에는 모든 요청이 401이고, 중계되는 캐릭터는 캐릭터별 공유 토글을
// 켠 것뿐이다(전체 공유 스위치는 일부러 두지 않는다).

import { useCallback, useEffect, useState } from "react";
import { useAppStore } from "../store/appStore";
import { peerApi, isRemoteAgentId } from "../ipc/peerApi";
import type { PeerHostStatus, PeerPermission, PeerViewerStatus } from "../ipc/peerApi";
import type { PeerBindPolicy } from "@shared/types";

const BIND_LABEL: Record<PeerBindPolicy, string> = {
  tailnet: "Tailscale 망만 (권장)",
  all: "모든 네트워크 (평문 전송 주의)",
  loopback: "이 컴퓨터만 (사실상 비활성)",
};

const STATE_LABEL: Record<PeerViewerStatus["state"], string> = {
  connected: "연결됨",
  connecting: "연결 중…",
  disconnected: "끊김",
};

export function PeerShareSection() {
  const appSettings = useAppStore((s) => s.appSettings);
  const updateAppSettings = useAppStore((s) => s.updateAppSettings);
  const agents = useAppStore((s) => s.agents);
  const agentOrder = useAppStore((s) => s.agentOrder);
  const setPeerPending = useAppStore((s) => s.setPeerPending);
  const viewers = useAppStore((s) => s.peerViewers);

  const [host, setHost] = useState<PeerHostStatus | null>(null);
  const [busy, setBusy] = useState(false);
  const [note, setNote] = useState<string | null>(null);

  const refreshHost = useCallback(async () => {
    try {
      const status = await peerApi.hostStatus();
      setHost(status);
      setPeerPending(status.pending ?? []);
    } catch {
      setHost(null);
    }
  }, [setPeerPending]);

  useEffect(() => {
    void refreshHost();
  }, [refreshHost, appSettings.peerShareEnabled]);

  const sharedSet = new Set(host?.sharedAgents ?? []);
  const localAgents = agentOrder.filter((id) => !isRemoteAgentId(id));

  const toggleShared = async (agentId: string, shared: boolean) => {
    setBusy(true);
    try {
      await peerApi.setShared(agentId, shared);
      await refreshHost();
    } catch (err) {
      setNote(String(err));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="settings-form">
      <label className="settings-item">
        <input
          type="checkbox"
          checked={appSettings.peerShareEnabled}
          onChange={(e) => updateAppSettings({ peerShareEnabled: e.target.checked })}
        />
        <span>
          <strong>세션 공유 (같은 망의 다른 Agent Office에 보여주기)</strong>
          <small>
            내 캐릭터의 터미널을 같은 네트워크의 다른 Agent Office에서 보고
            입력하게 합니다. 세션은 <b>이 컴퓨터에서 계속 돌고</b> 출력/입력만
            중계됩니다. 켜도 아래에서 <b>페어링을 승인</b>해야 하고, 공유되는
            캐릭터는 따로 체크한 것뿐입니다. 네트워크 표면이므로 기본 꺼짐.
          </small>
        </span>
      </label>

      {appSettings.peerShareEnabled && (
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
              value={appSettings.peerBind}
              onChange={(e) =>
                updateAppSettings({ peerBind: e.target.value as PeerBindPolicy })
              }
            >
              {(Object.keys(BIND_LABEL) as PeerBindPolicy[]).map((k) => (
                <option key={k} value={k}>
                  {BIND_LABEL[k]}
                </option>
              ))}
            </select>
          </label>

          <div className="settings-item" style={{ flexDirection: "column", alignItems: "stretch", gap: 8 }}>
            <div style={{ fontSize: 12, opacity: 0.85 }}>
              상태:{" "}
              {host
                ? host.running
                  ? `수신 중 · 상대에게 알려줄 주소 ${host.addressHint ?? "?"}:${host.port ?? "?"} (이름: ${host.hostName})`
                  : "정지"
                : "조회 중…"}
            </div>

            {/* 승인 대기 페어링(코드 표시)은 PairingRequestDialog가 모달 층에서
                맡는다 — 설정을 열어 두지 않아도 코드가 보여야 하고, 웹 호스팅만
                켠 경우에도 떠야 하기 때문이다. */}

            <div style={{ fontSize: 12, opacity: 0.85 }}>공유할 캐릭터</div>
            {localAgents.length === 0 && (
              <div style={{ fontSize: 12, opacity: 0.7 }}>아직 캐릭터가 없습니다.</div>
            )}
            {localAgents.map((id) => (
              <label key={id} className="settings-item" style={{ padding: "2px 0" }}>
                <input
                  type="checkbox"
                  checked={sharedSet.has(id)}
                  disabled={busy}
                  onChange={(e) => void toggleShared(id, e.target.checked)}
                />
                <span>{agents[id]?.name ?? id}</span>
              </label>
            ))}

            {host && host.peers.length > 0 && (
              <>
                <div style={{ fontSize: 12, opacity: 0.85, marginTop: 6 }}>
                  승인된 상대
                </div>
                {host.peers.map((p) => (
                  <div
                    key={p.peerId}
                    style={{ display: "flex", gap: 6, alignItems: "center", flexWrap: "wrap" }}
                  >
                    <span style={{ flex: 1 }}>{p.name}</span>
                    <select
                      value={p.permission}
                      disabled={busy}
                      onChange={(e) => {
                        setBusy(true);
                        void peerApi
                          .setPeerPermission(p.peerId, e.target.value as PeerPermission)
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
                        void peerApi
                          .revokePeer(p.peerId)
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

      <label className="settings-item">
        <input
          type="checkbox"
          checked={appSettings.webHostingEnabled}
          onChange={(e) => updateAppSettings({ webHostingEnabled: e.target.checked })}
        />
        <span>
          <strong>웹 호스팅 (브라우저로 접속해서 작업)</strong>
          <small>
            폰이나 다른 컴퓨터의 <b>브라우저</b>로 접속해 상태를 보고 터미널을
            조작합니다. 세션 공유와 <b>같은 서버·같은 포트</b>를 쓰고, 접속하려면
            역시 페어링 승인이 필요합니다. 브라우저에는 <b>정해진 명령만</b>
            열립니다(설정 변경·봇 조작은 열리지 않습니다).
          </small>
        </span>
      </label>

      {appSettings.webHostingEnabled && (
        <div className="settings-item" style={{ flexDirection: "column", alignItems: "stretch", gap: 6 }}>
          <div style={{ fontSize: 12, opacity: 0.85 }}>브라우저에서 이 주소로 접속하세요</div>
          <code style={{ fontSize: 13 }}>
            http://{host?.addressHint ?? "<이 컴퓨터 주소>"}:{host?.port ?? appSettings.peerPort}
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
        </div>
      )}

      <PeerConnectSection viewers={viewers} />
      {note && <div style={{ fontSize: 12, opacity: 0.85 }}>{note}</div>}
    </div>
  );
}

/**
 * 뷰어 쪽: 호스트 주소를 직접 입력해 페어링한다(수동 입력이 곧 디스커버리 —
 * tailnet에서는 mDNS가 동작하지 않는다). 코드 입력 후 호스트가 아직 승인을
 * 누르지 않았으면 백엔드가 "대기"를 돌려주므로 그대로 재시도한다.
 */
function PeerConnectSection({ viewers }: { viewers: PeerViewerStatus[] }) {
  const [address, setAddress] = useState("");
  const [pairingId, setPairingId] = useState<string | null>(null);
  const [hostName, setHostName] = useState("");
  const [code, setCode] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const start = async () => {
    setBusy(true);
    setError(null);
    try {
      const outcome = await peerApi.pairStart(address.trim());
      setPairingId(outcome.pairingId);
      setHostName(outcome.hostName);
    } catch (err) {
      setError(String(err));
    } finally {
      setBusy(false);
    }
  };

  const finish = async () => {
    if (!pairingId) return;
    setBusy(true);
    setError(null);
    try {
      const done = await peerApi.pairFinish(address.trim(), pairingId, code.trim());
      if (done) {
        setPairingId(null);
        setCode("");
        setAddress("");
      } else {
        setError("상대가 아직 승인하지 않았습니다. 승인 후 다시 누르세요.");
      }
    } catch (err) {
      setError(String(err));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="settings-item" style={{ flexDirection: "column", alignItems: "stretch", gap: 8 }}>
      <div style={{ fontSize: 12, opacity: 0.85 }}>
        <strong>다른 사무실에 연결</strong> — 상대 앱의 설정에 표시된 주소를
        입력하세요.
      </div>
      <div style={{ display: "flex", gap: 6, flexWrap: "wrap" }}>
        <input
          type="text"
          placeholder="100.x.y.z:47800"
          value={address}
          disabled={busy || pairingId !== null}
          onChange={(e) => setAddress(e.target.value)}
          style={{ flex: 1, minWidth: 160 }}
        />
        {pairingId === null ? (
          <button
            className="pixel-btn"
            disabled={busy || address.trim() === ""}
            onClick={() => void start()}
          >
            연결 요청
          </button>
        ) : (
          <>
            <input
              type="text"
              placeholder="6자리 코드"
              value={code}
              disabled={busy}
              onChange={(e) => setCode(e.target.value)}
              style={{ width: 110, fontFamily: "monospace" }}
            />
            <button className="pixel-btn" disabled={busy} onClick={() => void finish()}>
              코드 확인
            </button>
            <button
              className="pixel-btn"
              disabled={busy}
              onClick={() => {
                setPairingId(null);
                setCode("");
              }}
            >
              취소
            </button>
          </>
        )}
      </div>
      {pairingId !== null && (
        <div style={{ fontSize: 12, opacity: 0.85 }}>
          <b>{hostName}</b> 화면에 표시된 6자리 코드를 입력하세요(상대가 승인을
          누른 뒤에 통과됩니다).
        </div>
      )}
      {error && <div style={{ fontSize: 12, color: "var(--danger, #c0392b)" }}>{error}</div>}

      {viewers.map((v) => (
        <div key={v.peerId} style={{ display: "flex", gap: 6, alignItems: "center", flexWrap: "wrap" }}>
          <span style={{ flex: 1 }}>
            {v.label} <small style={{ opacity: 0.7 }}>({v.address})</small>
          </span>
          <span style={{ fontSize: 12, opacity: 0.85 }}>
            {STATE_LABEL[v.state]}
            {v.permission === "readOnly" ? " · 읽기 전용" : ""}
            {v.agents.length > 0 ? ` · 캐릭터 ${v.agents.length}명` : ""}
          </span>
          <button
            className="pixel-btn"
            onClick={() => {
              void peerApi.disconnect(v.peerId);
            }}
          >
            연결 끊기
          </button>
          <button
            className="pixel-btn"
            onClick={() => {
              void peerApi.forgetHost(v.peerId);
            }}
          >
            삭제
          </button>
        </div>
      ))}
    </div>
  );
}
