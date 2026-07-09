// src/renderer/agent/ConfirmClockOutDialog.tsx
//
// 퇴근 확인 다이얼로그. 개별 퇴근(confirm-clock-out)과 전체 퇴근
// (confirm-clock-out-all) 두 종류를 한 컴포넌트가 함께 처리한다(그 외 kind면
// null 렌더). ConfirmDeleteDialog와 동일한 패턴: 개별 퇴근은 대상 에이전트
// 이름을 보여주고 해당 세션이 실행 중(starting/running)일 때만 "진행 중인
// 세션이 종료됩니다" 경고를 띄우며, 전체 퇴근은 근무 중인 인원 수를 보여주고
// 여러 세션이 한꺼번에 종료되므로 항상 경고("진행 중인 세션이 모두
// 종료됩니다")를 띄운다. 확인 시 clockOutAgent/clockOutAll(오케스트레이터)
// 호출 후 모달을 닫고, 취소 시 모달만 닫는다. CSS는 ProfileDialog와 동일한
// 전역 클래스(modal-backdrop / pixel-panel / pixel-btn / dialog-actions)를
// 재사용 — layout.css가 App 부팅 시 로드되어 있어 별도 import 불필요.
import { useAppStore } from "../store/appStore";
import { clockOutAgent, clockOutAll } from "./clockOut";

export function ConfirmClockOutDialog() {
  const modal = useAppStore((s) => s.modal);
  const closeModal = useAppStore((s) => s.closeModal);
  const agentId = modal.kind === "confirm-clock-out" ? modal.agentId : undefined;
  const agent = useAppStore((s) => (agentId ? s.agents[agentId] : undefined));
  const running = useAppStore((s) => {
    if (!agentId) return false;
    const status = s.sessions[agentId]?.status;
    return status === "starting" || status === "running";
  });
  // 전체 퇴근 다이얼로그용 근무 중(=clockedOut 아님) 인원 수.
  const onDutyCount = useAppStore(
    (s) => s.agentOrder.filter((id) => s.agents[id] && !s.agents[id].clockedOut).length
  );

  if (modal.kind !== "confirm-clock-out" && modal.kind !== "confirm-clock-out-all") return null;

  const isAll = modal.kind === "confirm-clock-out-all";
  const name = agent?.name ?? agentId;

  const onConfirm = () => {
    if (isAll) {
      void clockOutAll();
    } else if (agentId) {
      void clockOutAgent(agentId);
    }
    closeModal();
  };

  return (
    <div
      className="modal-backdrop"
      onMouseDown={(e) => {
        if (e.button === 0 && e.target === e.currentTarget) closeModal();
      }}
    >
      <div className="pixel-panel confirm-clock-out-dialog">
        <h2 className="pixel-title">{isAll ? "전체 퇴근" : "퇴근"}</h2>
        {isAll ? (
          <p>근무 중인 캐릭터 {onDutyCount}명을 모두 퇴근시킬까요?</p>
        ) : (
          <p>
            정말 <strong>{name}</strong> 캐릭터를 퇴근시킬까요?
          </p>
        )}
        {(isAll || running) && (
          <p className="confirm-delete-warning" style={{ color: "#e0574a" }}>
            {isAll ? "진행 중인 세션이 모두 종료됩니다." : "진행 중인 세션이 종료됩니다."}
          </p>
        )}
        <div className="dialog-actions">
          <button className="pixel-btn primary" onClick={onConfirm}>
            퇴근
          </button>
          <button className="pixel-btn" onClick={closeModal}>
            취소
          </button>
        </div>
      </div>
    </div>
  );
}
