// src/renderer/discussion/MeetingBoard.tsx
//
// 회의록 보드: 토의가 이어지는 동안 오피스 회의실 옆에 참가자마다 한 줄씩 최근 발언과 "N분 전"을 보여 준다.
// 말하는 중인 참가자는 "말하는 중 · N분째". 위치는 씬이 알려 주는 회의실 화면 사각형의 오른쪽이고, 회의실이 없는
// 풍경이면 오피스 화면 왼쪽 위에 붙는다. 경과 표기는 30초마다 다시 센다(저장 상태는 건드리지 않는다).
// 접힘 여부는 이 기기에만 기억한다.
import { useEffect, useMemo, useState, type CSSProperties } from "react";
import { useTranslation } from "react-i18next";

import type { OfficeBus, ScreenRect } from "../office/bus";
import { useAppStore } from "../store/appStore";
import { useNow } from "../workpanels/AutoRunUsage";
import { useDiscussionStore } from "./discussionStore";
import { boardAge, meetingBoardRows, type BoardAge, type MeetingBoardRow } from "./meetingBoardModel";
import { stagedDiscussion } from "./officeStage";
import "./discussion.css";

const COLLAPSED_KEY = "agent-office.meetingBoard.collapsed";
/** 회의실 오른쪽 변과 보드 사이 간격(px). */
const ROOM_GAP_PX = 8;
const TICK_MS = 30_000;

function readCollapsed(): boolean {
  try {
    return localStorage.getItem(COLLAPSED_KEY) === "1";
  } catch {
    return false;
  }
}

function writeCollapsed(v: boolean): void {
  try {
    localStorage.setItem(COLLAPSED_KEY, v ? "1" : "0");
  } catch {
    // 저장소가 막혀 있으면 이번 실행 동안만 기억한다.
  }
}

export function MeetingBoard({ bus }: { bus: OfficeBus }) {
  const { t } = useTranslation("discussion");
  const discussions = useDiscussionStore((s) => s.discussions);
  const agents = useAppStore((s) => s.agents);
  const discussion = useMemo(() => stagedDiscussion(discussions), [discussions]);
  const rows = useMemo(() => (discussion ? meetingBoardRows(discussion) : []), [discussion]);
  const now = useNow(TICK_MS);
  const [rect, setRect] = useState<ScreenRect | null>(null);
  const [collapsed, setCollapsed] = useState(readCollapsed);
  useEffect(() => bus.onMeetingRoomRectChanged(setRect), [bus]);

  if (!discussion) return null;
  const style: CSSProperties | undefined = rect ? { left: Math.round(rect.x + rect.w + ROOM_GAP_PX), top: Math.round(rect.y) } : undefined;
  const age = (a: BoardAge) =>
    a.unit === "now" ? t("meetingBoard.ageNow") : t(a.unit === "minutes" ? "meetingBoard.ageMinutes" : "meetingBoard.ageHours", { count: a.count });
  const toggle = () => {
    setCollapsed((c) => {
      writeCollapsed(!c);
      return !c;
    });
  };

  return (
    <div className="meeting-board-layer">
      <section
        className={`meeting-board${rect ? "" : " docked"}${collapsed ? " collapsed" : ""}`}
        style={style}
        aria-label={t("meetingBoard.title")}
      >
        <button type="button" className="meeting-board-head" onClick={toggle} aria-expanded={!collapsed}
          title={collapsed ? t("meetingBoard.expand") : t("meetingBoard.collapse")}>
          <span className="meeting-board-title">{t("meetingBoard.title")}</span>
          <span className="meeting-board-topic">{discussion.topic}</span>
          <span className="meeting-board-state">
            {t("meetingBoard.round", { round: discussion.round })} · {t(`states.${discussion.state}`)}
          </span>
        </button>
        {!collapsed && (
          <ul className="meeting-board-rows">
            {rows.map((row) => (
              <BoardRow key={row.agentId} row={row} name={agents[row.agentId]?.name ?? t("board.missingAgent")} now={now} age={age} />
            ))}
          </ul>
        )}
      </section>
    </div>
  );
}

function BoardRow({ row, name, now, age }: { row: MeetingBoardRow; name: string; now: number; age: (a: BoardAge) => string }) {
  const { t } = useTranslation("discussion");
  let text: string;
  let when = "";
  let title: string | undefined;
  if (row.speakingSince !== undefined) {
    const minutes = Math.floor(Math.max(0, now - row.speakingSince) / 60_000);
    text = minutes < 1 ? t("meetingBoard.speaking") : t("meetingBoard.speakingFor", { minutes });
  } else if (row.last) {
    const asked = row.last.kind === "question";
    text = asked ? t("meetingBoard.question", { text: row.last.excerpt }) : row.last.excerpt;
    title = asked ? t("meetingBoard.question", { text: row.last.title }) : row.last.title;
    when = age(boardAge(row.last.at, now));
  } else {
    text = t("meetingBoard.silent");
  }
  return (
    <li className={`meeting-board-row${row.speakingSince !== undefined ? " speaking" : ""}${row.last || row.speakingSince !== undefined ? "" : " silent"}`}>
      <span className="meeting-board-name">
        {name}
        {row.moderator && <span className="meeting-board-tag">{t("meetingBoard.moderator")}</span>}
      </span>
      <span className="meeting-board-text" title={title}>{text}</span>
      {when && <span className="meeting-board-when">{when}</span>}
    </li>
  );
}
