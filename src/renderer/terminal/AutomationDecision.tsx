// src/renderer/terminal/AutomationDecision.tsx
//
// 타임아웃 결정 비차단 패널 (kbm #2t9 §1, §4, §5):
// - watching 제한시간 초과 시 나타나는 패널.
// - 비차단: 모달 배경(스크림)이 없어 사용자가 터미널에 계속 타이핑할 수 있고 포커스를 가로채지 않는다.
// - [더 기다리기] / [자동화 중단] 두 선택지 제공.
// - 선택 대기 중 마커가 관측되면 "완료 신호 감지"를 띄우고 선택을 유지한다.

import { useState } from "react";
import { useTranslation } from "react-i18next";
import { useAppStore } from "../store/appStore";

export function AutomationDecision({ agentId }: { agentId: string }) {
  const { t } = useTranslation("terminal");
  const status = useAppStore((s) => s.automation[agentId]);
  const decideAutomation = useAppStore((s) => s.decideAutomation);
  const [submitting, setSubmitting] = useState(false);

  if (
    !status ||
    (status.phase !== "timeoutDecision" &&
      status.phase !== "confirming" &&
      status.phase !== "waitingHumanInput")
  ) {
    return null;
  }

  const onExtend = async () => {
    if (submitting) return;
    setSubmitting(true);
    try {
      await decideAutomation(agentId, "extend");
    } finally {
      setSubmitting(false);
    }
  };

  const onStop = async () => {
    if (submitting) return;
    setSubmitting(true);
    try {
      await decideAutomation(agentId, "stop");
    } finally {
      setSubmitting(false);
    }
  };
  const onContinue = async () => {
    if (submitting) return;
    setSubmitting(true);
    try {
      await decideAutomation(agentId, "continue");
    } finally {
      setSubmitting(false);
    }
  };

  const markerObserved = status.markerObservedAtMs != null;
  const reason = status.decisionReason ?? "timeout";
  const title =
    reason === "confirm"
      ? t("automation.confirmTitle")
      : reason === "blocked"
        ? t("automation.blockedTitle")
        : reason === "humanInput"
          ? t("automation.humanInputTitle")
          : reason === "protocolError"
            ? t("automation.protocolErrorTitle")
            : reason === "shellReady"
              ? t("automation.shellReadyTitle")
              : reason === "cliExitTimeout"
                ? t("automation.cliExitTimeoutTitle")
                : reason === "cliExitUnconfirmed"
                  ? t("automation.cliExitUnconfirmedTitle")
                  : t("automation.decisionTitle");
  const receiptObserved = status.cliReturnObservedAtMs != null;
  const detail =
    status.decisionMessage === "automation-cli-input-before-exit"
      ? t("automation.cliInputBeforeExit")
      : status.decisionMessage === "automation-cli-input-after-launch"
        ? t("automation.cliInputAfterLaunch")
        : status.decisionMessage === "automation-cli-manual-return"
          ? t("automation.cliManualReturn")
          : status.decisionMessage?.startsWith("automation-cli-return-code:")
            ? t("automation.cliReturnFailed")
            : (status.decisionMessage ??
              (reason === "shellReady"
                ? t("automation.shellReadyDesc")
                : reason === "cliExitTimeout" || reason === "cliExitUnconfirmed"
                  ? t("automation.cliManualReturn")
                  : t("automation.decisionDesc")));

  return (
    <div className="automation-decision-panel" role="region" aria-label={title}>
      <div className="automation-decision-header">
        <span className="automation-decision-icon" aria-hidden="true">
          ⏳
        </span>
        <span className="automation-decision-title">{title}</span>
      </div>

      <div className="automation-decision-desc">{detail}</div>

      {(markerObserved || receiptObserved) && (
        <div className="automation-decision-marker">
          <span aria-hidden="true">🔔</span>{" "}
          {receiptObserved
            ? t("automation.decisionReceiptObserved")
            : t("automation.decisionMarkerObserved")}
        </div>
      )}

      <div className="automation-decision-actions">
        {(reason === "confirm" ||
          reason === "shellReady" ||
          reason === "cliExitUnconfirmed" ||
          reason === "blocked" ||
          reason === "humanInput") && (
          <button
            type="button"
            className="pixel-btn primary"
            onClick={onContinue}
            disabled={submitting}
          >
            {t("automation.continue")}
          </button>
        )}
        {(reason === "timeout" ||
          reason === "cliExitTimeout" ||
          reason === "blocked" ||
          reason === "protocolError") && (
          <button
            type="button"
            className="pixel-btn primary"
            onClick={onExtend}
            disabled={submitting}
          >
            {t("automation.extend")}
          </button>
        )}
        <button
          type="button"
          className="pixel-btn"
          onClick={onStop}
          disabled={submitting}
        >
          {t("automation.stop")}
        </button>
      </div>
    </div>
  );
}
