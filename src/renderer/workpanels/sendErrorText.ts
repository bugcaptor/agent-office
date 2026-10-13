import { t } from "../i18n";
const KEYS: Record<string, string> = {
  "work-send-busy": "busy", "work-send-agent-missing": "agentMissing", "work-send-clocked-out": "clockedOut",
  "work-send-profile-unsupported": "unsupportedProfile", "work-send-shell-unsupported": "unsupportedShell",
  "work-send-producer-running": "producerRunning", "work-send-human-input": "humanInput", "work-send-session-changed": "sessionChanged",
  "work-send-partial-submission": "partialSubmission", "work-send-launch-timeout": "launchTimeout", "work-send-cli-returned": "cliReturned",
  "work-send-cancelled": "cancelled", "work-send-restart-required": "restartRequired", "work-send-create-failed": "createFailed",
  "work-send-save-failed": "saveFailed", "work-send-blocked": "blocked", "work-send-prompt-invalid": "promptInvalid", "work-send-model-invalid": "modelInvalid",
  "automation-agy-effort-invalid": "effortInvalid", "automation-effort-invalid": "effortInvalid",
  "automation-agy-model-catalog-unavailable": "modelCatalog", "automation-agy-model-unavailable": "modelCatalog",
};
export function sendErrorText(error: string): string {
  const key = KEYS[error.split(":")[0]];
  return key ? t(`workpanels:sendErrors.${key}`) : t("workpanels:sendError", { error });
}
