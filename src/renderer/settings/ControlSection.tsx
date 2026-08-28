// src/renderer/settings/ControlSection.tsx
//
// 설정 다이얼로그 "제어" 탭의 CLI 제어 섹션(이슈 #55).
import { useCallback, useEffect, useState } from "react";
import { Trans, useTranslation } from "react-i18next";
import { useAppStore } from "../store/appStore";
import { tauriApi } from "../ipc/tauriApi";
import type { ControlStatus } from "@shared/types";

/**
 * CLI 제어(이슈 #55) 설정 — 2단계 옵트인. 1단계: "CLI 제어 활성화" 토글로
 * 로컬 control 서버를 켠다(control-port 기록). 2단계: "승인"으로 토큰을
 * 발급해야만 실제로 명령이 실행된다. 승인 전에는 서버가 떠 있어도 모든 요청
 * 401. 승인은 지속되며 "승인 취소"로 토큰을 폐기할 수 있다.
 */
export function ControlSection({ enabled }: { enabled: boolean }) {
  const { t } = useTranslation("settings");
  const updateAppSettings = useAppStore((s) => s.updateAppSettings);
  const [status, setStatus] = useState<ControlStatus | null>(null);
  const [busy, setBusy] = useState(false);

  const refresh = useCallback(async () => {
    try {
      setStatus(await tauriApi.controlStatus());
    } catch {
      setStatus(null);
    }
  }, []);

  useEffect(() => {
    void refresh();
  }, [refresh, enabled]);

  const approve = async () => {
    setBusy(true);
    try {
      await tauriApi.controlApprove();
      await refresh();
    } finally {
      setBusy(false);
    }
  };

  const revoke = async () => {
    setBusy(true);
    try {
      await tauriApi.controlRevoke();
      await refresh();
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="settings-form">
      <label className="settings-item">
        <input
          type="checkbox"
          checked={enabled}
          onChange={(e) => updateAppSettings({ cliEnabled: e.target.checked })}
        />
        <span>
          <strong>{t("control.title")}</strong>
          <small>
            <Trans t={t} i18nKey="control.help" components={{ b: <b />, code: <code /> }} />
          </small>
        </span>
      </label>

      {enabled && (
        <div className="settings-item" style={{ flexDirection: "column", alignItems: "stretch", gap: 8 }}>
          <div style={{ fontSize: 12, opacity: 0.85 }}>
            {t("control.statusLabel")}{" "}
            {status
              ? t("control.statusLine", {
                  server: status.running
                    ? t("control.serverRunning", { port: status.port ?? "?" })
                    : t("control.serverStopped"),
                  approval: status.approved ? t("control.approved") : t("control.unapproved"),
                })
              : t("control.loading")}
          </div>

          {status && !status.approved && (
            <button className="pixel-btn" disabled={busy} onClick={approve}>
              {t("control.approve")}
            </button>
          )}
          {status && status.approved && (
            <>
              <button className="pixel-btn" disabled={busy} onClick={revoke}>
                {t("control.revoke")}
              </button>
              <div style={{ fontSize: 12, opacity: 0.85 }}>
                <div style={{ marginBottom: 4 }}>{t("control.usageIntro")}</div>
                <code style={{ display: "block", whiteSpace: "pre-wrap" }}>
                  agent-office ctl status{"\n"}
                  agent-office ctl list{"\n"}
                  agent-office ctl send &lt;agentId&gt; "npm test" --enter
                </code>
                <div style={{ marginTop: 6, opacity: 0.7 }}>
                  {t("control.appDataNote")} <code>{status.appDataDir}</code>
                </div>
              </div>
            </>
          )}
        </div>
      )}
    </div>
  );
}
