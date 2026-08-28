// src/renderer/workdir/slices/detailSlice.ts
//
// 우측 상세(메뉴) 페인 — 파일 하나를 지목한 뒤의 변경점/히스토리.
//
// 인터랙션 모델(이슈 #54): **모든** 파일 클릭은 곧장 열지 않고 이 페인을
// 띄운다 — 변경 파일은 기본 "변경점" 탭, 변경 없는 파일은 기본 "히스토리"
// 탭으로 열려 깃 로그를 항상 볼 수 있다. 히스토리의 커밋은 펼치면
// (`toggleCommitExpand`) 그 커밋이 바꾼 파일 목록을 인라인으로 보여주고,
// 펼치지 않고 커밋만 고르면(`selectCommit`) 지금 파일의 그 커밋 시점 diff를 본다.
import { tauriApi } from "../../ipc/tauriApi";
import { useMarkdownStore } from "../../markdown/markdownStore";

import { COMMIT_FILES_PAGE, HISTORY_PAGE, cancel, cancelDetailOps, newOpId } from "../workdirOps";
import { isChangedStatus, isMarkdownPath, joinPath } from "../workdirTypes";
import type { WorkdirDetailSlice, WorkdirSlice } from "../workdirTypes";
import type { GitDiffMode } from "@shared/types";

