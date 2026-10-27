// 태스크 대화 탭 닫기 확인. 캐릭터 탭과 달리 닫으면 세션도 끝난다.
import { useTranslation } from "react-i18next";
import { useAppStore } from "../store/appStore";
import { ConfirmDialog } from "../agent/confirmDialogs";
import { closeTaskChat, taskChatTitle, useTaskChatStore } from "./taskChatStore";

export function ConfirmCloseTaskChatDialog() {
  const { t } = useTranslation("workpanels");
  const modal = useAppStore((s) => s.modal);
  const agentId = modal.kind === "confirm-close-task-chat" ? modal.agentId : undefined;
  const chat = useTaskChatStore((s) => (agentId ? s.chats[agentId] : undefined));
  if (!agentId || !chat) return null;
  return (
    <ConfirmDialog
      slug="close-task-chat"
      title={t("taskChat.close.title")}
      confirmLabel={t("taskChat.close.confirm")}
      onConfirm={() => void closeTaskChat(agentId)}
    >
      <p>{t("taskChat.close.body", { title: taskChatTitle(chat) })}</p>
    </ConfirmDialog>
  );
}
