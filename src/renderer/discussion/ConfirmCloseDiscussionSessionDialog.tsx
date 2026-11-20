// 토의 참가자 탭 닫기 확인. 닫으면 그 참가자의 세션이 끝나고 토의에서 빠진다.
import { useTranslation } from "react-i18next";
import { useAppStore } from "../store/appStore";
import { ConfirmDialog } from "../agent/confirmDialogs";
import { closeDiscussionSession, discussionSessionTitle, useDiscussionSessionStore } from "./discussionSessionStore";

export function ConfirmCloseDiscussionSessionDialog() {
  const { t } = useTranslation("discussion");
  const modal = useAppStore((s) => s.modal);
  const agentId = modal.kind === "confirm-close-discussion-session" ? modal.agentId : undefined;
  const session = useDiscussionSessionStore((s) => (agentId ? s.sessions[agentId] : undefined));
  if (!agentId || !session) return null;
  return (
    <ConfirmDialog
      slug="close-discussion-session"
      title={t("session.close.title")}
      confirmLabel={t("session.close.confirm")}
      onConfirm={() => void closeDiscussionSession(agentId)}
    >
      <p>{t("session.close.body", { title: discussionSessionTitle(session) })}</p>
    </ConfirmDialog>
  );
}
