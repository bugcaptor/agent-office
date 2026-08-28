// src/renderer/workdir/slices/repoLogSlice.ts
//
// 저장소 전체 커밋 로그 브라우저(이슈 #54, 2단계). 파일을 먼저 지목하지 않고
// 로그 → 커밋 → 변경파일 → diff 순으로 훑는다.
//
// 상세 페인과 화면이 닮았지만 상태 그릇이 다르다: 상세는 "지금 연 파일 하나"
// (`detail`)에 매달려 있고, 여기는 **root별 캐시**(`repoLog[root]`)라 팔레트를
// 닫았다 열어도 보던 자리가 남는다. 그래서 두 축의 페이지네이션은 모양만 닮았을
// 뿐 같은 코드로 묶이지 않는다(필드 이름도 staleness 판정도 다르다).
import { tauriApi } from "../../ipc/tauriApi";

import {
  COMMIT_FILES_PAGE,
  HISTORY_PAGE,
  cancel,
  cancelRepoLogOps,
  emptyRepoLog,
  newOpId,
  setRepoLog,
} from "../workdirOps";
import type { WorkdirRepoLogSlice, WorkdirSlice } from "../workdirTypes";

export const createRepoLogSlice: WorkdirSlice<WorkdirRepoLogSlice> = (set, get) => ({
  repoLog: {},

  loadRepoLog: async (reset) => {
    const p = get().palette;
    if (!p) return;
    const root = p.root;
    const prev = get().repoLog[root] ?? emptyRepoLog(root);
    const skip = reset ? 0 : prev.loaded;
    const gen = reset ? prev.gen + 1 : prev.gen;
    // reset(검색어/브랜치 전환)이면 진행 중이던 조회는 전부 버려질 응답이다.
    if (reset) cancelRepoLogOps(prev);
    else cancel(prev.opId);
    const opId = newOpId();
    setRepoLog(set, root, {
      ...prev,
      loading: true,
      opId,
      canceled: false,
      ...(reset
        ? {
            commits: undefined,
            selectedCommit: undefined,
            files: undefined,
            filesOpId: undefined,
            filesCanceled: false,
            selectedFile: undefined,
            fileDiff: undefined,
            fileDiffOpId: undefined,
            gen,
          }
        : {}),
    });
    try {
      const res = await tauriApi.workdirRepoLog(
        root,
        HISTORY_PAGE,
        skip,
        prev.allBranches,
        prev.query,
        opId,
      );
      const cur = get().repoLog[root];
      if (!cur || cur.gen !== gen) return; // stale(검색/브랜치 전환됨).
      // 취소면 첫 페이지는 비운 채로 두고(“커밋 없음”과 구분), 이어 로드는
      // 이미 받은 목록을 그대로 유지한다.
      const commits = reset
        ? res.canceled
          ? undefined
          : res.commits
        : [...(cur.commits ?? []), ...res.commits];
      setRepoLog(set, root, {
        ...cur,
        commits,
        hasMore: res.canceled ? (reset ? false : cur.hasMore) : res.hasMore,
        loaded: commits?.length ?? 0,
        timedOut: res.timedOut,
        canceled: res.canceled,
        loading: false,
        opId: undefined,
      });
    } catch (err) {
      console.warn("workdir: repo log failed", err);
      const cur = get().repoLog[root];
      if (cur && cur.gen === gen) {
        setRepoLog(set, root, { ...cur, loading: false, opId: undefined });
      }
    }
  },

  setRepoLogQuery: (query) => {
    const p = get().palette;
    if (!p) return;
    const prev = get().repoLog[p.root] ?? emptyRepoLog(p.root);
    setRepoLog(set, p.root, { ...prev, query });
    void get().loadRepoLog(true);
  },

  setRepoLogAllBranches: (all) => {
    const p = get().palette;
    if (!p) return;
    const prev = get().repoLog[p.root] ?? emptyRepoLog(p.root);
    setRepoLog(set, p.root, { ...prev, allBranches: all });
    void get().loadRepoLog(true);
  },

  selectRepoCommit: async (hash) => {
    const p = get().palette;
    if (!p) return;
    const root = p.root;
    const prev = get().repoLog[root];
    if (!prev) return;
    // 다른 커밋을 고르면 이전 커밋의 변경파일·diff 조회는 무의미해진다.
    cancel(prev.filesOpId);
    cancel(prev.fileDiffOpId);
    const opId = newOpId();
    setRepoLog(set, root, {
      ...prev,
      selectedCommit: hash,
      files: undefined,
      filesLoading: true,
      filesOpId: opId,
      filesCanceled: false,
      filesHasMore: false,
      filesLoaded: 0,
      selectedFile: undefined,
      fileDiff: undefined,
      fileDiffOpId: undefined,
    });
    try {
      const res = await tauriApi.workdirCommitFiles(root, hash, COMMIT_FILES_PAGE, 0, opId);
      const cur = get().repoLog[root];
      if (!cur || cur.selectedCommit !== hash) return;
      setRepoLog(set, root, {
        ...cur,
        // 취소면 빈 목록을 캐시하지 않는다("변경 없음"과 구분).
        files: res.canceled ? undefined : res.files,
        filesCanceled: res.canceled,
        filesHasMore: res.canceled ? false : res.hasMore,
        filesLoaded: res.canceled ? 0 : res.files.length,
        filesLoading: false,
        filesOpId: undefined,
      });
    } catch (err) {
      console.warn("workdir: repo log commit files failed", err);
      const cur = get().repoLog[root];
      if (cur && cur.selectedCommit === hash) {
        setRepoLog(set, root, { ...cur, filesLoading: false, filesOpId: undefined });
      }
    }
  },

  loadMoreRepoFiles: async () => {
    const p = get().palette;
    if (!p) return;
    const root = p.root;
    const prev = get().repoLog[root];
    if (!prev || !prev.selectedCommit || prev.filesLoading || !prev.filesHasMore) return;
    const hash = prev.selectedCommit;
    const opId = newOpId();
    setRepoLog(set, root, { ...prev, filesLoading: true, filesOpId: opId, filesCanceled: false });
    try {
      const res = await tauriApi.workdirCommitFiles(
        root,
        hash,
        COMMIT_FILES_PAGE,
        prev.filesLoaded,
        opId,
      );
      const cur = get().repoLog[root];
      if (!cur || cur.selectedCommit !== hash) return;
      const files = [...(cur.files ?? []), ...res.files];
      setRepoLog(set, root, {
        ...cur,
        files,
        // 취소면 이미 받은 페이지는 유지하고 "더 보기"를 남긴다.
        filesCanceled: res.canceled,
        filesHasMore: res.canceled ? cur.filesHasMore : res.hasMore,
        filesLoaded: files.length,
        filesLoading: false,
        filesOpId: undefined,
      });
    } catch (err) {
      console.warn("workdir: repo log commit files next page failed", err);
      const cur = get().repoLog[root];
      if (cur && cur.selectedCommit === hash) {
        setRepoLog(set, root, { ...cur, filesLoading: false, filesOpId: undefined });
      }
    }
  },

  selectRepoFile: async (hash, path) => {
    const p = get().palette;
    if (!p) return;
    const root = p.root;
    const prev = get().repoLog[root];
    if (!prev) return;
    // 다른 파일을 고르면(또는 같은 파일을 재시도하면) 이전 diff 조회는 버린다.
    cancel(prev.fileDiffOpId);
    const opId = newOpId();
    setRepoLog(set, root, {
      ...prev,
      selectedFile: path,
      fileDiff: undefined,
      fileDiffLoading: true,
      fileDiffOpId: opId,
    });
    try {
      const res = await tauriApi.workdirDiffCommit(root, hash, path, opId);
      const cur = get().repoLog[root];
      if (!cur || cur.selectedFile !== path || cur.selectedCommit !== hash) return;
      setRepoLog(set, root, { ...cur, fileDiff: res, fileDiffLoading: false, fileDiffOpId: undefined });
    } catch (err) {
      console.warn("workdir: repo log file diff failed", err);
      const cur = get().repoLog[root];
      if (cur && cur.selectedFile === path) {
        setRepoLog(set, root, { ...cur, fileDiffLoading: false, fileDiffOpId: undefined });
      }
    }
  },

  openRepoDifftool: () => {
    const p = get().palette;
    if (!p) return;
    const rl = get().repoLog[p.root];
    if (!rl || !rl.selectedCommit || !rl.selectedFile) return;
    void tauriApi
      .workdirDifftool(p.root, rl.selectedFile, "worktreeVsHead", rl.selectedCommit)
      .catch((err) => console.warn("workdir: difftool launch failed", err));
  },
});
