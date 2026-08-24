// src/renderer/agent/confirmDialogs.tsx
//
// 확인(confirm-*) 모달 6종을 한 껍데기로 모은 모듈. 예전엔 파일 6개가 스토어
// 셀렉터·running 계산·backdrop mousedown 핸들러·패널 마크업·버튼 쌍까지
// 그대로 복사돼 있었고 modal.kind 문자열/액션/문구만 달랐다 — 그 공통부를
// `ConfirmDialog`(껍데기) + `useConfirmTarget`(대상 이름/실행 여부)로 뽑고,
// 각 종류는 얇은 래퍼로 남겼다.
//
// CSS는 ProfileDialog와 동일한 전역 클래스(modal-backdrop / pixel-panel /
// pixel-btn / dialog-actions)를 재사용 — layout.css가 App 부팅 시 로드되어
// 있어 별도 import 불필요. `confirm-<slug>-dialog` / `confirm-<slug>-warning`은
// 현재 어떤 CSS도 정의하지 않는 식별용 훅(DOM 계약)이며, 종류별로 정확히
// 붙는다(예전 ConfirmClockOutDialog가 confirm-delete-warning을 복붙했던
// 버그를 여기서 바로잡았다). 경고색은 하드코딩 #e0574a 대신 테마 토큰
// `--accent-warn`을 쓴다.
import type { ReactNode } from "react";
import { Trans, useTranslation } from "react-i18next";

import { useAppStore } from "../store/appStore";
import { deleteAgent } from "./deleteAgent";
import { restartAgentSession } from "./restartAgentSession";
import { resumeAgentSession } from "./resumeAgentSession";
import { terminateAgentSession } from "./terminateSession";
import { clockOutAgent, clockOutAll } from "./clockOut";

interface ConfirmDialogProps {
  /** `confirm-<slug>-dialog` / `confirm-<slug>-warning` 클래스의 어근. */
  slug: string;
  title: string;
  /** 확인 버튼 라벨(취소 버튼은 항상 `app:confirm.cancel`). */
  confirmLabel: string;
  /** 확인 시 실행할 동작. 호출 뒤 모달은 껍데기가 닫는다. */
  onConfirm: () => void;
  /** 본문 문단(들). */
  children: ReactNode;
  /** 경고 문단 — falsy면 렌더하지 않는다(대개 `running &&`로 게이트). */
  warning?: ReactNode;
  /** 경고 뒤에 오는 마무리 문단(봇 모드 시작처럼 경고 다음에 질문이 오는 경우). */
  footer?: ReactNode;
}

/**
 * 확인 모달의 공통 껍데기. backdrop 왼쪽 버튼 클릭(패널 바깥)으로 닫히고,
 * [확인][취소] 두 버튼을 낸다. 확인은 `onConfirm` 후 항상 모달을 닫는다.
 */
export function ConfirmDialog({
  slug,
  title,
  confirmLabel,
  onConfirm,
  children,
  warning,
  footer,
}: ConfirmDialogProps) {
  const { t } = useTranslation("app");
  const closeModal = useAppStore((s) => s.closeModal);

  return (
    <div
      className="modal-backdrop"
      onMouseDown={(e) => {
        if (e.button === 0 && e.target === e.currentTarget) closeModal();
      }}
    >
      <div className={`pixel-panel confirm-${slug}-dialog`}>
        <h2 className="pixel-title">{title}</h2>
        {children}
        {warning && (
          <p className={`confirm-${slug}-warning`} style={{ color: "var(--accent-warn)" }}>
            {warning}
          </p>
        )}
        {footer}
        <div className="dialog-actions">
          <button
            className="pixel-btn primary"
            onClick={() => {
              onConfirm();
              closeModal();
            }}
          >
            {confirmLabel}
          </button>
          <button className="pixel-btn" onClick={closeModal}>
            {t("confirm.cancel")}
          </button>
        </div>
      </div>
    </div>
  );
}

/**
 * 확인 대상 에이전트의 표시 이름(프로필 이름 없으면 id 폴백)과 세션이
 * 실행 중(starting/running)인지. `agentId`가 없으면 이름 undefined·running false.
 */
