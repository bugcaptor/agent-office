// src/renderer/webdash/WebDashIndicator.tsx
//
// 웹 상태판이 떠 있고 누가 보고 있으면 화면 아래 왼쪽에 "웹 접속 n"을, 웹에서 명령이 들어오면 짧은
// 알림을 띄운다. 클릭을 막지 않는다(UIChrome 위의 장식).
import { useTranslation } from "react-i18next";
import { useWebDashStore } from "./webDashStore";
import "./webDash.css";

export function WebDashIndicator() {
  const { t } = useTranslation("app");
  const status = useWebDashStore((s) => s.status);
  const notices = useWebDashStore((s) => s.notices);
  const clients = status?.running ? status.clients : 0;
  if (!clients && !notices.length) return null;
  return (
    <div className="web-dash-indicator" aria-live="polite">
      {notices.map((n) => (
        <div key={n.id} className="web-dash-notice pixel-panel">
          {t(`webDash.op.${n.op}`, { name: n.name ?? "", detail: n.detail ?? "", defaultValue: t("webDash.op.other", { op: n.op }) })}
        </div>
      ))}
      {clients > 0 && <div className="web-dash-clients">{t("webDash.clients", { count: clients })}</div>}
    </div>
  );
}
