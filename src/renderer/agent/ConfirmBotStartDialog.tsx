// src/renderer/agent/ConfirmBotStartDialog.tsx
//
// 맨 셸 가드 확인 다이얼로그(이슈 #57 후속). 봇 모드를 켤 때 터미널에 에이전트
// (claude 등)가 떠 있는지 확신할 수 없으면(botGuard.looksLikeAgentRunning=false)
// 이 다이얼로그로 경고한다 — 맨 셸에서 켜면 봇 지시문이 셸 명령으로 잘못 실행돼
// 에러가 나기 때문이다. 확인하면 그래도 봇을 켠다. ConfirmTerminateDialog와 동일
// 패턴(전역 modal-backdrop / pixel-panel / dialog-actions).
import { useAppStore } from "../store/appStore";

export function ConfirmBotStartDialog() {
  const modal = useAppStore((s) => s.modal);
  const closeModal = useAppStore((s) => s.closeModal);
  const startBot = useAppStore((s) => s.startBot);
  const agentId = modal.kind === "confirm-bot-start" ? modal.agentId : undefined;
  const agent = useAppStore((s) => (agentId ? s.agents[agentId] : undefined));

  if (modal.kind !== "confirm-bot-start") return null;

  const name = agent?.name ?? agentId;

  const onConfirm = () => {
    if (agentId) void startBot(agentId);
    closeModal();
  };

  return (
    <div
      className="modal-backdrop"
      onMouseDown={(e) => {
        if (e.button === 0 && e.target === e.currentTarget) closeModal();
      }}
    >
      <div className="pixel-panel confirm-bot-start-dialog">
        <h2 className="pixel-title">봇 모드 시작</h2>
        <p>
          <strong>{name}</strong>의 터미널에서 에이전트(claude 등)가 실행 중인지
          확인할 수 없습니다.
        </p>
        <p className="confirm-bot-start-warning" style={{ color: "#e0574a" }}>
          맨 셸에서 봇을 켜면 봇이 보내는 작업 지시문이 셸 명령으로 잘못 입력되어
          에러가 납니다. 터미널에 claude를 먼저 띄운 뒤 켜는 것을 권장합니다.
        </p>
        <p>그래도 봇 모드를 켤까요?</p>
        <div className="dialog-actions">
          <button className="pixel-btn primary" onClick={onConfirm}>
            그래도 켜기
          </button>
          <button className="pixel-btn" onClick={closeModal}>
            취소
          </button>
        </div>
      </div>
    </div>
  );
}
