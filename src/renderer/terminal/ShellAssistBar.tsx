// src/renderer/terminal/ShellAssistBar.tsx
//
// 셸 캐릭터 터미널 위의 한 줄 입력: 말로 쓰면 LLM이 명령 하나를 만들고, 그 명령을 xterm
// 입력줄에 채워 둔다. 실행(Enter)은 사람이 터미널에서 누른다 — 웹의 [실행]과 같은 원칙.
import { useState } from "react";
import { useTranslation } from "react-i18next";
import type { ShellSuggestion } from "@shared/types";
import { tauriApi } from "../ipc/tauriApi";
import { backendErrorText } from "../shared/backendError";
import { IS_WINDOWS } from "../shared/platform";
import { terminalRegistry } from "./TerminalRegistry";

/** 셸이 이미 쓰다 만 줄을 비우는 키. zsh는 줄 전체, bash는 커서 앞을 지운다. PowerShell(PSReadLine
 *  Windows 모드)에는 같은 기본 키가 없어 비우지 않는다. */
const CLEAR_LINE = IS_WINDOWS ? "" : "\x15";

export function ShellAssistBar({ agentId }: { agentId: string }) {
  const { t } = useTranslation("terminal");
  const [text, setText] = useState("");
  const [busy, setBusy] = useState(false);
  const [last, setLast] = useState<ShellSuggestion | null>(null);
  const [error, setError] = useState<string | null>(null);

  const submit = async () => {
    const request = text.trim();
    if (!request || busy) return;
    setBusy(true);
    setError(null);
    try {
      const s = await tauriApi.shellAssistSuggest(agentId, request);
      setLast(s);
      if (!s.multiline) {
        tauriApi.writeInput(agentId, CLEAR_LINE + s.command, "human");
        setText("");
        terminalRegistry.get(agentId)?.term.focus();
      }
    } catch (e) {
      setLast(null);
      setError(backendErrorText(e, ERROR_KEYS));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="shell-assist-bar">
      <form
        className="shell-assist-row"
        onSubmit={(e) => {
          e.preventDefault();
          void submit();
        }}
      >
        <span className="shell-assist-icon" aria-hidden="true">›_</span>
        <input
          type="text"
          value={text}
          disabled={busy}
          placeholder={t("shellAssist.placeholder")}
          aria-label={t("shellAssist.placeholder")}
          onChange={(e) => setText(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Escape") terminalRegistry.get(agentId)?.term.focus();
          }}
        />
        <button type="submit" className="pixel-btn" disabled={busy || !text.trim()}>
          {busy ? t("shellAssist.making") : t("shellAssist.make")}
        </button>
      </form>
      {error && <div className="shell-assist-msg shell-assist-error">{error}</div>}
      {last && !error && (
        <div className={last.risky ? "shell-assist-msg shell-assist-risky" : "shell-assist-msg"}>
          {last.multiline ? (
            <>
              {t("shellAssist.multiline")} <code>{last.command}</code>
            </>
          ) : (
            <>
              {last.risky && `${t("shellAssist.risky")} `}
              {last.explanation || t("shellAssist.filled")}
              {last.interactive && ` ${t("shellAssist.interactive")}`}
              <span className="shell-assist-source"> · {last.source}</span>
            </>
          )}
        </div>
      )}
    </div>
  );
}

const ERROR_KEYS: Readonly<Record<string, string>> = {
  "shell-llm-none": "terminal:shellAssist.errorNone",
  "shell-llm-failed": "terminal:shellAssist.errorFailed",
  "shell-not-enabled": "terminal:shellAssist.errorNotEnabled",
  "not-ready": "terminal:shellAssist.errorNotReady",
};
