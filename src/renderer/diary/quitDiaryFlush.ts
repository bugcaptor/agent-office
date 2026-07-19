// src/renderer/diary/quitDiaryFlush.ts
//
// 앱 정상 종료 시 밀린(종료된 미기록) 세션 일기를 flush하는 오케스트레이션(#60).
// ConfirmQuitDialog가 이걸 호출하고, 완료/캔슬/데드라인 중 먼저 오는 것에서
// 창을 destroy한다(데드라인·캔슬 레이스는 컴포넌트가 담당 — 여기선 순수 flush만).
//
// 핵심: (1) 시작 시 작업 로그 스냅샷을 즉시 저장(flushNow) — 캔슬해도 로그가
// 디스크에 남아 다음 실행에 이어진다. (2) 대상 에이전트를 동시 상한 아래 병렬로
// 일기화(에이전트 내부는 flusher가 직렬화). (3) 끝나면 다시 flushNow로 비워진
// 버퍼를 반영(성공 시 중복 방지 happy path — 다음 실행 복원 대상에서 빠진다).
import { useAppStore } from "../store/appStore";
import { sharedDiaryFlusher, type DiaryFlusher } from "./diaryFlusher";
import { activeWorkLogPersister, type WorkLogPersister } from "./workLogPersister";

/** 동시에 CLI를 띄우는 에이전트 상한(haiku 호출이라 이 정도는 감당). */
export const QUIT_FLUSH_CONCURRENCY = 4;
/** 종료 flush 전체 데드라인(ms). 이 시간을 넘기면 캔슬과 동일하게 그냥 종료한다.
 *  summarize 타임아웃(20초) + 여유. 로그는 이미 디스크에 있어 다음 실행에 이어진다. */
export const QUIT_FLUSH_DEADLINE_MS = 30_000;

/**
 * 지금 종료하면 일기를 써야 할(자격 있는 종료 세션이 있는) 캐릭터 목록.
 * ConfirmQuitDialog가 flushing 단계로 갈지 판단하고, 진행률 분모로 쓴다.
 * diaryEnabled OFF면 flusher.hasPendingWork가 전부 false → 빈 배열.
 */
export function pendingDiaryAgents(flusher: DiaryFlusher = sharedDiaryFlusher()): string[] {
  const { agentOrder } = useAppStore.getState();
  return agentOrder.filter((id) =>
    flusher.hasPendingWork(id, { includeLive: false, source: "quit" }),
  );
}

export interface QuitDiaryFlushDeps {
  flusher?: DiaryFlusher;
  persister?: () => WorkLogPersister | null;
  concurrency?: number;
  /** 진행 콜백(done/total). 진행률 UI 갱신용. */
  onProgress?: (done: number, total: number) => void;
}

/** items를 동시 상한 아래 병렬 실행한다(각 완료마다 fn). */
async function runPool<T>(
  items: T[],
  limit: number,
  fn: (item: T, index: number) => Promise<void>,
): Promise<void> {
  let next = 0;
  const workerCount = Math.min(limit, items.length);
  const workers = Array.from({ length: workerCount }, async () => {
    while (next < items.length) {
      const index = next++;
      await fn(items[index], index);
    }
  });
  await Promise.all(workers);
}

/**
 * 대상 캐릭터들의 밀린 세션 일기를 flush한다. 시작·종료에 작업 로그 스냅샷을
 * 저장해 캔슬/성공 어느 쪽이든 디스크 상태가 안전하게 남는다. 개별 실패는 삼킨다
 * (best-effort — 종료를 막지 않는다).
 */
export async function runQuitDiaryFlush(
  agentIds: string[],
  deps: QuitDiaryFlushDeps = {},
): Promise<void> {
  const flusher = deps.flusher ?? sharedDiaryFlusher();
  const persister = (deps.persister ?? activeWorkLogPersister)();
  const concurrency = deps.concurrency ?? QUIT_FLUSH_CONCURRENCY;

  // 캔슬 대비 보험: 현재 버퍼를 먼저 디스크에 안착시킨다.
  if (persister) await persister.flushNow().catch(() => {});

  if (agentIds.length > 0) {
    let done = 0;
    deps.onProgress?.(0, agentIds.length);
    await runPool(agentIds, concurrency, async (agentId) => {
      try {
        await flusher.flushAgent(agentId, { includeLive: false, source: "quit" });
      } catch {
        // best-effort — 한 캐릭터 실패가 종료를 막지 않는다.
      }
      done += 1;
      deps.onProgress?.(done, agentIds.length);
    });
  }

  // 성공적으로 비워진 버퍼를 반영(중복 방지 happy path). 실패해도 §1-4 dedupe가 커버.
  if (persister) await persister.flushNow().catch(() => {});
}