export const createDetailSlice: WorkdirSlice<WorkdirDetailSlice> = (set, get) => ({
  detail: null,

  openDetail: (root, relPath, name, status) => {
    const isUntracked = status === "?";
    // 미추적은 untracked 모드 고정, 그 외는 전체 변경 합본(worktreeVsHead)이 기본.
    const diffMode: GitDiffMode = isUntracked ? "untracked" : "worktreeVsHead";
    // 변경 파일은 변경점을 먼저 보여주고, 변경 없는(clean) 파일은 볼 변경점이
    // 없으므로 히스토리 탭으로 열어 깃 로그를 바로 노출한다(이슈 #54).
    const tab: "diff" | "history" = isChangedStatus(status) ? "diff" : "history";
    // 이전 파일의 상세가 걸어 둔 조회는 여기서 무의미해진다.
    cancelDetailOps(get().detail);
    set((s) => ({
      detail: {
        root,
        relPath,
        name,
        status,
        isUntracked,
        tab,
        diffMode,
        diff: undefined,
        diffLoading: false,
        diffOpId: undefined,
        history: undefined,
        historyLoading: false,
        historyOpId: undefined,
        historyCanceled: false,
        historyHasMore: false,
        selectedCommit: undefined,
        selectedCommitFile: undefined,
        commitDiff: undefined,
        commitDiffLoading: false,
        commitDiffOpId: undefined,
        expandedCommit: undefined,
        commitFiles: undefined,
        commitFilesLoading: false,
        commitFilesOpId: undefined,
        commitFilesCanceled: false,
        commitFilesHasMore: false,
        commitFilesSkip: 0,
        gen: (s.detail?.gen ?? 0) + 1,
      },
    }));
    // 변경점 탭이면 diff를, 히스토리 탭이면 로그를 즉시 로드(보이는 탭 우선).
    if (tab === "diff") void get().loadDiff();
    else void get().loadHistory();
  },

  closeDetail: () => {
    // 상세를 닫으면 그 조회 결과를 볼 곳이 없다 — 진행 중이던 git을 끊는다.
    cancelDetailOps(get().detail);
    set({ detail: null });
  },

  openExternal: () => {
    const d = get().detail;
    if (!d) return;
    // 마크다운 포함 항상 외부 에디터로. 팔레트는 유지(참조용).
    void tauriApi
      .openInVscode(joinPath(d.root, d.relPath))
      .catch((err) => console.warn(`workdir: open external failed: ${d.name}`, err));
  },

  openInApp: () => {
    const d = get().detail;
    if (!d) return;
    if (!isMarkdownPath(d.relPath)) return; // 인앱 지원 형식만.
    const agentId = get().palette?.agentId ?? "";
    // 현재 탐색 상태(팔레트+메뉴)를 스냅샷해 두고, 인앱 뷰어를 닫으면 그대로
    // 복귀시킨다("인앱 뷰어 닫으면 다시 탐색 모드").
    const palette = get().palette;
    const detail = d;
    set({ palette: null, detail: null });
    void useMarkdownStore.getState().openFile(d.root, d.relPath, agentId, () => {
      set({ palette, detail });
      if (palette) {
        // 뷰어를 보던 사이 파일/ git이 바뀌었을 수 있으니 백그라운드 갱신.
        // 방금 편집기에서 돌아온 시점이라 TTL과 무관하게 강제 재스캔한다.
        void get().refreshListing(palette.root, { force: true });
        void get().refreshGit(palette.root);
      }
    });
  },

  setDetailTab: (tab) => {
    set((s) => (s.detail ? { detail: { ...s.detail, tab } } : s));
    // 아직 로드 안 된 탭을 처음 열면 지연 로드(변경 없는 파일은 diff 탭이,
    // 변경 파일은 history 탭이 최초 진입 시 비어 있다).
    const d = get().detail;
    if (!d) return;
    if (tab === "history" && d.history === undefined && !d.historyLoading) {
      void get().loadHistory();
    } else if (tab === "diff" && d.diff === undefined && !d.diffLoading) {
      void get().loadDiff();
    }
  },

  setDiffMode: (mode) => {
    // gen을 올려 진행 중이던 이전 모드의 응답을 폐기하고 새로 로드. 그 응답은
    // 어차피 버릴 것이므로 자식 git도 함께 끊는다.
    cancel(get().detail?.diffOpId);
    set((s) =>
      s.detail
        ? {
            detail: {
              ...s.detail,
              diffMode: mode,
              diff: undefined,
              diffOpId: undefined,
              gen: s.detail.gen + 1,
            },
          }
        : s,
    );
    void get().loadDiff();
  },

  loadDiff: async () => {
    const d = get().detail;
    if (!d) return;
    const { root, relPath, diffMode, gen } = d;
    // 재시도(취소/시간 초과 뒤)면 이전 결과를 비워 진행 표시가 보이게 한다.
    const opId = newOpId();
    set((s) =>
      s.detail
        ? {
            detail: {
              ...s.detail,
              diffLoading: true,
              diffOpId: opId,
              diff:
                s.detail.diff && (s.detail.diff.canceled || s.detail.diff.timedOut)
                  ? undefined
                  : s.detail.diff,
            },
          }
        : s,
    );
    try {
      const res = await tauriApi.workdirDiffFile(root, relPath, diffMode, opId);
      set((s) =>
        s.detail && s.detail.gen === gen && s.detail.relPath === relPath
          ? { detail: { ...s.detail, diff: res, diffLoading: false, diffOpId: undefined } }
          : s,
      );
    } catch (err) {
      console.warn("workdir: diff failed", err);
      set((s) =>
        s.detail && s.detail.gen === gen
          ? { detail: { ...s.detail, diffLoading: false, diffOpId: undefined } }
          : s,
      );
    }
  },

  loadHistory: async () => {
    const d = get().detail;
    if (!d) return;
    const { root, relPath } = d;
    const opId = newOpId();
    set((s) =>
      s.detail
        ? { detail: { ...s.detail, historyLoading: true, historyOpId: opId, historyCanceled: false } }
        : s,
    );
    try {
      const res = await tauriApi.workdirFileHistory(root, relPath, HISTORY_PAGE, 0, opId);
      set((s) =>
        s.detail && s.detail.relPath === relPath
          ? {
              detail: {
                ...s.detail,
                // 취소면 빈 목록을 캐시하지 않는다 — "커밋 없음"과 구분해
                // "다시 시도"를 띄우기 위함.
                history: res.canceled ? undefined : res.commits,
                historyCanceled: res.canceled,
                historyHasMore: res.canceled ? false : res.hasMore,
                historyLoading: false,
                historyOpId: undefined,
              },
            }
          : s,
      );
    } catch (err) {
      console.warn("workdir: file history failed", err);
      set((s) =>
        s.detail && s.detail.relPath === relPath
          ? { detail: { ...s.detail, historyLoading: false, historyOpId: undefined } }
          : s,
      );
    }
  },

  selectCommit: async (hash) => {
    const d = get().detail;
    if (!d) return;
    // 펼치지 않고 커밋만 고르면 "이 파일"의 그 커밋 시점 diff를 본다.
    await get().selectCommitFile(hash, d.relPath);
  },

  selectCommitFile: async (hash, path) => {
    const d = get().detail;
    if (!d) return;
    const { root } = d;
    // 이전 선택의 diff는 이제 볼 일이 없다.
    cancel(d.commitDiffOpId);
    const opId = newOpId();
    set((s) =>
      s.detail
        ? {
            detail: {
              ...s.detail,
              selectedCommit: hash,
              selectedCommitFile: path,
              commitDiff: undefined,
              commitDiffLoading: true,
              commitDiffOpId: opId,
            },
          }
        : s,
    );
    try {
      const res = await tauriApi.workdirDiffCommit(root, hash, path, opId);
      set((s) =>
        s.detail && s.detail.selectedCommit === hash && s.detail.selectedCommitFile === path
          ? {
              detail: {
                ...s.detail,
                commitDiff: res,
                commitDiffLoading: false,
                commitDiffOpId: undefined,
              },
            }
          : s,
      );
    } catch (err) {
      console.warn("workdir: commit diff failed", err);
      set((s) =>
        s.detail && s.detail.selectedCommit === hash
          ? { detail: { ...s.detail, commitDiffLoading: false, commitDiffOpId: undefined } }
          : s,
      );
    }
  },

  toggleCommitExpand: async (hash) => {
    const d = get().detail;
    if (!d) return;
    // 이미 펼친 커밋을 다시 누르면 접는다(진행 중이던 목록 조회도 함께 끊는다).
    if (d.expandedCommit === hash) {
      cancel(d.commitFilesOpId);
      set((s) =>
        s.detail
          ? {
              detail: {
                ...s.detail,
                expandedCommit: undefined,
                commitFilesLoading: false,
                commitFilesOpId: undefined,
              },
            }
          : s,
      );
      return;
    }
    const { root } = d;
    // 다른 커밋을 펼치면 이전 커밋의 목록 조회는 무의미해진다.
    cancel(d.commitFilesOpId);
    const opId = newOpId();
    set((s) =>
      s.detail
        ? {
            detail: {
              ...s.detail,
              expandedCommit: hash,
              commitFiles: undefined,
              commitFilesLoading: true,
              commitFilesOpId: opId,
              commitFilesCanceled: false,
              commitFilesHasMore: false,
              commitFilesSkip: 0,
            },
          }
        : s,
    );
    try {
      const res = await tauriApi.workdirCommitFiles(root, hash, COMMIT_FILES_PAGE, 0, opId);
      set((s) =>
        s.detail && s.detail.expandedCommit === hash
          ? {
              detail: {
                ...s.detail,
                // 취소면 빈 목록을 캐시하지 않는다("변경 없음"과 구분).
                commitFiles: res.canceled ? undefined : res.files,
                commitFilesCanceled: res.canceled,
                commitFilesHasMore: res.canceled ? false : res.hasMore,
                commitFilesLoading: false,
                commitFilesOpId: undefined,
                commitFilesSkip: res.canceled ? 0 : res.files.length,
              },
            }
          : s,
      );
    } catch (err) {
      console.warn("workdir: commit files failed", err);
      set((s) =>
        s.detail && s.detail.expandedCommit === hash
          ? { detail: { ...s.detail, commitFilesLoading: false, commitFilesOpId: undefined } }
          : s,
      );
    }
  },

  loadMoreCommitFiles: async () => {
    const d = get().detail;
    if (!d || !d.expandedCommit || d.commitFilesLoading || !d.commitFilesHasMore) return;
    const { root, expandedCommit, commitFilesSkip } = d;
    const opId = newOpId();
    set((s) =>
      s.detail
        ? {
            detail: {
              ...s.detail,
              commitFilesLoading: true,
              commitFilesOpId: opId,
              commitFilesCanceled: false,
            },
          }
        : s,
    );
    try {
      const res = await tauriApi.workdirCommitFiles(
        root,
        expandedCommit,
        COMMIT_FILES_PAGE,
        commitFilesSkip,
        opId,
      );
      set((s) =>
        s.detail && s.detail.expandedCommit === expandedCommit
          ? {
              detail: {
                ...s.detail,
                // 취소면 이미 받은 페이지는 그대로 두고 "더 보기"만 남긴다.
                commitFiles: [...(s.detail.commitFiles ?? []), ...res.files],
                commitFilesCanceled: res.canceled,
                commitFilesHasMore: res.canceled ? s.detail.commitFilesHasMore : res.hasMore,
                commitFilesLoading: false,
                commitFilesOpId: undefined,
                commitFilesSkip: commitFilesSkip + res.files.length,
              },
            }
          : s,
      );
    } catch (err) {
      console.warn("workdir: commit files next page failed", err);
      set((s) =>
        s.detail && s.detail.expandedCommit === expandedCommit
          ? { detail: { ...s.detail, commitFilesLoading: false, commitFilesOpId: undefined } }
          : s,
      );
    }
  },

  openDifftool: (commit) => {
    const d = get().detail;
    if (!d) return;
    // 하단 diff가 보고 있는 파일(없으면 이 상세 파일)을 대상으로.
    const rel = d.selectedCommitFile ?? d.relPath;
    void tauriApi
      .workdirDifftool(d.root, rel, d.diffMode, commit)
      .catch((err) => console.warn("workdir: difftool launch failed", err));
  },
});
