// src/renderer/diary/diaryAutoWriter.ts
//
// 캐릭터 일기(#60) 자동 트리거. 세션 종료(exited/disposed)를 구독해, 사용자가
// 버튼을 누르지 않아도 그 세션의 작업 로그로 일기 한 편을 조용히 쓴다. 동시성·정책
// (세션당 1편, 3일 컷오프, MIN_ITEMS, in-flight/타임아웃 재시도)은 공유 DiaryFlusher가
// 맡는다 — 이 모듈은 트리거 구독 + 성공 콜백(알림·오버레이 갱신)만 담당한다.
// 같은 flusher를 일기 보기 클릭·앱 종료 경로도 공유한다.
//
// 백그라운드 유휴 스윕(#66): 세션 종료 즉시 flush가 타임아웃 등으로 실패하면 그
// 세션은 미기록으로 남는다. 종료 시점(30초 데드라인)에 몰아 쓰면 실패가 잦으므로,
// 앱이 **유휴(활성 세션 0)** 일 때 백로그를 여유롭게(일기 타임아웃 120초) 비운다.
// 트리거 둘: (a) 마지막 세션이 끝나 유휴 진입 후 정착 지연, (b) 주기 백스톱.
// 둘 다 발화 시점에 유휴를 재확인하고, 스윕은 활성 세션 CLI와 경쟁하지 않도록
// concurrency=1(순차)로 돈다.
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

/** 유휴 진입(마지막 세션 종료) 후 스윕까지의 정착 지연 — 연쇄 세션 시작과의 경합 완충. */
export const DEFAULT_SETTLE_MS = 30_000;
/** 주기 백스톱 간격 — 정착 트리거가 이벤트 유실로 놓친 백로그를 유휴 시 비운다. */
export const DEFAULT_BACKSTOP_MS = 5 * 60_000;

export interface DiaryAutoWriterDeps {
  api?: Pick<AgentOfficeApi, "onSessionState">;
  now?: () => number;
  /** 주입용 버퍼(테스트). 기본은 전역 workLog. */
  log?: WorkLog;
  /** 자동 생성 알림(테스트 주입). 기본은 OS 데스크탑 알림. */
  notify?: (title: string, body: string) => void;
  /** 일기 생성기(테스트 주입). 기본은 generateDiary. */
  generate?: typeof generateDiary;
  /** 유휴 정착 지연(ms, 테스트 주입). 기본 DEFAULT_SETTLE_MS. */
  settleMs?: number;
  /** 백스톱 간격(ms, 테스트 주입). 기본 DEFAULT_BACKSTOP_MS. */
  backstopMs?: number;
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

  const settleMs = deps.settleMs ?? DEFAULT_SETTLE_MS;
  const backstopMs = deps.backstopMs ?? DEFAULT_BACKSTOP_MS;

  // 유휴 = 활성(starting/running) 세션이 하나도 없음. 스토어의 세션 상태를 직접
  // 읽어 판정한다 — 증감 카운터가 아니라 최신 상태 집계라, 종료 이벤트를 놓쳐도
  // (크래시 등) 스토어가 정리되면 자기수복된다.
  const isIdle = (): boolean => {
    const { sessions } = useAppStore.getState();
    for (const s of Object.values(sessions)) {
      if (s.status === "running" || s.status === "starting") return false;
    }
    return true;
  };

  // 밀린 세션 백로그를 유휴일 때 순차(concurrency=1)로 비운다. 각 캐릭터 처리 전
  // 유휴를 재확인해, 스윕 도중 사용자가 세션을 시작하면 즉시 양보한다(남은 대상은
  // 다음 기회). 중첩 실행은 플래그로 막는다.
  let sweeping = false;
  const runSweep = async (): Promise<void> => {
    if (sweeping || !isIdle()) return;
    sweeping = true;
    try {
      const { agentOrder } = useAppStore.getState();
      for (const agentId of agentOrder) {
        if (!isIdle()) break;
        const opts = { includeLive: false, source: "background" as const };
        if (!flusher.hasPendingWork(agentId, opts)) continue;
        await flusher.flushAgent(agentId, opts);
      }
    } finally {
      sweeping = false;
    }
  };

  // 유휴 진입 후 정착 지연을 두고 스윕 — 연쇄 세션 시작이면 취소(clearSettle).
  let settleTimer: ReturnType<typeof setTimeout> | undefined;
  const clearSettle = (): void => {
    if (settleTimer !== undefined) {
      clearTimeout(settleTimer);
      settleTimer = undefined;
    }
  };
  const armSettle = (): void => {
    clearSettle();
    settleTimer = setTimeout(() => {
      settleTimer = undefined;
      void runSweep();
    }, settleMs);
  };

  const offSession = api.onSessionState((e) => {
    if (e.state === "exited" || e.state === "disposed") {
      // 종료 즉시 그 세션은 기존대로 바로 시도(빠른 happy path).
      void flusher.flushAgent(e.agentId, { includeLive: false, source: "session-end" });
      // 유휴로 접어들면 정착 후 백로그 스윕을 예약(발화 시점에 재확인).
      if (isIdle()) armSettle();
    } else {
      // starting/running — 유휴가 아니므로 정착 예약을 취소한다.
      clearSettle();
    }
  });

  // 주기 백스톱 — 정착 트리거가 놓친 백로그를 유휴일 때 비운다. 첫 스윕은 최소
  // 한 주기 뒤라, 부팅 직후 상태 이벤트 미도착 시점의 오판을 피한다.
  const backstopTimer = setInterval(() => {
    void runSweep();
  }, backstopMs);

  return () => {
    offSession();
    clearSettle();
    clearInterval(backstopTimer);
    setSharedDiaryFlusher(null);
  };
}
