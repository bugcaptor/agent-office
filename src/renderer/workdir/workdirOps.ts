// src/renderer/workdir/workdirOps.ts
//
// 진행 중인 git 조회(op)를 다루는 공용 조각. 세 슬라이스가 모두 쓴다.
//
// 배경: 거대 저장소에서는 status/diff/log가 분 단위로 걸릴 수 있어, 짧은
// 타임아웃으로 끊는 대신 **사용자 취소**를 1차 탈출구로 삼는다. 그래서 조회
// 하나마다 `opId`를 만들어 백엔드에 넘기고, 그 op가 무의미해지는 지점(상세를
// 닫거나 다른 커밋을 고르는 등)에서 여기 있는 취소 함수로 자식 git을 죽인다.
import type { StoreApi } from "zustand";

import { tauriApi } from "../ipc/tauriApi";

import type { WorkdirDetail, WorkdirRepoLog, WorkdirState } from "./workdirTypes";

/** git 조회 1건의 식별자를 만든다. 백엔드 취소 레지스트리의 키이자, 스토어가
 *  "지금 진행 중인 조회"를 가리키는 손잡이다. `crypto.randomUUID`가 없는 환경
 *  (구형 jsdom 등)에서는 카운터 기반으로 폴백한다 — 유일하기만 하면 된다. */
let opSeq = 0;
export function newOpId(): string {
  const c = globalThis.crypto;
  if (c && typeof c.randomUUID === "function") return c.randomUUID();
  return `wd-op-${Date.now()}-${++opSeq}`;
}

/** 레코드에서 키 하나를 뺀 새 레코드(진행 중 opId 해제용). */
export function omitKey<T>(record: Record<string, T>, key: string): Record<string, T> {
  if (!(key in record)) return record;
  const next = { ...record };
  delete next[key];
  return next;
}

export const HISTORY_PAGE = 50;
/** 커밋 변경파일 인라인 목록 한 페이지 크기(more…로 이어 로드). */
export const COMMIT_FILES_PAGE = 100;


export function emptyRepoLog(root: string): WorkdirRepoLog {
  return {
    root,
    query: "",
    allBranches: false,
    commits: undefined,
    loading: false,
    opId: undefined,
    hasMore: false,
    loaded: 0,
    timedOut: false,
    canceled: false,
    selectedCommit: undefined,
    files: undefined,
    filesLoading: false,
    filesOpId: undefined,
    filesCanceled: false,
    filesHasMore: false,
    filesLoaded: 0,
    selectedFile: undefined,
    fileDiff: undefined,
    fileDiffLoading: false,
    fileDiffOpId: undefined,
    gen: 0,
  };
}

/** 진행 중인 조회 하나를 취소한다(fire-and-forget). 백엔드는 없는 opId를
 *  조용히 무시하므로 실패해도 흐름에 영향이 없다. */
export function cancel(opId?: string): void {
  if (!opId) return;
  void tauriApi.workdirGitCancel(opId).catch((err) => {
    // 취소 실패는 사용자 흐름에 영향이 없다(조회가 이미 끝났을 뿐일 수 있다).
    console.warn("workdir: git cancel failed", err);
  });
}

/** 상세 페인이 걸고 있던 모든 조회를 취소한다(상세를 닫거나 다른 파일로 바꿀 때). */
export function cancelDetailOps(d: WorkdirDetail | null): void {
  if (!d) return;
  cancel(d.diffOpId);
  cancel(d.historyOpId);
  cancel(d.commitDiffOpId);
  cancel(d.commitFilesOpId);
}

/** 로그 브라우저가 걸고 있던 모든 조회를 취소한다(뷰를 떠나거나 팔레트를 닫을 때). */
export function cancelRepoLogOps(rl: WorkdirRepoLog | undefined): void {
  if (!rl) return;
  cancel(rl.opId);
  cancel(rl.filesOpId);
  cancel(rl.fileDiffOpId);
}

/** repoLog[root]를 갱신하는 헬퍼(레코드 불변 갱신). `set`은 zustand 스토어의
 *  실제 setState 타입을 그대로 받아 배리언스 문제 없이 넘길 수 있다. */
export function setRepoLog(
  set: StoreApi<WorkdirState>["setState"],
  root: string,
  next: WorkdirRepoLog,
): void {
  set((s) => ({ repoLog: { ...s.repoLog, [root]: next } }));
}