export function useConfirmTarget(agentId: string | undefined) {
  const name = useAppStore((s) => (agentId ? (s.agents[agentId]?.name ?? agentId) : undefined));
  const running = useAppStore((s) => {
    if (!agentId) return false;
    const status = s.sessions[agentId]?.status;
    return status === "starting" || status === "running";
  });
  return { name, running };
}

/** 캐릭터 삭제 확인. 실행 중이면 세션 종료 경고. */
export function ConfirmDeleteDialog() {
  const { t } = useTranslation("app");
  const modal = useAppStore((s) => s.modal);
  const agentId = modal.kind === "confirm-delete" ? modal.agentId : undefined;
  const { name, running } = useConfirmTarget(agentId);

  if (modal.kind !== "confirm-delete") return null;

  return (
    <ConfirmDialog
      slug="delete"
      title={t("confirm.delete.title")}
      confirmLabel={t("confirm.delete.confirm")}
      onConfirm={() => {
        if (agentId) void deleteAgent(agentId);
      }}
      warning={running && t("confirm.delete.warning")}
    >
      <p>
        <Trans
          t={t}
          i18nKey="confirm.delete.body"
          values={{ name }}
          components={{ strong: <strong /> }}
        />
      </p>
    </ConfirmDialog>
  );
}

/** 터미널 재시작 확인. 실행 중이면 종료+스크롤백 소실 경고. */
export function ConfirmRestartDialog() {
  const { t } = useTranslation("app");
  const modal = useAppStore((s) => s.modal);
  const agentId = modal.kind === "confirm-restart" ? modal.agentId : undefined;
  const { name, running } = useConfirmTarget(agentId);

  if (modal.kind !== "confirm-restart") return null;

  return (
    <ConfirmDialog
      slug="restart"
      title={t("confirm.restart.title")}
      confirmLabel={t("confirm.restart.confirm")}
      onConfirm={() => {
        if (agentId) void restartAgentSession(agentId);
      }}
      warning={running && t("confirm.restart.warning")}
    >
      <p>
        <Trans
          t={t}
          i18nKey="confirm.restart.body"
          values={{ name }}
          components={{ strong: <strong /> }}
        />
      </p>
    </ConfirmDialog>
  );
}

/** 이전 Claude 세션 이어하기 확인. 캡처된 native sessionId를 함께 넘긴다. */
export function ConfirmResumeDialog() {
  const { t } = useTranslation("app");
  const modal = useAppStore((s) => s.modal);
  const agentId = modal.kind === "confirm-resume" ? modal.agentId : undefined;
  const sessionId = modal.kind === "confirm-resume" ? modal.sessionId : undefined;
  const { name, running } = useConfirmTarget(agentId);

  if (modal.kind !== "confirm-resume") return null;

  return (
    <ConfirmDialog
      slug="resume"
      title={t("confirm.resume.title")}
      confirmLabel={t("confirm.resume.confirm")}
      onConfirm={() => {
        if (agentId && sessionId) void resumeAgentSession(agentId, sessionId);
      }}
      warning={running && t("confirm.resume.warning")}
    >
      <p>
        <Trans
          t={t}
          i18nKey="confirm.resume.body"
          values={{ name }}
          components={{ strong: <strong /> }}
        />
      </p>
    </ConfirmDialog>
  );
}

/**
 * 터미널 종료 확인. 재시작과 달리 PTY만 죽이고 재생성하지 않는다 — 캐릭터는
 * FSM 규칙대로 탕비실로 이동하고, 캐릭터 클릭으로 재소환된다.
 */
