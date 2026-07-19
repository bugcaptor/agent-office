// src/renderer/diary/diaryAutoWriter.ts
//
// 캐릭터 일기(#60) 자동 트리거. #56은 수동("일기 쓰기" 버튼)만 있었는데, 이 모듈은
// 세션 종료(exited/disposed)를 구독해 사용자가 버튼을 누르지 않아도 그 세션의
// 작업 로그로 일기 한 편을 조용히 append한다. 게이트는 기존 diaryEnabled 재사용
// (요약기·수동과 같은 opt-in). soundManager의 설치 패턴(onSessionState 구독 +
// 해제 함수)을 미러한다.
//
// 정책(이슈 #60 요구):
// - 세션 종료가 기본 트리거(세션당 1편). "턴 종료"마다 쓰면 너무 잦고 비싸다.
// - 트리거 시, 이전에 놓친 세션 로그가 버퍼에 남아 있으면 그 사이 것도 함께
//   기록한다("누락된 그 사이의 기록"). 단 마지막 활동이 3일보다 오래된 세션은
//   자동 생성하지 않는다(수동으로만 — 오래된 과거를 소급 자동 생성하지 않음).
// - 작업량이 극히 적은 세션(항목 수 < AUTO_DIARY_MIN_ITEMS)은 건너뛴다.
// - 자동 생성 성공 시 OS 알림을 보낸다(확인 다이얼로그가 아니라 노티피케이션).
// - 오버레이가 그 캐릭터를 열고 있으면 목록을 갱신한다.
//
// diaryEnabled=false면 generateDiary가 CLI를 호출하지 않고 disabled로 폴백하므로
// 자동 생성은 전혀 일어나지 않는다. CLI 미설치·실패·타임아웃도 조용한 폴백이다.
import { useAppStore } from "../store/appStore";
import { tauriApi } from "../ipc/tauriApi";
import { maybeSendOsNotification } from "../ipc/osNotify";
import { useDiaryStore } from "./diaryStore";
import { generateDiary } from "./diaryGenerator";
import { workLog, type WorkLog } from "./workLog";
import type { AgentOfficeApi, DiaryEntry } from "@shared/types";

/** 자동 생성 대상의 최대 나이. 마지막 활동이 이보다 오래되면 수동으로만 기록. */
export const AUTO_DIARY_MAX_AGE_MS = 3 * 24 * 60 * 60 * 1000; // 3일
/** "작업량이 극히 적은" 세션 임계값 — 로그 항목이 이보다 적으면 건너뛴다. */
export const AUTO_DIARY_MIN_ITEMS = 3;

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
  const now = deps.now ?? Date.now;
  const log = deps.log ?? workLog;
  const notify = deps.notify ?? ((title, body) => void maybeSendOsNotification(title, body));
  const generate = deps.generate ?? generateDiary;

  // 이미 자동 처리(시도)한 (agentId,sessionId) — 이중 이벤트(exited→disposed)와
  // 스트래글러 재스캔에서 같은 세션을 두 번 생성하지 않게 한다. 성공 시엔 로그가
  // 소진돼 재등장하지 않지만, 실패/스킵도 재시도하지 않도록(조용한 폴백) 표시한다.
  const attempted = new Set<string>();

  async function handleAgent(agentId: string): Promise<void> {
    // 게이트를 먼저 확인 — OFF면 스캔조차 하지 않는다(CLI 미호출).
    if (!useAppStore.getState().appSettings.diaryEnabled) return;

    // 이 캐릭터의 버퍼를 세션별로 묶는다(종료된 세션 + 이전에 놓친 스트래글러).
    const bySession = new Map<string, { count: number; latestAt: number }>();
    for (const item of log.items(agentId)) {
      const g = bySession.get(item.sessionId);
      if (g) {
        g.count += 1;
        if (item.at > g.latestAt) g.latestAt = item.at;
      } else {
        bySession.set(item.sessionId, { count: 1, latestAt: item.at });
      }
    }

    const cutoff = now() - AUTO_DIARY_MAX_AGE_MS;
    for (const [sessionId, g] of bySession) {
      const key = `${agentId}:${sessionId}`;
      if (attempted.has(key)) continue;
      // 3일보다 오래된 과거는 수동으로만 — 소급 자동 생성 금지.
      if (g.latestAt < cutoff) {
        attempted.add(key);
        continue;
      }
      // 작업량이 극히 적은 세션은 제외.
      if (g.count < AUTO_DIARY_MIN_ITEMS) {
        attempted.add(key);
        continue;
      }
      attempted.add(key);
      let result;
      try {
        result = await generate(agentId, {}, sessionId);
      } catch (err) {
        console.warn(`diaryAutoWriter: 자동 일기 생성 예외(agent=${agentId})`, err);
        continue;
      }
      if (!result.ok) continue;
      onWritten(agentId, result.entry);
    }
  }

  function onWritten(agentId: string, entry: DiaryEntry): void {
    const state = useAppStore.getState();
    const name = state.agents[agentId]?.name ?? "캐릭터";
    notify(`📔 ${name}의 일기`, previewBody(entry.body));
    // 오버레이가 이 캐릭터를 열고 있으면 새 일기를 즉시 반영.
    const diary = useDiaryStore.getState();
    if (diary.overlay?.agentId === agentId) void diary.refresh(agentId);
  }

  const offSession = api.onSessionState((e) => {
    if (e.state !== "exited" && e.state !== "disposed") return;
    void handleAgent(e.agentId);
  });

  return () => {
    offSession();
    attempted.clear();
  };
}
