// src/renderer/diary/diaryFlusher.ts
//
// 캐릭터 일기(#60) 공통 flush 엔진. 원래 diaryAutoWriter 클로저에 갇혀 있던
// 동시성 장치(attempted Set + running Map + handleAgent 로직)를 여기로 추출해,
// 세 트리거 — (a) 세션 종료(자동), (b) 일기 보기 클릭, (c) 앱 종료 — 가 전부 같은
// 직렬화기를 타게 한다. 트리거마다 별도 장치를 만들면 generateDiary의 per-agent
// inflight와 얽혀 일기가 유실될 수 있어, 단일 인스턴스로 통일한다.
//
// 정책(이슈 #60):
// - 세션당 1편. 같은 (agentId,sessionId)는 attempted로 1회만 시도(성공/스킵 모두).
//   단 in-flight(다른 경로가 이 캐릭터를 쓰는 중)만은 표시하지 않고 다음에 재시도.
// - 진행 중(running) 세션은 제외(includeLive=false 기본) — 지금 일기화하면 로그가
//   소진돼 세션이 실제 끝날 때 일기가 반토막 난다. 진행 중 세션 즉시 기록은 수동
//   "일기 쓰기" 버튼(writeNow, flusher를 안 탐)의 몫이다.
// - 마지막 활동이 3일보다 오래된 과거는 자동 생성하지 않는다(소급 금지).
// - 작업량이 극히 적은 세션(항목 < MIN_ITEMS)은 건너뛴다.
// - 부팅 복원된 세션은 이미 일기가 있으면 재생성하지 않는다(중복 방지, §1-4).
import { useAppStore } from "../store/appStore";
import { tauriApi } from "../ipc/tauriApi";
import { generateDiary } from "./diaryGenerator";
import { workLog, type WorkLog } from "./workLog";
import { restoredSessionKeys } from "./workLogPersister";
import type { DiaryEntry } from "@shared/types";

/** 자동/종료 생성 대상의 최대 나이. 마지막 활동이 이보다 오래되면 수동으로만 기록. */
export const AUTO_DIARY_MAX_AGE_MS = 3 * 24 * 60 * 60 * 1000; // 3일
/** "작업량이 극히 적은" 세션 임계값 — 로그 항목이 이보다 적으면 건너뛴다. */
export const AUTO_DIARY_MIN_ITEMS = 3;

/** flush를 부른 계기 — 로깅/의미 구분용(동작 분기는 하지 않는다). */
export type FlushSource = "session-end" | "open-diary" | "quit" | "background";

/** 한 세션의 타임아웃 재시도 상한. 초과 시 attempted 확정(무한 재시도 방지, #66). */
export const TIMEOUT_MAX_RETRIES = 2;

export interface FlushAgentOpts {
  /** 진행 중(running) 세션도 포함할지. 기본 false — 종료된 세션만 쓴다. */
  includeLive?: boolean;
  source: FlushSource;
}

export interface DiaryFlusherDeps {
  now?: () => number;
  /** 주입용 작업 로그 버퍼(테스트). 기본은 전역 workLog. */
  log?: WorkLog;
  /** 일기 생성기(테스트 주입). 기본은 generateDiary. */
  generate?: typeof generateDiary;
  /** 중복 검사용 기존 일기 로드(테스트 주입). 기본은 tauriApi.loadDiary. */
  loadDiary?: (agentId: string) => Promise<DiaryEntry[]>;
  /** 생성 성공 콜백(알림·오버레이 갱신 등). autoWriter가 주입한다. */
  onWritten?: (agentId: string, entry: DiaryEntry) => void;
}

/**
 * 여러 트리거가 공유하는 일기 flush 엔진. attempted/running을 인스턴스에 보관한다.
 */
export class DiaryFlusher {
  private readonly now: () => number;
  private readonly log: WorkLog;
  private readonly generate: typeof generateDiary;
  private readonly loadDiary: (agentId: string) => Promise<DiaryEntry[]>;
  private readonly onWritten?: (agentId: string, entry: DiaryEntry) => void;

