// src/renderer/terminal/AutomationBanner.tsx
//
// 자동화 상단 상태 배너 (비차단):
// - 자동화가 돌고 있을 때 터미널 위에 떠서 현재 상태, 남은 시간, 연장 횟수, 중단 버튼을 제공한다.
// - 터미널 전체를 가리지 않으며(pointer-events: none 컨테이너), 사용자의 직접 입력을 방해하지 않는다.

import { useEffect, useState } from "react";
import { useTranslation } from "react-i18next";
import { useAppStore } from "../store/appStore";
import { automationStatusText } from "./automationStatusText";

export function AutomationBanner({ agentId }: { agentId: string }) {
  const { t } = useTranslation("terminal");
  const status = useAppStore((s) => s.automation[agentId]);
  const openModal = useAppStore((s) => s.openModal);
  const stopAutomation = useAppStore((s) => s.stopAutomation);
  const continueAutomation = useAppStore((s) => s.continueAutomation);

  // 1초 주기 리렌더링으로 카운트다운 갱신
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    if (!status || !status.running) return;
    const interval = window.setInterval(() => setNow(Date.now()), 1000);
    return () => window.clearInterval(interval);
  }, [status]);

  if (
    !status ||
    (!status.running &&
      status.phase !== "failed" &&
      status.phase !== "cancelled")
  ) {
    return null;
  }

  const { icon, title, detail } = automationStatusText(status);

  // 보류 사유 문구 (pendingReason)
  let pendingText: string | undefined;
  if (status.pendingReason) {
    pendingText =
      status.pendingReason === "humanTyping"
        ? t("automation.pendingHuman")
        : t("automation.pendingAnotherProducer");
  }

  // 남은 시간 계산 (deadlineMs)
  let remainingText: string | undefined;
  if (
    status.deadlineMs &&
    (status.phase === "watching" || status.phase === "exiting")
  ) {
    const diffSec = Math.max(0, Math.floor((status.deadlineMs - now) / 1000));
    const mins = Math.floor(diffSec / 60);
    const secs = diffSec % 60;
    const timeStr = `${mins}:${secs.toString().padStart(2, "0")}`;
    remainingText = t("automation.bannerRemaining", { time: timeStr });
  }

  const onStop = () => {
    void stopAutomation(agentId);
  };

  const onContinue = () => {
    void continueAutomation(agentId);
  };

  return (
    <div className="automation-banner-container" aria-live="polite">
      <div className="automation-banner">
        <span className="automation-banner-icon" aria-hidden="true">
          {icon}
        </span>
        <div className="automation-banner-content">
          <span className="automation-banner-cli">
            {status.cliContext?.state === "shell"
              ? t("automation.cliContextShell")
              : status.cliContext?.state === "unknown"
                ? t("automation.cliContextUnknown")
                : status.cli}
          </span>
          {status.definitionName && (
            <span className="automation-banner-cli">
              {status.definitionName}
            </span>
          )}
          {status.cycle != null && status.stepIndex != null && (
            <span className="automation-banner-cli">
              {t("automation.bannerStep", {
                cycle: status.cycle,
                step: (status.stepIndex ?? 0) + 1,
              })}
            </span>
          )}
          <span className="automation-banner-title">{title}</span>
          {detail && !pendingText && (
            <span className="automation-banner-pending">{detail}</span>
          )}
          {pendingText && (
            <span className="automation-banner-pending">{pendingText}</span>
          )}
          {remainingText && (
            <span className="automation-banner-remaining">{remainingText}</span>
          )}
          {status.extensionCount != null && status.extensionCount > 0 && (
            <span className="automation-banner-extension">
              {t("automation.bannerExtension", {
                count: status.extensionCount,
              })}
            </span>
          )}
        </div>
        {status.running && status.pendingReason === "humanTyping" && (
          <button
            type="button"
            className="pixel-btn automation-banner-continue"
            onClick={onContinue}
          >
            {t("automation.continue")}
          </button>
        )}
        {!status.running && (
          <button
            type="button"
            className="pixel-btn automation-banner-restart"
            onClick={() => openModal({ kind: "confirm-restart", agentId })}
          >
            {t("menu.restart")}
          </button>
        )}
        {status.running && (
          <button
            type="button"
            className="pixel-btn automation-banner-stop"
            onClick={onStop}
          >
            {t("automation.stop")}
          </button>
        )}
      </div>
    </div>
  );
}
