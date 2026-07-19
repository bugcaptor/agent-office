// src/renderer/diary/diaryAutoWriter.ts
//
// 캐릭터 일기(#60) 자동 트리거. 세션 종료(exited/disposed)를 구독해, 사용자가
// 버튼을 누르지 않아도 그 세션의 작업 로그로 일기 한 편을 조용히 쓴다. 동시성·정책
// (세션당 1편, 3일 컷오프, MIN_ITEMS, in-flight 재시도)은 공유 DiaryFlusher가
// 맡는다 — 이 모듈은 세션 종료 구독 + 성공 콜백(알림·오버레이 갱신)만 담당하는
// 얇은 껍데기다. 같은 flusher를 일기 보기 클릭·앱 종료 경로도 공유한다.
//
// diaryEnabled=false면 flusher가 CLI를 호출하지 않고 조용히 폴백하므로 자동 생성은
// 전혀 일어나지 않는다. CLI 미설치·실패·타임아웃도 조용한 폴백이다.
import { useAppStore } from "../store/appStore";
import { tauriApi } from "../ipc/tauriApi";
import { maybeSendOsNotification } from "../ipc/osNotify";
import { useDiaryStore } from "./diaryStore";
import { generateDiary } from "./diaryGenerator";
import { DiaryFlusher, setSharedDiaryFlusher } from "./diaryFlusher";
import { workLog, type WorkLog } from "./workLog";
import type { AgentOfficeApi, DiaryEntry } from "@shared/types";

// 정책 상수는 flusher가 정본. 기존 임포터(테스트) 호환을 위해 재export.
export { AUTO_DIARY_MAX_AGE_MS, AUTO_DIARY_MIN_ITEMS } from "./diaryFlusher";

export interface DiaryAutoWriterDeps {
  api?: Pick<AgentOfficeApi, "onSessionState">;
  now?: () => number;
  /** 주입용 버퍼(테스트). 기본은 전역 workLog. */
  log?: WorkLog;
  /** 자동 생성 알림(테스트 주입). 기본은 OS 데스크탑 알림. */
  notify?: (title: string, body: string) => void;
  /** 일기 생성기(테스트 주입). 기본은 generateDiary. */
  generate?: typeof generateDiary;
}

/** 알림 본문용: 일기 본문 앞부분을 한 줄로 자른다. */
function previewBody(body: string): string {
  const line = body.replace(/\s+/g, " ").trim();
  const chars = Array.from(line);
  return chars.length > 40 ? `${chars.slice(0, 40).join("")}…` : line;
}

/**
 * 세션 종료 시 자동으로 일기를 쓰게 설치한다. 앱 부트에서 1회 호출(bootstrap.ts).
 * 해제 함수를 돌려준다. deps는 테스트 주입용 — 실제 앱은 인자 없이 부른다.
 */
export function installDiaryAutoWriter(deps: DiaryAutoWriterDeps = {}): () => void {
  const api = deps.api ?? tauriApi;
  const log = deps.log ?? workLog;
  const notify = deps.notify ?? ((title, body) => void maybeSendOsNotification(title, body));

  // 생성 성공 시: OS 알림 + 오버레이가 그 캐릭터를 열고 있으면 갱신.
  const onWritten = (agentId: string, entry: DiaryEntry): void => {
    const state = useAppStore.getState();
    const name = state.agents[agentId]?.name ?? "캐릭터";
    notify(`📔 ${name}의 일기`, previewBody(entry.body));
    const diary = useDiaryStore.getState();
    if (diary.overlay?.agentId === agentId) void diary.refresh(agentId);
  };

  // 공유 flusher를 이 콜백/주입 deps로 구성해 전역에 등록 — 일기 보기·앱 종료
  // 경로가 같은 attempted/running을 공유하게 된다.
  const flusher = new DiaryFlusher({
    now: deps.now,
    log,
    generate: deps.generate,
    onWritten,
  });
  setSharedDiaryFlusher(flusher);

  const offSession = api.onSessionState((e) => {
    if (e.state !== "exited" && e.state !== "disposed") return;
    void flusher.flushAgent(e.agentId, { includeLive: false, source: "session-end" });
  });

  return () => {
    offSession();
    setSharedDiaryFlusher(null);
  };
}
