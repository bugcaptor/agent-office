// src/renderer/agent/ConfirmDeleteDialog.tsx
//
// 캐릭터 삭제 확인 다이얼로그. ModalState가
// confirm-delete일 때만 렌더한다. 에이전트 이름을 표시하고, 해당 세션이
// 실행 중(starting/running)이면 "실행 중인 세션이 종료됩니다" 경고를 띄운다.
// 삭제 확인 시 deleteAgent(오케스트레이터) 호출 후 모달을 닫고, 취소 시
// 모달만 닫는다. CSS는 ProfileDialog와 동일한 전역 클래스(modal-backdrop /
// pixel-panel / pixel-btn / dialog-actions)를 재사용 — layout.css가 App
// 부팅 시 로드되어 있어 별도 import 불필요.
import { useAppStore } from "../store/appStore";
import { deleteAgent } from "./deleteAgent";

export function ConfirmDeleteDialog() {
  const modal = useAppStore((s) => s.modal);
  const closeModal = useAppStore((s) => s.closeModal);
  const agentId = modal.kind === "confirm-delete" ? modal.agentId : undefined;
  const agent = useAppStore((s) => (agentId ? s.agents[agentId] : undefined));
  const running = useAppStore((s) => {
    if (!agentId) return false;
    const status = s.sessions[agentId]?.status;
    return status === "starting" || status === "running";
  });

  if (modal.kind !== "confirm-delete") return null;

  const name = agent?.name ?? agentId;

  const onConfirm = () => {
    if (agentId) void deleteAgent(agentId);
    closeModal();
  };

  return (
    <div
      className="modal-backdrop"
      onMouseDown={(e) => {
        if (e.button === 0 && e.target === e.currentTarget) closeModal();
      }}
    >
      <div className="pixel-panel confirm-delete-dialog">
        <h2 className="pixel-title">캐릭터 삭제</h2>
        <p>
          정말 <strong>{name}</strong> 캐릭터를 삭제할까요?
        </p>
        {running && (
          <p className="confirm-delete-warning" style={{ color: "#e0574a" }}>
            실행 중인 세션이 종료됩니다.
          </p>
        )}
        <div className="dialog-actions">
          <button className="pixel-btn primary" onClick={onConfirm}>
            삭제
          </button>
          <button className="pixel-btn" onClick={closeModal}>
            취소
          </button>
        </div>
      </div>
    </div>
  );
}
