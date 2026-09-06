import { useState } from "react";
import { useTranslation } from "react-i18next";
import type { RepositoryAuditItem } from "@shared/types";
import { tauriApi } from "../ipc/tauriApi";
import { useEscapeToClose } from "../shared/useEscapeToClose";
import { useRepositoryAuditStore } from "./repositoryAuditStore";
import "./repositoryAudit.css";

export type AuditState =
  | "unknown"
  | "conflict"
  | "commit"
  | "diverged"
  | "push"
  | "pull"
  | "noRemote"
  | "noUpstream"
  | "clean";

/** 한 저장소에 커밋할 변경과 push할 커밋이 함께 있을 수 있으므로 상태를
 * 하나로 접지 않는다. 서버의 그룹 정렬은 유지하되 행에서는 모든 조치를 보인다. */
export function auditStates(item: RepositoryAuditItem): AuditState[] {
  if (item.unavailable || item.timedOut) return ["unknown"];
  const states: AuditState[] = [];
  if (item.conflictCount) states.push("conflict");
  if (item.ahead && item.behind) states.push("diverged");
  if (item.changedCount) states.push("commit");
  if (item.ahead && !item.behind) states.push("push");
  if (item.behind && !item.ahead) states.push("pull");
  if (!item.hasRemote) states.push("noRemote");
  else if (!item.upstream) states.push("noUpstream");
  return states.length > 0 ? states : ["clean"];
}

export function RepositoryAuditOverlay() {
  const { t } = useTranslation("app");
  const open = useRepositoryAuditStore((s) => s.open);
  const items = useRepositoryAuditStore((s) => s.items);
  const loading = useRepositoryAuditStore((s) => s.loading);
  const error = useRepositoryAuditStore((s) => s.error);
  const close = useRepositoryAuditStore((s) => s.close);
  const refresh = useRepositoryAuditStore((s) => s.refresh);
  const [terminalErrorPath, setTerminalErrorPath] = useState<string | null>(null);
  useEscapeToClose(open, close);
  if (!open) return null;
  return (
    <div
      className="repository-audit-overlay"
      onMouseDown={(e) => {
        if (e.button === 0 && e.target === e.currentTarget) close();
      }}
    >
      <section
        className="pixel-panel repository-audit-dialog"
        role="dialog"
        aria-modal="true"
        aria-label={t("repositoryAudit.aria")}
      >
        <header>
          <div>
            <h2>{t("repositoryAudit.title")}</h2>
            <p>{t("repositoryAudit.basisNote")}</p>
          </div>
          <div className="repository-audit-actions">
            <button className="pixel-btn" onClick={() => void refresh()} disabled={loading}>
              {t("repositoryAudit.refresh")}
            </button>
            <button className="pixel-btn" onClick={close}>{t("repositoryAudit.close")}</button>
          </div>
        </header>
        {loading ? (
          <p className="repository-audit-message">{t("repositoryAudit.loading")}</p>
        ) : error ? (
          <p className="repository-audit-message">{t("repositoryAudit.error")}</p>
        ) : items.length === 0 ? (
          <p className="repository-audit-message">{t("repositoryAudit.empty")}</p>
        ) : (
          <ul className="repository-audit-list">
            {items.map((item) => (
              <li key={item.path} className="repository-audit-row">
                <div className="repository-audit-identity">
                  <strong>{item.name}</strong>
                  <small>{item.branch ?? t("repositoryAudit.noBranch")} · {item.path}</small>
                  {terminalErrorPath === item.path && (
                    <small className="repository-audit-terminal-error">
                      {t("repositoryAudit.terminalError")}
                    </small>
                  )}
                </div>
                <div className="repository-audit-states">
                  {auditStates(item).map((state) => (
                    <span key={state} className={`repository-audit-state ${state}`}>
                      {t(`repositoryAudit.state.${state}`, {
                        ahead: item.ahead,
                        behind: item.behind,
                        count: item.changedCount,
                      })}
                    </span>
                  ))}
                </div>
                <button
                  className="pixel-btn"
                  onClick={() => {
                    setTerminalErrorPath(null);
                    void tauriApi.openInTerminal(item.path).catch(() => setTerminalErrorPath(item.path));
                  }}
                >
                  {t("repositoryAudit.terminal")}
                </button>
              </li>
            ))}
          </ul>
        )}
      </section>
    </div>
  );
}
