// src/web/PairingScreen.tsx
//
// 브라우저 페어링(#7m §D). 흐름은 앱↔앱과 같지만 **코드 표시 주체가 반대**다:
// 앱이 6자리 코드를 보여주고 브라우저가 입력한다(폰에서 이쪽이 편하다).
// 승인 성공 시 서버가 HttpOnly 쿠키를 내려주므로, 이후 WS는 저절로 인증된다.

import { useState } from "react";

interface Props {
  onPaired: () => void;
}

export function PairingScreen({ onPaired }: Props) {
  const [pairingId, setPairingId] = useState<string | null>(null);
  const [hostName, setHostName] = useState("");
  const [code, setCode] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const start = async () => {
    setBusy(true);
    setError(null);
    try {
      const res = await fetch("/peer/v1/pair/start", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({
          viewerName: navigator.userAgent.includes("Mobile") ? "휴대폰 브라우저" : "브라우저",
          clientKind: "web",
        }),
      });
      const body = await res.json();
      if (!res.ok || body.ok !== true) {
        setError(body.error ?? "연결을 시작하지 못했습니다");
        return;
      }
      setPairingId(body.data.pairingId);
      setHostName(body.data.hostName);
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
      const res = await fetch("/peer/v1/pair/complete", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ pairingId, code: code.trim() }),
      });
      if (res.status === 202) {
        setError("앱에서 아직 승인하지 않았습니다. 승인 후 다시 누르세요.");
        return;
      }
      const body = await res.json();
      if (!res.ok || body.ok !== true) {
        setError(body.error ?? "페어링에 실패했습니다");
        return;
      }
      onPaired();
    } catch (err) {
      setError(String(err));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="pair">
      <h1>Agent Office</h1>
      {pairingId === null ? (
        <>
          <p className="muted">
            이 기기를 사무실에 연결합니다. 아래를 누르면 <b>앱 화면에 6자리 코드</b>가
            뜹니다.
          </p>
          <button className="btn primary" disabled={busy} onClick={() => void start()}>
            연결 요청
          </button>
        </>
      ) : (
        <>
          <p className="muted">
            <b>{hostName}</b> 앱에 표시된 6자리 코드를 입력하고, 앱에서 승인을 누르세요.
          </p>
          <input
            className="code-input"
            inputMode="numeric"
            autoComplete="one-time-code"
            maxLength={6}
            placeholder="000000"
            value={code}
            onChange={(e) => setCode(e.target.value.replace(/\D/g, ""))}
          />
          <button
            className="btn primary"
            disabled={busy || code.length < 6}
            onClick={() => void finish()}
          >
            연결
          </button>
          <button
            className="btn"
            disabled={busy}
            onClick={() => {
              setPairingId(null);
              setCode("");
              setError(null);
            }}
          >
            취소
          </button>
        </>
      )}
      {error && <p className="error">{error}</p>}
    </div>
  );
}