  // 이미 처리(시도)한 (agentId,sessionId). 이중 이벤트·재스캔에서 두 번 쓰지 않게.
  private readonly attempted = new Set<string>();
  // 에이전트별 처리 직렬화. 겹친 트리거가 generateDiary의 per-agent inflight와
  // 얽혀 세션을 잃지 않도록 앞 처리가 끝난 뒤에만 다음 처리가 돌게 한다.
  private readonly running = new Map<string, Promise<void>>();
  // 세션별(agentId:sessionId) 타임아웃 재시도 횟수. 상한(TIMEOUT_MAX_RETRIES)
  // 초과 시 attempted로 확정한다 — 근본적으로 느린 CLI에서 같은 세션을 영원히
  // 두드리지 않도록. 인스턴스(=앱 세션) 수명 한정이라 재부팅 시 리셋(의도).
  private readonly timeoutRetries = new Map<string, number>();

  constructor(deps: DiaryFlusherDeps = {}) {
    this.now = deps.now ?? Date.now;
    this.log = deps.log ?? workLog;
    this.generate = deps.generate ?? generateDiary;
    this.loadDiary = deps.loadDiary ?? ((id) => tauriApi.loadDiary(id));
    this.onWritten = deps.onWritten;
  }

  /**
   * 한 캐릭터의 자격 있는 미기록 세션들을 일기화한다. 같은 캐릭터의 앞선 flush가
   * 끝난 뒤 실행된다(직렬화). 반환 Promise는 이 호출 몫의 완료 — 종료 flush가
   * Promise.all로 기다릴 수 있다.
   */
  flushAgent(agentId: string, opts: FlushAgentOpts): Promise<void> {
    const prev = this.running.get(agentId) ?? Promise.resolve();
    const next = prev.then(() => this.handle(agentId, opts)).catch((err) => {
      console.warn(`diaryFlusher: flush 실패(agent=${agentId})`, err);
    });
    this.running.set(agentId, next);
    void next.finally(() => {
      if (this.running.get(agentId) === next) this.running.delete(agentId);
    });
    return next;
  }

  /**
   * 지금 flushAgent를 부르면 실제로 일기를 쓸 자격 있는 세션이 있는지(읽기 전용,
   * 부수효과 없음). UI가 "밀린 일기 쓰는 중" 배지를 깜빡임 없이 켜거나, 앱 종료 시
   * flushing 단계로 갈지 판단하는 데 쓴다. handle의 게이트를 거울처럼 따른다.
   */
  hasPendingWork(agentId: string, opts: FlushAgentOpts): boolean {
    if (!useAppStore.getState().appSettings.diaryEnabled) return false;
    const liveSid = opts.includeLive ? undefined : this.liveSession(agentId);
    const cutoff = this.now() - AUTO_DIARY_MAX_AGE_MS;
    const bySession = new Map<string, { count: number; latestAt: number }>();
    for (const item of this.log.items(agentId)) {
      const g = bySession.get(item.sessionId);
      if (g) {
        g.count += 1;
        if (item.at > g.latestAt) g.latestAt = item.at;
      } else {
        bySession.set(item.sessionId, { count: 1, latestAt: item.at });
      }
    }
    for (const [sessionId, g] of bySession) {
      if (this.attempted.has(`${agentId}:${sessionId}`)) continue;
      if (sessionId === liveSid) continue;
      if (g.latestAt < cutoff) continue;
      if (g.count < AUTO_DIARY_MIN_ITEMS) continue;
      return true;
    }
    return false;
  }

  /** 이 캐릭터의 진행 중(running) 세션 id — 있으면 includeLive=false에서 제외한다. */
  private liveSession(agentId: string): string | undefined {
    const st = useAppStore.getState();
    const sid = st.taskLabels[agentId]?.sessionId;
    if (sid && st.sessions[agentId]?.status === "running") return sid;
    return undefined;
  }

