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
/**
 * 종료 flush가 타임아웃/in-flight로 미완이면, **유휴와 무관하게** 그 에이전트만
 * 이 간격 뒤 재시도 예약한다(#75). 재시작을 연발해 앱이 계속 활성(비유휴)이라
 * 정착·백스톱 스윕이 영영 안 도는 구간에서도 밀린 일기를 따라잡기 위함.
 */
export const DEFAULT_END_RETRY_MS = 20_000;
/**
 * 한 에이전트의 유휴-비의존 재시도 예약 상한(무한 루프 방어선). flusher 자체가
 * 타임아웃을 TIMEOUT_MAX_RETRIES로 유계화하므로 보통 그 전에 pending이 사라지지만,
 * 병리적 in-flight 반복에 대비한 하드 캡.
 */
export const DEFAULT_MAX_END_RETRIES = 6;

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
  /** 유휴-비의존 종료 재시도 간격(ms, 테스트 주입). 기본 DEFAULT_END_RETRY_MS. */
  endRetryMs?: number;
  /** 에이전트별 종료 재시도 예약 상한(테스트 주입). 기본 DEFAULT_MAX_END_RETRIES. */
  maxEndRetries?: number;
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
  const endRetryMs = deps.endRetryMs ?? DEFAULT_END_RETRY_MS;
  const maxEndRetries = deps.maxEndRetries ?? DEFAULT_MAX_END_RETRIES;

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

  // 종료 flush의 유휴-비의존 재시도(#75). 종료 즉시 시도가 타임아웃/in-flight로
  // 미완이면, 앱이 계속 활성이라 정착·백스톱 스윕이 안 돌아도 이 타이머가 그
  // 에이전트만 유계 재시도한다. hasPendingWork가 false가 되면(성공·스킵·타임아웃
  // 상한 확정) 스스로 멈춘다. 재시작 연발로 disposed 이벤트가 억제되는 세션은
  // restartAgentSession의 명시 트리거가 이 경로를 태운다.
  const endRetryTimers = new Map<string, ReturnType<typeof setTimeout>>();
  const endRetryCounts = new Map<string, number>();

  const scheduleEndRetry = (agentId: string): void => {
    if (endRetryTimers.has(agentId)) return; // 이미 예약됨.
    const count = endRetryCounts.get(agentId) ?? 0;
    if (count >= maxEndRetries) return; // 하드 캡 — 병리적 반복 방어.
    endRetryCounts.set(agentId, count + 1);
    endRetryTimers.set(
      agentId,
      setTimeout(() => {
        endRetryTimers.delete(agentId);
        void endFlush(agentId);
      }, endRetryMs),
    );
  };

  const endFlush = async (agentId: string): Promise<void> => {
    const opts = { includeLive: false, source: "session-end" as const };
    await flusher.flushAgent(agentId, opts);
    // 아직 자격 있는 미기록 세션이 남았으면(타임아웃/in-flight) 유휴와 무관하게
    // 재시도 예약, 없으면 카운터를 비워 다음 종료가 온전한 재시도 예산을 받게 한다.
    if (flusher.hasPendingWork(agentId, opts)) scheduleEndRetry(agentId);
    else endRetryCounts.delete(agentId);
  };

  const offSession = api.onSessionState((e) => {
    if (e.state === "exited" || e.state === "disposed") {
      // 새 종료 이벤트 — 온전한 재시도 예산으로 즉시 시도(빠른 happy path) +
      // 미완 시 유휴-비의존 재시도 예약.
      endRetryCounts.delete(e.agentId);
      void endFlush(e.agentId);
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
    for (const t of endRetryTimers.values()) clearTimeout(t);
    endRetryTimers.clear();
    endRetryCounts.clear();
    clearInterval(backstopTimer);
    setSharedDiaryFlusher(null);
  };
}
