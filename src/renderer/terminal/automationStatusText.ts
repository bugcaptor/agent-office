// src/renderer/terminal/automationStatusText.ts
//
// 자동화 점검(kbm) 런타임 상태(AutomationAgentStatus) → 사람이 읽는 문구.
// `botStatusText.ts`와 같은 모양 — 탭 배지 툴팁이 이 문구를 쓴다.
//
// React 밖에서도 불릴 수 있는 순수 모듈이라 훅이 아니라 모듈 `t`를 쓴다 —
// 다만 **호출 시점**에만 부른다(모듈 최상위에서 부르면 언어를 바꿔도
// 문구가 그대로 굳는다).
import { t } from "@renderer/i18n";
import { backendErrorText } from "../shared/backendError";
import type { AutomationAgentStatus } from "@shared/types";

export interface AutomationStatusText {
  /** 아이콘(진행/완료/경고). */
  icon: string;
  /** 한 줄 제목. */
  title: string;
  /** 보조 설명. 없을 수 있다. */
  detail?: string;
}

/** 백엔드 자동화 에러 코드 → 카탈로그 키. `backendErrorText`의 override로 넘긴다. */
const AUTOMATION_ERROR_KEYS: Record<string, string> = {
  "automation-session-not-running": "terminal:automation.errSessionNotRunning",
  "automation-cwd-unknown": "terminal:automation.errCwdUnknown",
  "automation-file-reset-failed": "terminal:automation.errFileReset",
  "automation-path-invalid": "terminal:automation.errPathInvalid",
  "automation-prompt-write-failed": "terminal:automation.errPromptWrite",
  "automation-result-version-unsupported":
    "terminal:automation.errResultVersion",
  "automation-session-lost": "terminal:automation.errSessionLost",
  "automation-bot-running": "terminal:automation.errBotRunning",
  "automation-session-changed": "terminal:automation.errSessionChanged",
  "automation-transition-partial-submission":
    "terminal:automation.errTransitionPartialSubmission",
  "automation-cli-return-shell-unverified":
    "terminal:automation.errCliReturnShellUnverified",
  "automation-cli-return-shell-unsupported":
    "terminal:automation.errCliReturnShellUnsupported",
  "automation-cli-return-read-failed":
    "terminal:automation.errCliReturnReadFailed",
  "automation-cli-return-file-invalid":
    "terminal:automation.errCliReturnFileInvalid",
  "automation-cli-launch-source-too-long":
    "terminal:automation.errCliLaunchSourceTooLong",
  "automation-cli-launch-script-write-failed":
    "terminal:automation.errCliLaunchScriptWriteFailed",
  "automation-cli-launch-ack-timeout":
    "terminal:automation.errCliLaunchAckTimeout",
  "automation-cli-return-path-escape":
    "terminal:automation.errCliReturnFileInvalid",
  "automation-cli-return-root-invalid":
    "terminal:automation.errCliReturnFileInvalid",
  "automation-cli-return-mismatch": "terminal:automation.errCliReturnMismatch",
  "automation-cli-return-symlink":
    "terminal:automation.errCliReturnFileInvalid",
};

/** phase·error로 배지 툴팁 문구를 만든다. */
export function automationStatusText(
  st: AutomationAgentStatus,
): AutomationStatusText {
  if (st.phase === "failed" || st.error) {
    return {
      icon: "⚠️",
      title: t("terminal:automation.failed"),
      detail: st.error
        ? st.error.startsWith("automation-cli-returned-before-task:")
          ? t("terminal:automation.errCliReturnedBeforeTask")
          : backendErrorText(st.error, AUTOMATION_ERROR_KEYS)
        : t("terminal:automation.errorUnknown"),
    };
  }
  let base: AutomationStatusText;
  switch (st.phase) {
    case "launching":
      base = { icon: "⚙️", title: t("terminal:automation.launching") };
      break;
    case "waitingStartup":
      base = { icon: "⚙️", title: t("terminal:automation.waitingStartup") };
      break;
    case "injecting":
      base = { icon: "⚙️", title: t("terminal:automation.injecting") };
      break;
    case "watching":
      base = { icon: "⚙️", title: t("terminal:automation.watching") };
      break;
    case "waitingHumanInput":
      base = { icon: "✋", title: t("terminal:automation.waitingHumanInput") };
      break;
    case "confirming":
      base = { icon: "❔", title: t("terminal:automation.confirming") };
      break;
    case "timeoutDecision":
      base = { icon: "⏳", title: t("terminal:automation.timeoutDecision") };
      break;
    case "settling":
      base = { icon: "⏳", title: t("terminal:automation.settling") };
      break;
    case "cancelling":
      base = { icon: "⏳", title: t("terminal:automation.cancelling") };
      break;
    case "cancelled":
      base = { icon: "⏹️", title: t("terminal:automation.cancelled") };
      break;
    case "interrupted":
      base = { icon: "⚠️", title: t("terminal:automation.interrupted") };
      break;
    case "completed":
      base = { icon: "✅", title: t("terminal:automation.completed") };
      break;
    case "exiting":
      base = { icon: "⚙️", title: t("terminal:automation.exiting") };
      break;
    case "done":
    default:
      base = { icon: "✅", title: t("terminal:automation.done") };
      break;
  }
  if (st.pendingReason && !base.detail) {
    base.detail =
      st.pendingReason === "humanTyping"
        ? t("terminal:automation.pendingHuman")
        : t("terminal:automation.pendingAnotherProducer");
  }
  if (st.cliContext?.state === "shell" && !base.detail)
    base.detail = t("terminal:automation.cliContextShell");
  if (st.cliContext?.state === "unknown" && !base.detail)
    base.detail = t("terminal:automation.cliContextUnknown");
  if (st.cliContext?.state === "cli" && !base.detail)
    base.detail = t("terminal:automation.cliContextCli", {
      cli: st.cliContext.cli,
    });
  return base;
}
