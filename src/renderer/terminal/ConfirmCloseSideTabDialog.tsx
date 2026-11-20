// 곁 세션 탭(태스크 대화·토의 참가자) 닫기 확인. 캐릭터 탭과 달리 닫으면 세션도 끝난다.
// 확인창이 떠 있는 사이 탭이 스스로 사라지면(토의 실행기가 탭을 닫는 등) 모달도 닫는다. 그대로 두면
// 보이지 않는 모달이 단축키를 막는다.
import { useEffect } from "react";
import { useTranslation } from "react-i18next";
import { useAppStore } from "../store/appStore";
import { ConfirmDialog } from "../agent/confirmDialogs";
import { closeTaskChat, taskChatTitle, useTaskChatStore } from "../workpanels/taskChatStore";
import { closeDiscussionSession, discussionSessionTitle, useDiscussionSessionStore } from "../discussion/discussionSessionStore";

export function ConfirmCloseSideTabDialog() {
  const { t } = useTranslation(["workpanels", "discussion"]);
  const modal = useAppStore((s) => s.modal);
  const closeModal = useAppStore((s) => s.closeModal);
  const chatId = modal.kind === "confirm-close-task-chat" ? modal.agentId : undefined;
  const discussionId = modal.kind === "confirm-close-discussion-session" ? modal.agentId : undefined;
  const chat = useTaskChatStore((s) => (chatId ? s.chats[chatId] : undefined));
  const session = useDiscussionSessionStore((s) => (discussionId ? s.sessions[discussionId] : undefined));
  const gone = (!!chatId && !chat) || (!!discussionId && !session);
  useEffect(() => { if (gone) closeModal(); }, [gone, closeModal]);

  if (chatId && chat) {
    return <ConfirmDialog slug="close-task-chat" title={t("workpanels:taskChat.close.title")}
      confirmLabel={t("workpanels:taskChat.close.confirm")} onConfirm={() => void closeTaskChat(chatId)}>
      <p>{t("workpanels:taskChat.close.body", { title: taskChatTitle(chat) })}</p>
    </ConfirmDialog>;
  }
  if (discussionId && session) {
    return <ConfirmDialog slug="close-discussion-session" title={t("discussion:session.close.title")}
      confirmLabel={t("discussion:session.close.confirm")} onConfirm={() => void closeDiscussionSession(discussionId)}>
      <p>{t("discussion:session.close.body", { title: discussionSessionTitle(session) })}</p>
    </ConfirmDialog>;
  }
  return null;
}
