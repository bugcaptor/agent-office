// src/renderer/agent/ConfirmResumeDialog.tsx
//
// Claude 세션 이어하기 확인 다이얼로그 (ConfirmRestartDialog와 동일 패턴).
// ModalState가 confirm-resume일 때만 렌더한다. 에이전트 이름을 표시하고,
// 해당 세션이 실행 중(starting/running)이면 종료/스크롤백 경고를 띄운다.
// 확인 시 resumeAgentSession(오케스트레이터)에 캡처된 native sessionId를
// 넘겨 이전 대화를 이어서 시작한다. CSS는 다른 다이얼로그와 동일한 전역
// 클래스(modal-backdrop / pixel-panel / pixel-btn / dialog-actions)를 재사용.
import { useAppStore } from "../store/appStore";
import { resumeAgentSession } from "./resumeAgentSession";

export function ConfirmResumeDialog() {
  const modal = useAppStore((s) => s.modal);
  const closeModal = useAppStore((s) => s.closeModal);
  const agentId = modal.kind === "confirm-resume" ? modal.agentId : undefined;
  const sessionId = modal.kind === "confirm-resume" ? modal.sessionId : undefined;
  const agent = useAppStore((s) => (agentId ? s.agents[agentId] : undefined));
  const running = useAppStore((s) => {
    if (!agentId) return false;
    const status = s.sessions[agentId]?.status;
    return status === "starting" || status === "running";
  });

  if (modal.kind !== "confirm-resume") return null;

  const name = agent?.name ?? agentId;

  const onConfirm = () => {
    if (agentId && sessionId) void resumeAgentSession(agentId, sessionId);
    closeModal();
  };

  return (
    <div
      className="modal-backdrop"
      onMouseDown={(e) => {
        if (e.button === 0 && e.target === e.currentTarget) closeModal();
      }}
    >
      <div className="pixel-panel confirm-resume-dialog">
        <h2 className="pixel-title">이전 세션 이어하기</h2>
        <p>
          <strong>{name}</strong>의 현재 세션을 종료하고 이전 Claude 세션을 이어할까요?
        </p>
        {running && (
          <p className="confirm-resume-warning" style={{ color: "#e0574a" }}>
            실행 중인 세션이 종료되고 스크롤백이 지워집니다.
          </p>
        )}
        <div className="dialog-actions">
          <button className="pixel-btn primary" onClick={onConfirm}>
            이어하기
          </button>
          <button className="pixel-btn" onClick={closeModal}>
            취소
          </button>
        </div>
      </div>
    </div>
  );
}