export function ConfirmTerminateDialog() {
  const { t } = useTranslation("app");
  const modal = useAppStore((s) => s.modal);
  const agentId = modal.kind === "confirm-terminate" ? modal.agentId : undefined;
  const { name, running } = useConfirmTarget(agentId);

  if (modal.kind !== "confirm-terminate") return null;

  return (
    <ConfirmDialog
      slug="terminate"
      title={t("confirm.terminate.title")}
      confirmLabel={t("confirm.terminate.confirm")}
      onConfirm={() => {
        if (agentId) void terminateAgentSession(agentId);
      }}
      warning={running && t("confirm.terminate.warning")}
    >
      <p>
        <Trans
          t={t}
          i18nKey="confirm.terminate.body"
          values={{ name }}
          components={{ strong: <strong /> }}
        />
      </p>
    </ConfirmDialog>
  );
}

/**
 * 맨 셸 가드 확인(이슈 #57 후속). 봇 모드를 켤 때 터미널에 에이전트(claude 등)가
 * 떠 있는지 확신할 수 없으면(botGuard.looksLikeAgentRunning=false) 이 다이얼로그로
 * 경고한다 — 맨 셸에서 켜면 봇 지시문이 셸 명령으로 잘못 실행돼 에러가 나기
 * 때문이다. 확인하면 그래도 봇을 켠다(경고는 running과 무관하게 항상 표시).
 */
export function ConfirmBotStartDialog() {
  const { t } = useTranslation("app");
  const modal = useAppStore((s) => s.modal);
  const startBot = useAppStore((s) => s.startBot);
  const agentId = modal.kind === "confirm-bot-start" ? modal.agentId : undefined;
  const { name } = useConfirmTarget(agentId);

  if (modal.kind !== "confirm-bot-start") return null;

  return (
    <ConfirmDialog
      slug="bot-start"
      title={t("confirm.botStart.title")}
      confirmLabel={t("confirm.botStart.confirm")}
      onConfirm={() => {
        if (agentId) void startBot(agentId);
      }}
      warning={t("confirm.botStart.warning")}
      footer={<p>{t("confirm.botStart.footer")}</p>}
    >
      <p>
        <Trans
          t={t}
          i18nKey="confirm.botStart.body"
          values={{ name }}
          components={{ strong: <strong /> }}
        />
      </p>
    </ConfirmDialog>
  );
}

/**
 * 퇴근 확인. 개별 퇴근(confirm-clock-out)과 전체 퇴근(confirm-clock-out-all)
 * 두 종류를 한 컴포넌트가 함께 처리한다. 개별은 대상 이름을 보여주고 그 세션이
 * 실행 중일 때만 경고를, 전체는 근무 중 인원 수를 보여주고 여러 세션이 한꺼번에
 * 끝나므로 항상 경고를 띄운다.
 */
export function ConfirmClockOutDialog() {
  const { t } = useTranslation("app");
  const modal = useAppStore((s) => s.modal);
  const agentId = modal.kind === "confirm-clock-out" ? modal.agentId : undefined;
  const { name, running } = useConfirmTarget(agentId);
  // 전체 퇴근 다이얼로그용 근무 중(=clockedOut 아님) 인원 수.
  const onDutyCount = useAppStore(
    (s) => s.agentOrder.filter((id) => s.agents[id] && !s.agents[id].clockedOut).length
  );

  if (modal.kind !== "confirm-clock-out" && modal.kind !== "confirm-clock-out-all") return null;

  const isAll = modal.kind === "confirm-clock-out-all";

  return (
    <ConfirmDialog
      slug="clock-out"
      title={isAll ? t("confirm.clockOut.titleAll") : t("confirm.clockOut.title")}
      confirmLabel={t("confirm.clockOut.confirm")}
      onConfirm={() => {
        if (isAll) {
          void clockOutAll();
        } else if (agentId) {
          void clockOutAgent(agentId);
        }
      }}
      warning={
        (isAll || running) &&
        (isAll ? t("confirm.clockOut.warningAll") : t("confirm.clockOut.warning"))
      }
    >
      {isAll ? (
        <p>{t("confirm.clockOut.bodyAll", { count: onDutyCount })}</p>
      ) : (
        <p>
          <Trans
            t={t}
            i18nKey="confirm.clockOut.body"
            values={{ name }}
            components={{ strong: <strong /> }}
          />
        </p>
      )}
    </ConfirmDialog>
  );
}