  private async handle(agentId: string, opts: FlushAgentOpts): Promise<void> {
    // 게이트를 먼저 확인 — OFF면 스캔조차 하지 않는다(CLI 미호출).
    if (!useAppStore.getState().appSettings.diaryEnabled) return;

    const liveSid = opts.includeLive ? undefined : this.liveSession(agentId);

    // 이 캐릭터의 버퍼를 세션별로 묶는다(종료된 세션 + 이전에 놓친 스트래글러).
    const bySession = new Map<string, { count: number; latestAt: number }>();
    for (const item of this.log.items(agentId)) {
      const g = bySession.get(item.sessionId);
      if (g) {
        g.count += 1;
        if (item.at > g.latestAt) g.latestAt = item.at;
      } else {
        bySession.set(item.sessionId, { count: 1, latestAt: item.at });
      }
    }

    const cutoff = this.now() - AUTO_DIARY_MAX_AGE_MS;
    // 중복 검사용 기존 일기 — 복원 세션을 처음 만날 때 1회만 로드(메모).
    let existingDiaries: DiaryEntry[] | null = null;

    for (const [sessionId, g] of bySession) {
      const key = `${agentId}:${sessionId}`;
      if (this.attempted.has(key)) continue;
      // 진행 중 세션은 제외 — attempted 표시하지 않아, 나중에 종료되면 그때 쓴다.
      if (sessionId === liveSid) continue;
      // 3일보다 오래된 과거는 수동으로만 — 소급 자동 생성 금지. 타임아웃 재시도로
      // 미뤄지다 컷오프를 넘긴 세션도 여기서 확정된다(재시도가 소급 금지를 우회 못 함).
      if (g.latestAt < cutoff) {
        this.attempted.add(key);
        this.timeoutRetries.delete(key);
        continue;
      }
      // 작업량이 극히 적은 세션은 제외.
      if (g.count < AUTO_DIARY_MIN_ITEMS) {
        this.attempted.add(key);
        this.timeoutRetries.delete(key);
        continue;
      }

      // 부팅 복원된 세션이면 이미 일기가 있는지 확인 — 크래시로 clear 스냅샷이
      // 유실된 경우의 중복 생성을 막는다. 이미 있으면 로그만 소진하고 스킵.
      if (restoredSessionKeys.has(key)) {
        if (existingDiaries === null) {
          existingDiaries = await this.loadDiary(agentId).catch(() => []);
        }
        const already = existingDiaries.some(
          (e) => e.sessionId === sessionId && e.at >= g.latestAt,
        );
        if (already) {
          this.log.clear(agentId, sessionId);
          restoredSessionKeys.delete(key);
          this.attempted.add(key);
          continue;
        }
      }

      let result;
      try {
        result = await this.generate(agentId, {}, sessionId);
      } catch (err) {
        this.attempted.add(key); // 예외는 재시도하지 않는다(조용한 폴백).
        console.warn(`diaryFlusher: 일기 생성 예외(agent=${agentId})`, err);
        continue;
      }
      // in-flight면 표시하지 않고 다음 트리거에서 재시도 — 자격 있는 일기를
      // 잃지 않는다(상한 없음: 다른 경로가 곧 놓아준다).
      if (!result.ok && result.reason === "in-flight") continue;
      // 타임아웃도 재시도 가능하되 세션당 상한까지만(#66) — 백그라운드 스윕이
      // 다음 유휴에 다시 시도한다. 상한 내면 표시하지 않고 넘어간다.
      if (!result.ok && result.reason === "timeout") {
        const tries = (this.timeoutRetries.get(key) ?? 0) + 1;
        if (tries <= TIMEOUT_MAX_RETRIES) {
          this.timeoutRetries.set(key, tries);
          continue;
        }
        // 상한 초과 — 아래에서 attempted 확정(조용한 폴백).
        console.warn(`diaryFlusher: 타임아웃 재시도 상한 초과 — 포기(${key})`);
      }
      // 그 외(성공·disabled·cli-missing·failed·타임아웃 상한초과)는 표시해 재시도 방지.
      this.attempted.add(key);
      this.timeoutRetries.delete(key);
      restoredSessionKeys.delete(key);
      if (result.ok) this.onWritten?.(agentId, result.entry);
    }
  }
}

// 앱 전역 공유 flusher — 세션 종료(autoWriter)·일기 보기·앱 종료가 같은
// attempted/running을 공유하게 하는 단일 인스턴스. autoWriter가 설치될 때 자신의
// 콜백(알림·오버레이)을 담은 인스턴스로 교체한다(setSharedDiaryFlusher).
let shared: DiaryFlusher | null = null;

/** 공유 flusher를 반환한다(없으면 기본 인스턴스를 lazily 생성). */
export function sharedDiaryFlusher(): DiaryFlusher {
  if (!shared) shared = new DiaryFlusher();
  return shared;
}

/** 공유 flusher를 지정한다(autoWriter 설치용). null이면 기본으로 되돌린다. */
export function setSharedDiaryFlusher(flusher: DiaryFlusher | null): void {
  shared = flusher;
}
