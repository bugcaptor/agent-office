// src/renderer/settings/PairingRequestDialog.tsx
//
// 페어링 승인 다이얼로그(웹 원격, docs/web-remote-design.md).
//
// 페어링은 **호스트 화면에 뜬 6자리 코드**를 브라우저에서 받아 적는 사람-루프
// 장치라, 코드가 어디에도 안 보이면 페어링 자체가 불가능하다. 승인은 설정을
// 열어 둔 사람만 하는 일이 아니므로 설정 섹션이 아니라 모달 층에 둔다 —
// 상시 마운트, `peerPending`이 비면 null 렌더.
//
// 코드는 백엔드에서 TTL(2분)이 지나면 사라지지만 렌더러 스토어는 이벤트로
// 밀어 넣은 항목의 나이를 모른다. 그래서 `expiresInMs`를 같이 받아 만료 시각에
// 스스로 지운다(만료된 코드를 계속 띄우면 상대는 통과할 수 없는 숫자를 친다).

import { useEffect } from "react";
import { useAppStore } from "../store/appStore";
import { peerApi } from "../ipc/peerApi";
import type { PeerPermission } from "../ipc/peerApi";

/** 백엔드 PAIRING_TTL과 같은 값 — expiresInMs가 없는 구버전 응답의 보수적 기본값. */
const PAIRING_TTL_MS = 120_000;

export function PairingRequestDialog() {
  const pending = useAppStore((s) => s.peerPending);
  const setPeerPending = useAppStore((s) => s.setPeerPending);

  // 승인 대기는 한 번에 하나만 보여준다(가장 먼저 온 것). 뒤엣것은 이게
  // 처리되면 저절로 올라온다 — 코드 두 개를 나란히 띄우면 상대가 어느 쪽을
  // 칠지 알 수 없다.
  const current = pending[0] ?? null;

  // 만료 자동 소멸. 대기 목록이 바뀔 때마다 타이머를 다시 건다.
  useEffect(() => {
    if (pending.length === 0) return;
    const timers = pending.map((p) =>
      window.setTimeout(
        () => {
          const rest = useAppStore.getState().peerPending.filter((q) => q.pairingId !== p.pairingId);
          useAppStore.getState().setPeerPending(rest);
        },
        Math.max(0, p.expiresInMs ?? PAIRING_TTL_MS)
      )
    );
    return () => timers.forEach((t) => window.clearTimeout(t));
  }, [pending]);

  if (!current) return null;

  const drop = () => setPeerPending(pending.filter((p) => p.pairingId !== current.pairingId));

  const approve = (permission: PeerPermission) => {
    void peerApi.approvePairing(current.pairingId, permission).catch(() => {
      /* 이미 만료·소비됨 — 목록에서 빼는 것으로 충분하다 */
    });
    drop();
  };

  const reject = () => {
    void peerApi.rejectPairing(current.pairingId).catch(() => {});
    drop();
  };

  return (
    <div className="modal-backdrop">
      {/* backdrop 클릭으로는 닫지 않는다 — 승인/거부는 명시적 선택이어야 한다. */}
      <div className="pixel-panel pairing-request-dialog">
        <h2 className="pixel-title">연결 요청</h2>
        <p>
          <b>{current.viewerName}</b> (웹 브라우저) 이(가) 이 사무실에 연결하려
          합니다.
        </p>
        <p>브라우저 화면에 이 코드를 입력하세요:</p>
        <div
          className="pairing-request-code"
          style={{ fontSize: 32, letterSpacing: 6, fontFamily: "monospace", textAlign: "center" }}
        >
          {current.code}
        </div>
        <p style={{ color: "var(--accent-warn)" }}>
          모르는 요청이면 거부하세요. 승인하면 내 캐릭터의 터미널을 보고 정해진
          명령을 쓸 수 있게 됩니다.
        </p>
        {pending.length > 1 && (
          <p style={{ fontSize: 12, opacity: 0.75 }}>대기 중인 요청 {pending.length - 1}건 더</p>
        )}
        <div className="dialog-actions">
          <button className="pixel-btn primary" onClick={() => approve("input")}>
            승인 (입력 허용)
          </button>
          <button className="pixel-btn" onClick={() => approve("readOnly")}>
            승인 (읽기 전용)
          </button>
          <button className="pixel-btn" onClick={reject}>
            거부
          </button>
        </div>
      </div>
    </div>
  );
}
