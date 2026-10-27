import { useState } from "react";
import { useTranslation } from "react-i18next";
import { openTaskChat, taskChatBlockedReason, taskChatErrorText, useTaskChatStore } from "./taskChatStore";

/**
 * 태스크 대화 열기. 막힘 사유·열기 오류는 `hint`로 돌려주고 버튼 아래 문구로 보인다.
 * 열리면 `onOpened`(팝업 닫기)를 부른다. 기다리는 사이 다른 팝업으로 옮겼을 수 있으므로 연 태스크 id를 넘긴다. 실패하면 팝업을 열어 두어 오류를 볼 수 있게 한다.
 */
export function useTaskChatButton(taskId: string | undefined, onOpened: (taskId: string) => void) {
  // 막힘 판정은 스토어를 직접 읽으므로, 대화 목록·여는 중이 바뀔 때 다시 그리도록 구독한다.
  useTaskChatStore((s) => s.chats);
  const opening = useTaskChatStore((s) => !!taskId && !!s.opening[taskId]);
  const [error, setError] = useState<{ taskId: string; text: string }>();
  const blocked = taskId ? taskChatBlockedReason(taskId) : undefined;
  // 여는 중은 버튼 이름이 알리므로 문구를 따로 두지 않는다.
  const hint = blocked && blocked !== "task-chat-busy" ? taskChatErrorText(blocked) : error && error.taskId === taskId ? error.text : undefined;
  const open = async () => {
    if (!taskId) return;
    setError(undefined);
    const code = await openTaskChat(taskId);
    if (code) setError({ taskId, text: taskChatErrorText(code) });
    else onOpened(taskId);
  };
  return { disabled: opening || !!blocked, opening, hint, open };
}

export function TaskChatButton({ chat, label, title, hintId }: { chat: ReturnType<typeof useTaskChatButton>; label: string; title?: string; hintId: string }) {
  const { t } = useTranslation("workpanels");
  return <button type="button" className="pixel-btn work-chat-btn" disabled={chat.disabled} title={title ?? t("taskChat.openTooltip")}
    aria-describedby={chat.hint ? hintId : undefined} onClick={() => void chat.open()}>{chat.opening ? t("taskChat.opening") : label}</button>;
}
