import { t } from "../i18n";
const KEYS: Record<string, string> = {
  "work-send-busy": "busy", "work-send-agent-missing": "agentMissing", "work-send-clocked-out": "clockedOut",
  "work-send-profile-unsupported": "unsupportedProfile", "work-send-shell-unsupported": "unsupportedShell",
  "work-send-producer-running": "producerRunning", "work-send-human-input": "humanInput", "work-send-session-changed": "sessionChanged",
  "work-send-partial-submission": "partialSubmission", "work-send-launch-timeout": "launchTimeout", "work-send-cli-returned": "cliReturned",
  "work-send-cancelled": "cancelled", "work-send-restart-required": "restartRequired", "work-send-create-failed": "createFailed",
  "work-send-save-failed": "saveFailed", "work-send-blocked": "blocked", "work-send-plan-root": "planRoot", "work-send-schedule-template": "scheduleTemplate", "work-send-auto-run": "autoRunActive", "work-send-prompt-invalid": "promptInvalid", "work-send-model-invalid": "modelInvalid",
  "automation-agy-effort-invalid": "effortInvalid", "automation-effort-invalid": "effortInvalid",
  "automation-agy-model-catalog-unavailable": "modelCatalog", "automation-agy-model-unavailable": "modelCatalog",
};
/** 시작 차단·일시정지 상세 코드를 화면 문장으로. 자동 진행 코드가 아니면 보내기 오류 문구를 쓴다. `t`는 workpanels 이름공간이다. */
export function codeText(t: (key: string, options?: Record<string, unknown>) => string, code: string, options?: Record<string, unknown>): string {
  if (code.startsWith("auto-run-") && !code.startsWith("auto-run-judge")) return t(`autoRun.blocked.${code}`, options);
  if (code.startsWith("work-send-") || code.startsWith("automation-")) return sendErrorText(code);
  return code;
}
export function sendErrorText(error: string): string {
  const key = KEYS[error.split(":")[0]];
  return key ? t(`workpanels:sendErrors.${key}`) : t("workpanels:sendError", { error });
}
