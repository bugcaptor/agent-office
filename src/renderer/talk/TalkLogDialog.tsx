// src/renderer/talk/TalkLogDialog.tsx
//
// 동료 대화 감사 로그 열람(docs/agent-talk-design.md §7). 하단바 "대화 N"
// 메뉴의 "대화 로그 보기"로 열린다. AnalyticsDialog와 같은 셀프 게이팅
// (`modal.kind`가 talk-log일 때만 렌더)에, MemoArchiveDialog의 좌·우 2단 구성과
// Esc 닫기 훅(useEscapeToClose)을 따른다.
//
// 좌: 로그가 있는 날짜(최신순). 우: 그 날의 로그를 **대화(convId) 단위로 묶어**
// 시간순으로. 묶는 규칙은 talkLogView.ts(순수)에 있다.
//
// 로그 줄에는 받는 쪽 *이름*이 없다(TalkLogEntry는 to=agentId만 들고 있다).
// 그래서 스토어의 프로필로 이름을 풀고, 모르는 id는 id를 그대로 보여 준다
// (삭제된 캐릭터의 옛 기록도 남아 있어야 하므로 "알 수 없음"으로 뭉개지 않는다).
import { useCallback, useEffect, useRef, useState } from "react";
import type { TalkLogEntry } from "@shared/types";
import { useAppStore } from "../store/appStore";
import { tauriApi } from "../ipc/tauriApi";
import { useEscapeToClose } from "../shared/useEscapeToClose";
import { formatTalkTime, groupByConversation, kindLabel, participantsOf } from "./talkLogView";
import "./talk.css";

type LoadState =
  | { status: "loading" }
  | { status: "error" }
  | { status: "ready"; entries: TalkLogEntry[] };

export function TalkLogDialog() {
  const modal = useAppStore((s) => s.modal);
  const closeModal = useAppStore((s) => s.closeModal);
  const agents = useAppStore((s) => s.agents);

  const [dates, setDates] = useState<string[] | null>(null);
  const [date, setDate] = useState<string | null>(null);
  const [load, setLoad] = useState<LoadState>({ status: "loading" });
  // 요청 세대 카운터: 날짜를 빠르게 바꾸면 이전 조회의 늦은 응답이 최신 결과를
  // 덮을 수 있어(레이스), 최신 세대가 아닌 응답은 버린다(AnalyticsDialog와 동일).
  const genRef = useRef(0);

  const open = modal.kind === "talk-log";
  useEscapeToClose(open, closeModal);

  // 열릴 때 날짜 목록을 새로 읽고 최신 날짜를 고른다. 닫히면 상태를 비워
  // 다음에 열 때 항상 최신 로그를 보게 한다.
  useEffect(() => {
    if (!open) {
      setDates(null);
      setDate(null);
      setLoad({ status: "loading" });
      return;
    }
    let cancelled = false;
    void (async () => {
      try {
        const list = await tauriApi.listTalkLogDates();
        if (cancelled) return;
        setDates(list);
        setDate(list[0] ?? null);
        if (list.length === 0) setLoad({ status: "ready", entries: [] });
      } catch (err) {
        if (cancelled) return;
        console.warn("talk: 로그 날짜 목록 실패", err);
        setDates([]);
        setLoad({ status: "error" });
      }
    })();
    return () => {
      cancelled = true;
    };
  }, [open]);

  const fetchDate = useCallback(async (d: string) => {
    const gen = ++genRef.current;
    setLoad({ status: "loading" });
    try {
      const entries = await tauriApi.readTalkLog(d);
      if (gen !== genRef.current) return; // 낡은 응답
      setLoad({ status: "ready", entries });
    } catch (err) {
      if (gen !== genRef.current) return;
      console.warn("talk: 로그 읽기 실패", err);
      setLoad({ status: "error" });
    }
  }, []);

  useEffect(() => {
    if (open && date) void fetchDate(date);
  }, [open, date, fetchDate]);

  if (!open) return null;

  const nameOf = (agentId: string): string => agents[agentId]?.name ?? agentId;
  const groups = load.status === "ready" ? groupByConversation(load.entries) : [];

  return (
    <div
      className="modal-backdrop"
      onMouseDown={(e) => {
        if (e.button === 0 && e.target === e.currentTarget) closeModal();
      }}
    >
      <div className="pixel-panel talk-log-dialog" role="dialog" aria-label="동료 대화 로그">
        <div className="talk-log-head">
          <h2 className="pixel-title">💬 동료 대화 로그</h2>
          <button type="button" className="pixel-btn" onClick={closeModal}>
            닫기
          </button>
        </div>

        <div className="talk-log-body">
          <ul className="talk-log-dates">
            {(dates ?? []).map((d) => (
              <li key={d}>
                <button
                  type="button"
                  className={
                    d === date ? "talk-log-date talk-log-date-active" : "talk-log-date"
                  }
                  aria-pressed={d === date}
                  onClick={() => setDate(d)}
                >
                  {d}
                </button>
              </li>
            ))}
          </ul>

          <div className="talk-log-view">
            {load.status === "loading" && <p className="talk-log-msg">불러오는 중…</p>}
            {load.status === "error" && (
              <p className="talk-log-msg">대화 로그를 불러오지 못했습니다.</p>
            )}
            {load.status === "ready" && groups.length === 0 && (
              <p className="talk-log-msg">기록된 대화가 없습니다.</p>
            )}
            {load.status === "ready" &&
              groups.map((g) => (
                <section key={g.convId} className="talk-log-conv">
                  <h3 className="talk-log-conv-head">
                    <span className="talk-log-conv-id">conv={g.convId}</span>
                    <span className="talk-log-conv-who">
                      {participantsOf(g, nameOf).join(" ↔ ")}
                    </span>
                  </h3>
                  <ul className="talk-log-lines">
                    {g.entries.map((e) => (
                      <li
                        key={`${e.kind}:${e.id}:${e.at}`}
                        className={
                          e.kind === "expire" ? "talk-log-line talk-log-line-expire" : "talk-log-line"
                        }
                      >
                        <span className="talk-log-when">{formatTalkTime(e.at)}</span>
                        <span className="talk-log-who">
                          {e.fromName} → {nameOf(e.to)}
                        </span>
                        <span className="talk-log-kind">{kindLabel(e.kind)}</span>
                        <span className="talk-log-text">{e.text}</span>
                      </li>
                    ))}
                  </ul>
                </section>
              ))}
          </div>
        </div>
      </div>
    </div>
  );
}
